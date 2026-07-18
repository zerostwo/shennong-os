use super::audit;
use super::projects::{require_project_read, require_project_write};
use crate::{
    AppState,
    auth::{authenticate, require_admin},
    clients::{JsonResponse, UpstreamError},
    crypto::sha256_hex,
    error::ApiError,
};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{OriginalUri, Path, Query, State},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use std::collections::HashSet;
use uuid::Uuid;

const PROJECT_DATA_ROUTES: &[&str] = &[
    "context-pack",
    "entities",
    "activities",
    "studies",
    "associations",
    "evidence",
    "resources",
];

const DEFAULT_GRAPH_DEPTH: u8 = 1;
const DEFAULT_GRAPH_LIMIT: u16 = 80;
const PUBLIC_RESOURCE_CATALOG_LIMIT: usize = 100;
const PUBLIC_RESOURCE_MAX_OFFSET: i64 = 1_000_000;
const DEFAULT_AGENT_RESOURCE_LIMIT: i64 = 20;
const OS_ACTOR_HEADER: &str = "x-shennong-os-actor-id";
const OS_PROJECT_HEADER: &str = "x-shennong-os-project-id";
const MAX_AGENT_RESOURCE_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
pub struct ProjectSubgraphQuery {
    root: String,
    depth: Option<u8>,
    limit: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct PublicResourceListQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceProviderInstall {
    name: String,
}

pub async fn resource_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers, false).await?;
    proxy_json(&state, Method::GET, &["providers"], vec![], None).await
}

pub async fn install_resource_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<ResourceProviderInstall>,
) -> Result<Response, ApiError> {
    let actor = require_admin(&state, &headers, true).await?;
    let name = value.name.trim();
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
    {
        return Err(ApiError::invalid("invalid Resource provider name"));
    }
    let response = db_request(
        &state,
        Method::POST,
        &["resources", "install"],
        &[],
        Some(&json!({"name": name})),
    )
    .await?;
    audit(
        &state,
        Some(&actor),
        None,
        "resource_provider.install",
        "resource_provider",
        Some(name.to_owned()),
        json!({}),
    )
    .await?;
    json_response(response)
}

#[derive(Debug, FromRow)]
struct ProjectShadow {
    id: Uuid,
    owner_user_id: Uuid,
    name: String,
    description: String,
    visibility: String,
    status: String,
}

impl ProjectShadow {
    fn payload(&self) -> Value {
        json!({
            "id": self.id,
            "owner_user_id": self.owner_user_id,
            "name": self.name,
            "description": self.description,
            "visibility": self.visibility,
            "status": self.status,
            "metadata": {"authority": "shennong-os"}
        })
    }
}

pub async fn resources_root(
    State(state): State<AppState>,
    Query(query): Query<PublicResourceListQuery>,
) -> Result<Response, ApiError> {
    let query = public_resource_query(&query)?;
    let response = db_request(&state, Method::GET, &["resources"], &query, None).await?;
    let mut response = json_response(filter_public_resource_list(response)?)?;
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=30, stale-while-revalidate=60"),
    );
    Ok(response)
}

fn public_resource_query(
    query: &PublicResourceListQuery,
) -> Result<Vec<(&'static str, String)>, ApiError> {
    let mut output = Vec::with_capacity(3);
    if let Some(search) = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if search.len() > 256 {
            return Err(ApiError::invalid("q must be at most 256 characters"));
        }
        output.push(("q", search.to_owned()));
    }
    let limit = query.limit.unwrap_or(PUBLIC_RESOURCE_CATALOG_LIMIT as i64);
    if !(1..=PUBLIC_RESOURCE_CATALOG_LIMIT as i64).contains(&limit) {
        return Err(ApiError::invalid("limit must be between 1 and 100"));
    }
    output.push(("limit", limit.to_string()));
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| ApiError::invalid("cursor must be a decimal offset"))
        })
        .transpose()?;
    if query.offset.is_some() && cursor.is_some() && query.offset != cursor {
        return Err(ApiError::invalid(
            "cursor and offset must match when both are provided",
        ));
    }
    let offset = query.offset.or(cursor).unwrap_or(0);
    if !(0..=PUBLIC_RESOURCE_MAX_OFFSET).contains(&offset) {
        return Err(ApiError::invalid("offset must be between 0 and 1000000"));
    }
    output.push(("offset", offset.to_string()));
    Ok(output)
}

pub async fn resource(
    State(state): State<AppState>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    validate_segment(&id)?;
    let response = db_request(
        &state,
        Method::GET,
        &["resources", &id],
        &query_pairs(uri.query()),
        None,
    )
    .await?;
    json_response(sanitize_public_resource_response(response, &id)?)
}

pub async fn resource_child(
    State(state): State<AppState>,
    Path((id, child)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    validate_segment(&id)?;
    if !matches!(child.as_str(), "artifacts" | "relations" | "graph-context") {
        return Err(ApiError::not_found());
    }
    let resource = db_request(&state, Method::GET, &["resources", &id], &[], None).await?;
    require_public_resource_response(&resource, &id)?;
    let response = db_request(
        &state,
        Method::GET,
        &["resources", &id, &child],
        &query_pairs(uri.query()),
        None,
    )
    .await?;
    json_response(sanitize_public_resource_child(&state, &id, &child, response).await?)
}

pub async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let (project_id, resource) = governed_query_scope(&body)?;
    require_project_read(&state, &actor, project_id).await?;
    ensure_project_resource(&state, project_id, resource).await?;
    proxy_json(&state, Method::POST, &["query"], Vec::new(), Some(&body)).await
}

pub async fn project_subgraph(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Query(request): Query<ProjectSubgraphQuery>,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, project_id).await?;
    let query = project_subgraph_query(project_id, &request)?;
    sync_project_shadow(&state, project_id).await?;
    proxy_json(&state, Method::GET, &["graph", "subgraph"], query, None).await
}

pub async fn list_project_uploads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, project_id).await?;
    require_active_project(&state, project_id).await?;
    sync_project_shadow(&state, project_id).await?;
    let response = platform_db_json(
        &state,
        Method::GET,
        &["uploads"],
        None,
        actor.id,
        project_id,
    )
    .await?;
    json_response(response)
}

pub async fn upload_project_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    body: Body,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, project_id).await?;
    require_active_project(&state, project_id).await?;
    let (filename, content_type, content_length) =
        validate_upload_headers(&headers, state.config.max_upload_bytes)?;
    sync_project_shadow(&state, project_id).await?;
    let client = state.config.db_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_unavailable",
            "Shennong DB is not configured",
        )
    })?;
    let key = state.config.db_admin_key.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_auth_unavailable",
            "Shennong DB credential is not configured",
        )
    })?;
    let actor_id = actor.id.to_string();
    let project = project_id.to_string();
    let content_length_header = content_length.to_string();
    let stream = reqwest::Body::wrap_stream(body.into_data_stream());
    let response = client
        .request_streaming_json(
            Method::POST,
            &["api", "v1", "uploads"],
            stream,
            Some(("x-shennong-admin-key", key)),
            &[
                (OS_ACTOR_HEADER, actor_id.as_str()),
                (OS_PROJECT_HEADER, project.as_str()),
                ("x-filename", filename.as_str()),
                ("content-type", content_type.as_str()),
                ("content-length", content_length_header.as_str()),
            ],
            state.config.upload_timeout,
        )
        .await
        .map_err(map_upstream)?;
    if response.status.is_success() {
        let upload_id = response
            .body
            .pointer("/data/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        audit(
            &state,
            Some(&actor),
            Some(project_id),
            "upload.create",
            "upload",
            upload_id,
            json!({"filename":filename,"size_bytes":content_length}),
        )
        .await?;
    }
    json_response(response)
}

pub async fn register_project_uploads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, project_id).await?;
    require_active_project(&state, project_id).await?;
    let object = body
        .as_object_mut()
        .ok_or_else(|| ApiError::invalid("upload registration body must be an object"))?;
    for untrusted in [
        "actor_id",
        "owner",
        "owner_user_id",
        "project",
        "project_id",
        "user_id",
    ] {
        object.remove(untrusted);
    }
    object.insert("visibility".into(), Value::String("private".into()));
    sync_project_shadow(&state, project_id).await?;
    let response = platform_db_json(
        &state,
        Method::POST,
        &["uploads", "register"],
        Some(&body),
        actor.id,
        project_id,
    )
    .await?;
    if response.status.is_success() {
        let resource_id = response
            .body
            .pointer("/data/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        audit(
            &state,
            Some(&actor),
            Some(project_id),
            "upload.register",
            "resource",
            resource_id,
            json!({"visibility":"private"}),
        )
        .await?;
    }
    json_response(response)
}

async fn require_active_project(state: &AppState, project_id: Uuid) -> Result<(), ApiError> {
    let active = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1 AND status='active')",
    )
    .bind(project_id)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if !active {
        return Err(ApiError::not_found());
    }
    Ok(())
}

fn validate_upload_headers(
    headers: &HeaderMap,
    maximum: usize,
) -> Result<(String, String, usize), ApiError> {
    let filename = headers
        .get("x-filename")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 255
                && *value != "."
                && *value != ".."
                && !value.starts_with(char::is_whitespace)
                && !value.ends_with(char::is_whitespace)
                && !value
                    .chars()
                    .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        })
        .ok_or_else(|| {
            ApiError::invalid("x-filename must be a safe file name of at most 255 bytes")
        })?
        .to_owned();
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream");
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    let valid_token = |token: &str| {
        !token.is_empty()
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&byte))
    };
    if content_type.len() > 255
        || content_type.chars().any(char::is_control)
        || !media_type
            .split_once('/')
            .is_some_and(|(kind, subtype)| valid_token(kind) && valid_token(subtype))
    {
        return Err(ApiError::invalid(
            "content-type must be a valid media type of at most 255 bytes",
        ));
    }
    let content_length = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::LENGTH_REQUIRED,
                "upload_length_required",
                "A valid Content-Length header is required for uploads",
            )
        })?;
    if content_length == 0 || content_length > maximum {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            "Upload is empty or exceeds the configured size limit",
        ));
    }
    Ok((filename, content_type.to_owned(), content_length))
}

async fn platform_db_json(
    state: &AppState,
    method: Method,
    segments: &[&str],
    body: Option<&Value>,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<JsonResponse, ApiError> {
    let client = state.config.db_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_unavailable",
            "Shennong DB is not configured",
        )
    })?;
    let key = state.config.db_admin_key.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_auth_unavailable",
            "Shennong DB credential is not configured",
        )
    })?;
    let actor = actor_id.to_string();
    let project = project_id.to_string();
    let api_segments = db_api_segments(segments);
    client
        .request_json_with_headers(
            method,
            &api_segments,
            &[],
            body,
            Some(("x-shennong-admin-key", key)),
            None,
            &[
                (OS_ACTOR_HEADER, actor.as_str()),
                (OS_PROJECT_HEADER, project.as_str()),
            ],
        )
        .await
        .map_err(map_upstream)
}

pub async fn project_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    Path((project_id, tail)): Path<(Uuid, String)>,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, ApiError> {
    let mutation = !matches!(method, Method::GET | Method::HEAD);
    let actor = authenticate(&state, &headers, mutation).await?;
    if mutation {
        require_project_write(&state, &actor, project_id).await?;
    } else {
        require_project_read(&state, &actor, project_id).await?;
    }
    let tail_segments = tail.split('/').collect::<Vec<_>>();
    if tail_segments.is_empty()
        || tail_segments.len() > 4
        || !PROJECT_DATA_ROUTES.contains(&tail_segments[0])
        || tail_segments
            .iter()
            .any(|segment| validate_segment(segment).is_err())
    {
        return Err(ApiError::not_found());
    }
    if !allowed_project_method(&tail_segments, &method) {
        return Err(ApiError::not_found());
    }
    let value = if matches!(method, Method::GET | Method::HEAD | Method::DELETE) || body.is_empty()
    {
        None
    } else {
        Some(
            serde_json::from_slice::<Value>(&body)
                .map_err(|_| ApiError::invalid("data-plane body must be JSON"))?,
        )
    };
    sync_project_shadow(&state, project_id).await?;
    let project = project_id.to_string();
    let segments = research_project_segments(&project, &tail_segments);
    let response = db_request(
        &state,
        method.clone(),
        &segments,
        &query_pairs(uri.query()),
        value.as_ref(),
    )
    .await?;
    if response.status.is_success() && mutation {
        audit(
            &state,
            Some(&actor),
            Some(project_id),
            "data_plane.mutation",
            tail_segments[0],
            tail_segments.get(1).map(|value| (*value).to_owned()),
            json!({"method":method.as_str()}),
        )
        .await?;
    }
    json_response(response)
}

pub(crate) async fn execute_db_tool(
    state: &AppState,
    project_id: Uuid,
    name: &str,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let response = match name {
        "db.discover_resources" => discover_project_resources(state, project_id, arguments).await?,
        "db.inspect_resource" | "db.get_provenance" => {
            let resource = arguments
                .get("resource")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::invalid("resource is required"))?;
            validate_segment(resource)?;
            ensure_project_resource(state, project_id, resource).await?;
            db_request(state, Method::GET, &["resources", resource], &[], None).await?
        }
        "db.query_resource" => {
            let resource = arguments
                .get("resource")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::invalid("resource is required"))?;
            validate_segment(resource)?;
            ensure_project_resource(state, project_id, resource).await?;
            let body = agent_resource_query_body(project_id, arguments)?;
            db_request(state, Method::POST, &["query"], &[], Some(&body)).await?
        }
        _ => return Err(ApiError::not_found()),
    };
    if !response.status.is_success() {
        return Err(ApiError::new(
            if response.status == StatusCode::NOT_FOUND {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            },
            "data_plane_rejected",
            "Shennong DB rejected the governed request",
        ));
    }
    Ok(response.body)
}

fn agent_resource_query_body(project_id: Uuid, arguments: &Value) -> Result<Value, ApiError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| ApiError::invalid("query arguments must be an object"))?;
    let resource = object
        .get("resource")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("resource is required"))?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("operation is required"))?;
    let limit = object
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(1, 1000);
    let context = object.get("context").cloned().unwrap_or_else(|| json!({}));
    if !context.is_object() {
        return Err(ApiError::invalid("context must be an object"));
    }
    let mut body = json!({
        "project_id": project_id,
        "resource": resource,
        "operation": operation,
        "context": context,
        "options": {"limit": limit}
    });
    if let Some(feature) = object.get("feature").and_then(Value::as_str) {
        body["feature"] = json!({"type": "gene", "name": feature});
    }
    Ok(body)
}

async fn discover_project_resources(
    state: &AppState,
    project_id: Uuid,
    arguments: &Value,
) -> Result<JsonResponse, ApiError> {
    let arguments = arguments
        .as_object()
        .ok_or_else(|| ApiError::invalid("discovery arguments must be an object"))?;
    let query = match arguments.get("q") {
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.len() > 256 {
                return Err(ApiError::invalid("q must be at most 256 characters"));
            }
            (!value.is_empty()).then(|| value.to_owned())
        }
        Some(_) => return Err(ApiError::invalid("q must be a string")),
        None => None,
    };
    let limit = bounded_agent_resource_limit(arguments.get("limit"))?;

    // The authoritative Project collection is queried first so private
    // metadata can only enter the result through an exact Project binding.
    sync_project_shadow(state, project_id).await?;
    let project = project_id.to_string();
    let project_segments = research_project_segments(&project, &["resources"]);
    let project_response = db_request(state, Method::GET, &project_segments, &[], None).await?;
    if !project_response.status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "data_plane_rejected",
            "Shennong DB rejected the governed Resource discovery request",
        ));
    }

    let mut public_query = Vec::new();
    if let Some(value) = query.as_ref() {
        public_query.push(("q", value.clone()));
    }
    public_query.push(("limit", MAX_AGENT_RESOURCE_LIMIT.to_string()));
    let public_response =
        db_request(state, Method::GET, &["resources"], &public_query, None).await?;
    if !public_response.status.is_success() {
        return Err(public_data_plane_error(public_response.status));
    }

    let body = merge_discoverable_resources(
        &project_response.body,
        &public_response.body,
        project_id,
        query.as_deref(),
        limit,
    )?;
    Ok(JsonResponse {
        status: StatusCode::OK,
        body,
    })
}

fn bounded_agent_resource_limit(value: Option<&Value>) -> Result<usize, ApiError> {
    let limit = match value {
        Some(value) => value
            .as_i64()
            .ok_or_else(|| ApiError::invalid("limit must be an integer"))?,
        None => DEFAULT_AGENT_RESOURCE_LIMIT,
    };
    Ok(limit.clamp(1, MAX_AGENT_RESOURCE_LIMIT) as usize)
}

async fn ensure_project_resource(
    state: &AppState,
    project_id: Uuid,
    resource: &str,
) -> Result<(), ApiError> {
    sync_project_shadow(state, project_id).await?;
    let project = project_id.to_string();
    let project_tail = ["resources"];
    let segments = research_project_segments(&project, &project_tail);
    let response = db_request(state, Method::GET, &segments, &[], None).await?;
    if !response.status.is_success() {
        return Err(ApiError::not_found());
    }
    if !project_resource_is_bound(&response.body, project_id, resource) {
        return Err(ApiError::not_found());
    }
    Ok(())
}

fn bound_artifact_contract(
    body: &Value,
    resource_id: &str,
    artifact_id: &str,
    maximum: usize,
) -> Result<(usize, String), ApiError> {
    let artifact = body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|artifacts| {
            artifacts
                .iter()
                .find(|artifact| artifact.get("id").and_then(Value::as_str) == Some(artifact_id))
        })
        .filter(|artifact| {
            artifact.get("resource_id").and_then(Value::as_str) == Some(resource_id)
                && artifact.get("immutable").and_then(Value::as_bool) == Some(true)
                && matches!(
                    artifact.get("storage_backend").and_then(Value::as_str),
                    Some("local" | "s3")
                )
        })
        .ok_or_else(ApiError::not_found)?;
    let size = artifact
        .get("size")
        .and_then(Value::as_u64)
        .and_then(|size| usize::try_from(size).ok())
        .filter(|size| *size > 0 && *size <= maximum)
        .ok_or_else(|| {
            ApiError::invalid("project artifact must be a non-empty file within the staging limit")
        })?;
    let digest = artifact
        .get("content_sha256")
        .and_then(Value::as_str)
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ApiError::invalid("project artifact has no verified SHA-256 digest"))?;
    Ok((size, digest))
}

/// Resolve one immutable UTF-8 Artifact that is already bound to the exact
/// Project. The OS verifies the binding before using its DB service credential,
/// then verifies size and digest again after the bounded download. This is the
/// only V1 bridge from governed uploaded Resources into Runtime workspace files.
pub(crate) async fn read_project_artifact_text(
    state: &AppState,
    project_id: Uuid,
    resource_id: &str,
    artifact_id: &str,
    maximum: usize,
) -> Result<Value, ApiError> {
    validate_segment(resource_id)?;
    validate_segment(artifact_id)?;
    if !(1..=1_048_576).contains(&maximum) {
        return Err(ApiError::invalid(
            "project artifact staging limit must be 1..1048576 bytes",
        ));
    }
    ensure_project_resource(state, project_id, resource_id).await?;
    let listing = db_request(
        state,
        Method::GET,
        &["resources", resource_id, "artifacts"],
        &[],
        None,
    )
    .await?;
    if !listing.status.is_success() {
        return Err(if listing.status == StatusCode::NOT_FOUND {
            ApiError::not_found()
        } else {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "data_plane_rejected",
                "Shennong DB rejected the governed Artifact request",
            )
        });
    }
    let (expected_size, expected_digest) =
        bound_artifact_contract(&listing.body, resource_id, artifact_id, maximum)?;

    let client = state.config.db_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_unavailable",
            "Shennong DB is not configured",
        )
    })?;
    let key = state.config.db_admin_key.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_auth_unavailable",
            "Shennong DB credential is not configured",
        )
    })?;
    let url = client
        .streaming_url(
            &[
                "api",
                "v1",
                "resources",
                resource_id,
                "artifacts",
                artifact_id,
                "download",
            ],
            None,
        )
        .map_err(map_upstream)?;
    let response = client
        .streaming_client()
        .get(url)
        .header("x-shennong-admin-key", key)
        .header("accept", "application/octet-stream")
        .send()
        .await
        .map_err(|error| map_upstream(UpstreamError::Request(error)))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found());
    }
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "data_plane_rejected",
            "Shennong DB rejected the governed Artifact download",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length != expected_size as u64 || length > maximum as u64)
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "artifact_integrity_mismatch",
            "Shennong DB Artifact size changed during download",
        ));
    }
    let mut bytes = Vec::with_capacity(expected_size);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| map_upstream(UpstreamError::Request(error)))?;
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(ApiError::invalid(
                "project artifact exceeds the runtime staging limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != expected_size || sha256_hex(&bytes) != expected_digest {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "artifact_integrity_mismatch",
            "Shennong DB Artifact content did not match its immutable manifest",
        ));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| ApiError::invalid("V1 Runtime staging accepts UTF-8 text Artifacts only"))?;
    Ok(json!({
        "uri":format!("project://current/resources/{resource_id}/artifacts/{artifact_id}"),
        "content":content,
        "size_bytes":expected_size,
        "truncated":false,
        "content_sha256":expected_digest,
        "version":1,
        "content_is_untrusted":true,
        "source_kind":"bound-resource-artifact"
    }))
}

fn governed_query_scope(body: &Value) -> Result<(Uuid, &str), ApiError> {
    let object = body
        .as_object()
        .ok_or_else(|| ApiError::invalid("query body must be an object"))?;
    let project_id = object
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("project_id is required and must be a UUID string"))?
        .parse::<Uuid>()
        .map_err(|_| ApiError::invalid("project_id is required and must be a UUID string"))?;
    let resource = object
        .get("resource")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("resource is required"))?;
    if validate_segment(resource).is_err() {
        return Err(ApiError::invalid("resource must be a valid identifier"));
    }
    Ok((project_id, resource))
}

fn project_resource_is_bound(body: &Value, project_id: Uuid, resource: &str) -> bool {
    let Some(data) = body.get("data").and_then(Value::as_object) else {
        return false;
    };
    let Some(resources) = data.get("resources").and_then(Value::as_array) else {
        return false;
    };
    let Some(bindings) = data.get("bindings").and_then(Value::as_array) else {
        return false;
    };
    let project_id = project_id.to_string();
    resources
        .iter()
        .any(|row| row.get("id").and_then(Value::as_str) == Some(resource))
        && bindings.iter().any(|row| {
            row.get("project_id").and_then(Value::as_str) == Some(project_id.as_str())
                && row.get("resource_id").and_then(Value::as_str) == Some(resource)
        })
}

fn merge_discoverable_resources(
    project_body: &Value,
    public_body: &Value,
    project_id: Uuid,
    query: Option<&str>,
    limit: usize,
) -> Result<Value, ApiError> {
    let project_resources = project_body
        .pointer("/data/resources")
        .and_then(Value::as_array)
        .ok_or_else(invalid_data_plane_response)?;
    project_body
        .pointer("/data/bindings")
        .and_then(Value::as_array)
        .ok_or_else(invalid_data_plane_response)?;
    let public_resources = public_body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(invalid_data_plane_response)?;

    let mut resources = Vec::with_capacity(limit);
    let mut seen = HashSet::with_capacity(limit);
    for resource in project_resources {
        let Some(id) = resource.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !project_resource_is_bound(project_body, project_id, id)
            || !resource_matches_query(resource, query)
            || !seen.insert(id.to_owned())
        {
            continue;
        }
        resources.push(resource.clone());
        if resources.len() == limit {
            return Ok(json!({"data":resources}));
        }
    }
    for resource in public_resources {
        let Some(id) = resource.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !is_explicitly_public_resource(resource)
            || !resource_matches_query(resource, query)
            || !seen.insert(id.to_owned())
        {
            continue;
        }
        resources.push(resource.clone());
        if resources.len() == limit {
            break;
        }
    }
    Ok(json!({"data":resources}))
}

fn resource_matches_query(resource: &Value, query: Option<&str>) -> bool {
    let Some(query) = query else { return true };
    let query = query.to_lowercase();
    ["id", "kind"]
        .into_iter()
        .filter_map(|field| resource.get(field).and_then(Value::as_str))
        .any(|value| value.to_lowercase().contains(&query))
        || resource
            .get("metadata")
            .is_some_and(|metadata| metadata.to_string().to_lowercase().contains(&query))
}

fn is_explicitly_public_resource(resource: &Value) -> bool {
    resource
        .pointer("/permissions/visibility")
        .and_then(Value::as_str)
        == Some("public")
}

fn invalid_data_plane_response() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "data_plane_invalid_response",
        "Shennong DB returned an invalid governed response",
    )
}

fn filter_public_resource_list(mut response: JsonResponse) -> Result<JsonResponse, ApiError> {
    if !response.status.is_success() {
        return Err(public_data_plane_error(response.status));
    }
    let resources = response
        .body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(invalid_data_plane_response)?;
    let resources = resources
        .iter()
        .filter(|resource| is_explicitly_public_resource(resource))
        .take(PUBLIC_RESOURCE_CATALOG_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    response.body = json!({"data":resources});
    Ok(response)
}

fn public_data_plane_error(status: StatusCode) -> ApiError {
    if status == StatusCode::NOT_FOUND {
        ApiError::not_found()
    } else {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "data_plane_rejected",
            "Shennong DB rejected the public Resource request",
        )
    }
}

fn require_public_resource_response(
    response: &JsonResponse,
    expected_id: &str,
) -> Result<(), ApiError> {
    if response.status == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found());
    }
    if !response.status.is_success() {
        return Err(public_data_plane_error(response.status));
    }
    let resource = response
        .body
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(invalid_data_plane_response)?;
    if resource.get("id").and_then(Value::as_str) != Some(expected_id) {
        return Err(invalid_data_plane_response());
    }
    if !is_explicitly_public_resource(resource) {
        return Err(ApiError::not_found());
    }
    Ok(())
}

fn sanitize_public_resource_response(
    mut response: JsonResponse,
    expected_id: &str,
) -> Result<JsonResponse, ApiError> {
    require_public_resource_response(&response, expected_id)?;
    let resource = response
        .body
        .get("data")
        .cloned()
        .ok_or_else(invalid_data_plane_response)?;
    response.body = json!({"data":resource});
    Ok(response)
}

async fn sanitize_public_resource_child(
    state: &AppState,
    resource_id: &str,
    child: &str,
    mut response: JsonResponse,
) -> Result<JsonResponse, ApiError> {
    if !response.status.is_success() {
        return Err(public_data_plane_error(response.status));
    }
    match child {
        "artifacts" => {
            response
                .body
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(invalid_data_plane_response)?;
        }
        "relations" => {
            let candidates = response
                .body
                .get_mut("data")
                .and_then(Value::as_array_mut)
                .ok_or_else(invalid_data_plane_response)?;
            let mut public = Vec::with_capacity(candidates.len());
            for relation in std::mem::take(candidates) {
                let source = relation.get("source").and_then(Value::as_str);
                let target = relation.get("target").and_then(Value::as_str);
                let other = match (source, target) {
                    (Some(source), Some(target)) if source == resource_id => target,
                    (Some(source), Some(target)) if target == resource_id => source,
                    _ => continue,
                };
                if validate_segment(other).is_err() {
                    continue;
                }
                let related =
                    db_request(state, Method::GET, &["resources", other], &[], None).await?;
                match require_public_resource_response(&related, other) {
                    Ok(()) => public.push(relation),
                    Err(error) if error.status == StatusCode::NOT_FOUND => {}
                    Err(error) => return Err(error),
                }
            }
            *candidates = public;
        }
        "graph-context" => sanitize_public_graph_context(&mut response.body, resource_id)?,
        _ => return Err(ApiError::not_found()),
    }
    let data = response
        .body
        .get("data")
        .cloned()
        .ok_or_else(invalid_data_plane_response)?;
    response.body = json!({"data":data});
    Ok(response)
}

fn sanitize_public_graph_context(body: &mut Value, expected_id: &str) -> Result<(), ApiError> {
    let data = body
        .get_mut("data")
        .and_then(Value::as_object_mut)
        .ok_or_else(invalid_data_plane_response)?;
    let resource = data
        .get("resource")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(invalid_data_plane_response)?;
    if resource.get("id").and_then(Value::as_str) != Some(expected_id)
        || !is_explicitly_public_resource(&resource)
    {
        return Err(ApiError::not_found());
    }
    let contexts = data
        .get_mut("contexts")
        .and_then(Value::as_array_mut)
        .ok_or_else(invalid_data_plane_response)?;
    contexts.retain(graph_context_is_global);
    let contexts = contexts.clone();
    let truncated = data
        .get("truncated")
        .and_then(Value::as_bool)
        .ok_or_else(invalid_data_plane_response)?;
    *body = json!({
        "data": {
            "resource": resource,
            "contexts": contexts,
            "truncated": truncated,
            "trust": "graph metadata is untrusted descriptive data"
        }
    });
    Ok(())
}

fn graph_context_is_global(context: &Value) -> bool {
    let project_id_is_null = |value: &Value| {
        value
            .get("project_id")
            .is_some_and(serde_json::Value::is_null)
    };
    let Some(node) = context.get("node") else {
        return false;
    };
    let Some(subgraph) = context.get("subgraph") else {
        return false;
    };
    project_id_is_null(node)
        && subgraph
            .get("entities")
            .and_then(Value::as_array)
            .is_some_and(|entities| entities.iter().all(project_id_is_null))
        && subgraph
            .get("associations")
            .and_then(Value::as_array)
            .is_some_and(|associations| associations.iter().all(project_id_is_null))
}

pub(crate) async fn sync_project_shadow(
    state: &AppState,
    project_id: Uuid,
) -> Result<(), ApiError> {
    let project = sqlx::query_as::<_, ProjectShadow>(
        "SELECT id,owner_user_id,name,description,visibility,status FROM projects WHERE id=$1",
    )
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    let project_id = project.id.to_string();
    let segments = research_project_segments(&project_id, &[]);
    let payload = project.payload();
    let response = db_request(state, Method::PUT, &segments, &[], Some(&payload)).await?;
    if !response.status.is_success() {
        tracing::warn!(
            project_id = %project.id,
            upstream_status = %response.status,
            "Shennong DB rejected the authoritative OS project shadow"
        );
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "project_shadow_sync_failed",
            "Shennong DB could not synchronize the authoritative project record",
        ));
    }
    Ok(())
}

async fn proxy_json(
    state: &AppState,
    method: Method,
    segments: &[&str],
    query: Vec<(&str, String)>,
    body: Option<&Value>,
) -> Result<Response, ApiError> {
    json_response(db_request(state, method, segments, &query, body).await?)
}

async fn db_request(
    state: &AppState,
    method: Method,
    segments: &[&str],
    query: &[(&str, String)],
    body: Option<&Value>,
) -> Result<JsonResponse, ApiError> {
    let client = state.config.db_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_unavailable",
            "Shennong DB is not configured",
        )
    })?;
    let key = state.config.db_admin_key.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "data_plane_auth_unavailable",
            "Shennong DB credential is not configured",
        )
    })?;
    let api_segments = db_api_segments(segments);
    client
        .request_json(
            method,
            &api_segments,
            query,
            body,
            Some(("x-shennong-admin-key", key)),
            None,
        )
        .await
        .map_err(map_upstream)
}

fn db_api_segments<'a>(segments: &'a [&'a str]) -> Vec<&'a str> {
    let mut api_segments = Vec::with_capacity(segments.len() + 2);
    api_segments.extend(["api", "v1"]);
    api_segments.extend_from_slice(segments);
    api_segments
}

fn research_project_segments<'a>(project: &'a str, tail: &'a [&'a str]) -> Vec<&'a str> {
    let mut segments = Vec::with_capacity(tail.len() + 2);
    segments.extend(["research-projects", project]);
    segments.extend_from_slice(tail);
    segments
}

fn project_subgraph_query(
    project_id: Uuid,
    request: &ProjectSubgraphQuery,
) -> Result<Vec<(&'static str, String)>, ApiError> {
    validate_segment(&request.root)?;
    let depth = request.depth.unwrap_or(DEFAULT_GRAPH_DEPTH);
    if !(1..=3).contains(&depth) {
        return Err(ApiError::invalid("depth must be between 1 and 3"));
    }
    let limit = request.limit.unwrap_or(DEFAULT_GRAPH_LIMIT);
    if !(1..=200).contains(&limit) {
        return Err(ApiError::invalid("limit must be between 1 and 200"));
    }
    Ok(vec![
        ("root", request.root.clone()),
        ("depth", depth.to_string()),
        ("limit", limit.to_string()),
        ("project_id", project_id.to_string()),
    ])
}

fn json_response(response: JsonResponse) -> Result<Response, ApiError> {
    Ok((response.status, Json(response.body)).into_response())
}

fn query_pairs(raw: Option<&str>) -> Vec<(&'static str, String)> {
    let Some(raw) = raw else { return Vec::new() };
    url::form_urlencoded::parse(raw.as_bytes())
        .filter_map(|(name, value)| match name.as_ref() {
            "q" | "limit" | "cursor" | "offset" | "kind" | "type" | "status" | "include" => Some((
                match name.as_ref() {
                    "q" => "q",
                    "limit" => "limit",
                    "cursor" => "cursor",
                    "offset" => "offset",
                    "kind" => "kind",
                    "type" => "type",
                    "status" => "status",
                    "include" => "include",
                    _ => unreachable!(),
                },
                value.into_owned(),
            )),
            _ => None,
        })
        .take(32)
        .collect()
}

fn allowed_project_method(path: &[&str], method: &Method) -> bool {
    match path {
        ["context-pack"] => method == Method::GET,
        ["entities"] | ["activities"] | ["studies"] | ["associations"] | ["evidence"] => {
            matches!(*method, Method::GET | Method::POST)
        }
        ["activities", _, "io"] | ["activities", _, "actors"] => {
            matches!(*method, Method::GET | Method::POST)
        }
        ["associations", _, "evidence"] => method == Method::GET,
        ["associations", _, "evidence", _] => method == Method::PUT,
        ["resources"] => method == Method::GET,
        ["resources", _] => matches!(*method, Method::PUT | Method::DELETE),
        _ => false,
    }
}

fn validate_segment(value: &str) -> Result<(), ApiError> {
    if value.is_empty()
        || value.len() > 180
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(ApiError::not_found());
    }
    Ok(())
}

fn map_upstream(error: UpstreamError) -> ApiError {
    tracing::error!(%error, "data-plane request failed");
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "data_plane_upstream_failed",
        "Shennong DB could not be reached safely",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ProjectShadow, ProjectSubgraphQuery, PublicResourceListQuery, agent_resource_query_body,
        allowed_project_method, bound_artifact_contract, bounded_agent_resource_limit,
        db_api_segments, filter_public_resource_list, governed_query_scope,
        merge_discoverable_resources, project_resource_is_bound, project_subgraph_query,
        public_resource_query, require_public_resource_response, research_project_segments,
        validate_upload_headers,
    };
    use crate::clients::JsonResponse;
    use axum::http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    };
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn data_plane_requests_use_the_shennong_db_v1_prefix() {
        assert_eq!(
            db_api_segments(&["resources", "resource-1"]),
            ["api", "v1", "resources", "resource-1"]
        );
    }

    #[test]
    fn project_data_uses_the_headless_research_project_contract() {
        assert_eq!(
            research_project_segments("project-1", &["resources"]),
            ["research-projects", "project-1", "resources"]
        );
    }

    #[test]
    fn project_data_methods_match_the_headless_db_allowlist() {
        assert!(allowed_project_method(
            &["resources", "resource-1"],
            &Method::PUT
        ));
        assert!(allowed_project_method(
            &["activities", "activity-1", "io"],
            &Method::POST
        ));
        assert!(allowed_project_method(
            &["associations", "association-1", "evidence", "evidence-1"],
            &Method::PUT
        ));
        assert!(!allowed_project_method(&["resources"], &Method::PUT));
        assert!(!allowed_project_method(
            &["entities", "entity-1"],
            &Method::DELETE
        ));
    }

    #[test]
    fn project_subgraph_query_is_bounded_and_project_scoped() {
        let project_id = Uuid::from_u128(7);
        let query = project_subgraph_query(
            project_id,
            &ProjectSubgraphQuery {
                root: "sample-1".into(),
                depth: None,
                limit: None,
            },
        )
        .expect("valid graph query");
        assert_eq!(
            query,
            vec![
                ("root", "sample-1".into()),
                ("depth", "1".into()),
                ("limit", "80".into()),
                ("project_id", project_id.to_string()),
            ]
        );
    }

    #[test]
    fn project_subgraph_query_rejects_invalid_identifiers_and_bounds() {
        let project_id = Uuid::nil();
        let invalid_root = project_subgraph_query(
            project_id,
            &ProjectSubgraphQuery {
                root: "../sample".into(),
                depth: None,
                limit: None,
            },
        )
        .expect_err("invalid graph root");
        assert_eq!(invalid_root.status, StatusCode::NOT_FOUND);

        for (depth, limit) in [
            (Some(0), None),
            (Some(4), None),
            (None, Some(0)),
            (None, Some(201)),
        ] {
            let error = project_subgraph_query(
                project_id,
                &ProjectSubgraphQuery {
                    root: "sample-1".into(),
                    depth,
                    limit,
                },
            )
            .expect_err("out-of-range graph query");
            assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    #[test]
    fn project_upload_metadata_is_bounded_and_path_safe() {
        let mut headers = HeaderMap::new();
        headers.insert("x-filename", HeaderValue::from_static("counts.tsv.gz"));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/tab-separated-values"),
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("4096"));
        assert_eq!(
            validate_upload_headers(&headers, 8192).unwrap(),
            (
                "counts.tsv.gz".into(),
                "text/tab-separated-values".into(),
                4096
            )
        );

        headers.insert("x-filename", HeaderValue::from_static("../counts.tsv"));
        assert_eq!(
            validate_upload_headers(&headers, 8192).unwrap_err().status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        headers.insert("x-filename", HeaderValue::from_static("counts.tsv"));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("8193"));
        assert_eq!(
            validate_upload_headers(&headers, 8192).unwrap_err().status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        headers.remove(CONTENT_LENGTH);
        assert_eq!(
            validate_upload_headers(&headers, 8192).unwrap_err().status,
            StatusCode::LENGTH_REQUIRED
        );
    }

    #[test]
    fn public_catalog_drops_private_and_malformed_resources() {
        let response = filter_public_resource_list(JsonResponse {
            status: StatusCode::OK,
            body: json!({
                "data": [
                    {"id":"public-1","permissions":{"visibility":"public"}},
                    {"id":"private-1","permissions":{"visibility":"private"}},
                    {"id":"missing-visibility","permissions":{}},
                    {"id":"missing-permissions"}
                ]
            }),
        })
        .expect("valid public catalog envelope");
        assert_eq!(
            response.body,
            json!({"data":[{"id":"public-1","permissions":{"visibility":"public"}}]})
        );

        let malformed = filter_public_resource_list(JsonResponse {
            status: StatusCode::OK,
            body: json!({"data":{"id":"private-1"}}),
        })
        .expect_err("malformed catalog must fail closed");
        assert_eq!(malformed.status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn public_catalog_is_defensively_truncated() {
        let resources = (0..105)
            .map(|index| {
                json!({
                    "id":format!("public-{index}"),
                    "permissions":{"visibility":"public"}
                })
            })
            .collect::<Vec<_>>();
        let response = filter_public_resource_list(JsonResponse {
            status: StatusCode::OK,
            body: json!({"data":resources}),
        })
        .expect("valid public catalog envelope");
        assert_eq!(response.body["data"].as_array().unwrap().len(), 100);
    }

    #[test]
    fn public_catalog_query_matches_the_documented_bounds() {
        let query = PublicResourceListQuery {
            q: Some("  PBMC  ".into()),
            limit: Some(25),
            offset: None,
            cursor: Some("10".into()),
        };
        assert_eq!(
            public_resource_query(&query).unwrap(),
            vec![
                ("q", "PBMC".into()),
                ("limit", "25".into()),
                ("offset", "10".into())
            ]
        );

        for query in [
            PublicResourceListQuery {
                q: None,
                limit: Some(0),
                offset: None,
                cursor: None,
            },
            PublicResourceListQuery {
                q: None,
                limit: Some(101),
                offset: None,
                cursor: None,
            },
            PublicResourceListQuery {
                q: None,
                limit: None,
                offset: Some(1),
                cursor: Some("2".into()),
            },
        ] {
            assert_eq!(
                public_resource_query(&query).unwrap_err().status,
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
    }

    #[test]
    fn agent_discovery_prefers_bound_project_resources_and_adds_only_public_global_rows() {
        let project_id = Uuid::from_u128(21);
        let project = json!({"data":{
            "resources":[
                {"id":"bound-private","kind":"Dataset","metadata":{"assay":"RNA-seq"},"permissions":{"visibility":"private"}},
                {"id":"unbound-private","kind":"Dataset","metadata":{"assay":"RNA-seq"},"permissions":{"visibility":"private"}}
            ],
            "bindings":[
                {"project_id":project_id,"resource_id":"bound-private","role":"input"}
            ]
        }});
        let global = json!({"data":[
            {"id":"global-public","kind":"Dataset","metadata":{"assay":"RNA-seq"},"permissions":{"visibility":"public"}},
            {"id":"malicious-private","kind":"Dataset","metadata":{"assay":"RNA-seq"},"permissions":{"visibility":"private"}}
        ]});
        let result =
            merge_discoverable_resources(&project, &global, project_id, Some("rna-seq"), 10)
                .expect("valid governed discovery envelopes");
        let ids = result["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|resource| resource["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["bound-private", "global-public"]);

        let first = merge_discoverable_resources(&project, &global, project_id, None, 1)
            .expect("bounded governed discovery");
        assert_eq!(first["data"][0]["id"], "bound-private");
    }

    #[test]
    fn agent_discovery_limit_is_clamped_at_both_boundaries() {
        assert_eq!(bounded_agent_resource_limit(None).unwrap(), 20);
        assert_eq!(bounded_agent_resource_limit(Some(&json!(-1))).unwrap(), 1);
        assert_eq!(
            bounded_agent_resource_limit(Some(&json!(500))).unwrap(),
            100
        );
        assert!(bounded_agent_resource_limit(Some(&json!("20"))).is_err());
    }

    #[test]
    fn private_or_unlabelled_resource_is_hidden_as_not_found() {
        for permissions in [json!({"visibility":"private"}), json!({})] {
            let response = JsonResponse {
                status: StatusCode::OK,
                body: json!({"data":{"id":"resource-1","permissions":permissions}}),
            };
            let error = require_public_resource_response(&response, "resource-1")
                .expect_err("non-public resource must be hidden");
            assert_eq!(error.status, StatusCode::NOT_FOUND);
        }
    }

    #[test]
    fn governed_query_requires_project_resource_and_exact_binding() {
        let project_id = Uuid::from_u128(11);
        for body in [
            json!({"resource":"resource-1"}),
            json!({"project_id":project_id}),
            json!({"project_id":project_id,"resource":"../resource-1"}),
        ] {
            let error = governed_query_scope(&body).expect_err("query scope must be complete");
            assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        }
        let body = json!({"project_id":project_id,"resource":"resource-1"});
        assert_eq!(
            governed_query_scope(&body).expect("valid governed query"),
            (project_id, "resource-1")
        );

        let bound = json!({"data":{
            "resources":[{"id":"resource-1","permissions":{"visibility":"private"}}],
            "bindings":[{"project_id":project_id,"resource_id":"resource-1","role":"input"}]
        }});
        assert!(project_resource_is_bound(&bound, project_id, "resource-1"));
        assert!(!project_resource_is_bound(
            &bound,
            Uuid::from_u128(12),
            "resource-1"
        ));
        assert!(!project_resource_is_bound(
            &json!({"data":{"resources":[{"id":"resource-1"}],"bindings":[]}}),
            project_id,
            "resource-1"
        ));
    }

    #[test]
    fn agent_query_is_adapted_to_shennong_db_resource_query_contract() {
        let project_id = Uuid::from_u128(11);
        let body = agent_resource_query_body(
            project_id,
            &json!({
                "resource": "pbmc-3k",
                "operation": "expression",
                "feature": "CD3D",
                "context": {},
                "limit": 10
            }),
        )
        .expect("valid agent query");
        assert_eq!(
            body,
            json!({
                "project_id": project_id,
                "resource": "pbmc-3k",
                "operation": "expression",
                "feature": {"type": "gene", "name": "CD3D"},
                "context": {},
                "options": {"limit": 10}
            })
        );
    }

    #[test]
    fn agent_query_defaults_and_bounds_limit() {
        let project_id = Uuid::from_u128(11);
        let defaulted = agent_resource_query_body(
            project_id,
            &json!({"resource":"pbmc-3k","operation":"expression"}),
        )
        .expect("defaulted query");
        assert_eq!(defaulted["options"]["limit"], 100);

        let bounded = agent_resource_query_body(
            project_id,
            &json!({"resource":"pbmc-3k","operation":"expression","limit":10000}),
        )
        .expect("bounded query");
        assert_eq!(bounded["options"]["limit"], 1000);
    }

    #[test]
    fn runtime_staging_accepts_only_exact_immutable_bounded_artifact_manifests() {
        let valid = json!({"data":[{
            "id":"upload-1234",
            "resource_id":"cohort-a",
            "size":42,
            "content_sha256":"A".repeat(64),
            "immutable":true,
            "storage_backend":"s3"
        }]});
        assert_eq!(
            bound_artifact_contract(&valid, "cohort-a", "upload-1234", 1_048_576)
                .expect("valid immutable Artifact manifest"),
            (42, "a".repeat(64))
        );

        for invalid in [
            json!({"data":[{
                "id":"upload-1234","resource_id":"other-resource","size":42,
                "content_sha256":"a".repeat(64),"immutable":true,"storage_backend":"s3"
            }]}),
            json!({"data":[{
                "id":"upload-1234","resource_id":"cohort-a","size":42,
                "content_sha256":"a".repeat(64),"immutable":false,"storage_backend":"s3"
            }]}),
            json!({"data":[{
                "id":"upload-1234","resource_id":"cohort-a","size":1_048_577,
                "content_sha256":"a".repeat(64),"immutable":true,"storage_backend":"s3"
            }]}),
            json!({"data":[{
                "id":"upload-1234","resource_id":"cohort-a","size":42,
                "content_sha256":"not-a-digest","immutable":true,"storage_backend":"s3"
            }]}),
            json!({"data":[{
                "id":"upload-1234","resource_id":"cohort-a","size":42,
                "content_sha256":"a".repeat(64),"immutable":true,"storage_backend":"http"
            }]}),
        ] {
            assert!(
                bound_artifact_contract(&invalid, "cohort-a", "upload-1234", 1_048_576).is_err()
            );
        }
    }

    #[test]
    fn project_shadow_payload_marks_os_as_the_authority() {
        let project = ProjectShadow {
            id: Uuid::nil(),
            owner_user_id: Uuid::from_u128(1),
            name: "Cancer atlas".into(),
            description: "Governed analysis".into(),
            visibility: "private".into(),
            status: "active".into(),
        };
        assert_eq!(
            project.payload(),
            json!({
                "id": Uuid::nil(),
                "owner_user_id": Uuid::from_u128(1),
                "name": "Cancer atlas",
                "description": "Governed analysis",
                "visibility": "private",
                "status": "active",
                "metadata": {"authority": "shennong-os"}
            })
        );
    }
}
