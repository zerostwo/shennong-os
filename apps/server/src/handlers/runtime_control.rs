use super::projects::{require_project_read, require_project_write};
use super::{Envelope, audit};
use crate::{
    AppState,
    auth::{AuthUser, authenticate},
    clients::{JsonResponse, UpstreamError},
    config::RuntimeJwtSigner,
    crypto::{random_secret, sha256},
    error::ApiError,
};
use axum::{
    Json,
    body::Body,
    extract::{
        FromRequestParts, OriginalUri, Path, Query, State,
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode,
        header::{CONTENT_TYPE, COOKIE, HOST, ORIGIN, SET_COOKIE},
    },
    response::{IntoResponse, Response},
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use tokio_tungstenite::{connect_async, tungstenite};
use uuid::Uuid;

const RSTUDIO_REQUEST_HEADER: &str = "x-shennong-rstudio-request";

#[derive(Serialize)]
struct RuntimeClaims {
    iss: String,
    aud: String,
    sub: String,
    exp: i64,
    iat: i64,
    jti: String,
    scopes: Vec<String>,
    workspace_refs: Vec<String>,
}

#[derive(Deserialize)]
pub struct SessionCreate {
    kind: String,
    worker_profile: Option<String>,
    idle_timeout_seconds: Option<u64>,
    max_lifetime_seconds: Option<u64>,
}

#[derive(Deserialize)]
pub struct IdeLaunchQuery {
    ticket: String,
}

struct IdeAccess {
    actor: AuthUser,
    project_id: Uuid,
}

pub async fn list_all_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let rows = if actor.role == "admin" {
        sqlx::query("SELECT * FROM jobs ORDER BY created_at DESC LIMIT 500")
            .fetch_all(&state.pool)
            .await
    } else {
        sqlx::query(
            "SELECT j.* FROM jobs j JOIN project_members pm ON pm.project_id=j.project_id \
             WHERE pm.user_id=$1 ORDER BY j.created_at DESC LIMIT 500",
        )
        .bind(actor.id)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(job_view).collect(),
    }))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let row = sqlx::query("SELECT project_id FROM jobs WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    let project_id: Uuid = row.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    let value = runtime_job_action(&state, &actor, project_id, id, "cancel").await?;
    persist_runtime_job_view(&state, id, &value).await?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "runtime.job_cancel",
        "job",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(Json(Envelope { data: value }))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, project_id).await?;
    let rows = sqlx::query(
        "SELECT * FROM runtime_sessions WHERE project_id=$1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(session_view).collect(),
    }))
}

pub async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(value): Json<SessionCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, project_id).await?;
    if !matches!(value.kind.as_str(), "rstudio" | "jupyterlab") {
        return Err(ApiError::invalid(
            "session kind must be rstudio or jupyterlab",
        ));
    }
    let idle = value
        .idle_timeout_seconds
        .unwrap_or(1800)
        .clamp(300, 28_800);
    let lifetime = value
        .max_lifetime_seconds
        .unwrap_or(28_800)
        .clamp(idle, 28_800);
    // Profile selection is server-side. The WebUI's friendly profile label is
    // deliberately not forwarded as authority to the execution plane.
    let profile = match value.worker_profile.as_deref() {
        Some("ide-small") | None | Some("interactive-standard") => "ide-small",
        Some(_) => return Err(ApiError::invalid("unsupported interactive worker profile")),
    };
    let body = json!({
        "api_version":"shennong.dev/v1",
        "workspace_ref":workspace_ref(project_id),
        "worker_profile":profile,
        "kind":value.kind,
        "resources":default_session_resources(),
        "network":"internet_only",
        "idle_timeout_seconds":idle,
        "max_lifetime_seconds":lifetime
    });
    let response = runtime_request(
        &state,
        &actor,
        project_id,
        Method::POST,
        &["v1", "sessions"],
        Some(&body),
        Some(&format!("session-{}", Uuid::new_v4())),
        &["runtime:sessions:write"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    let id: Uuid = response
        .body
        .get("id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| upstream_invalid("runtime session response omitted id"))?;
    let status = runtime_status(&response.body, "starting");
    let expires_at = response
        .body
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<chrono::DateTime<Utc>>().ok());
    sqlx::query(
        "INSERT INTO runtime_sessions(id,project_id,created_by_user_id,kind,worker_profile,status,runtime_view,expires_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(project_id)
    .bind(actor.id)
    .bind(&value.kind)
    .bind(profile)
    .bind(&status)
    .bind(&response.body)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "runtime.session_start",
        "runtime_session",
        Some(id.to_string()),
        json!({"kind":value.kind,"worker_profile":profile}),
    )
    .await?;
    let row = sqlx::query("SELECT * FROM runtime_sessions WHERE id=$1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: session_view(row),
        }),
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let project_id = session_project(&state, id).await?;
    require_project_read(&state, &actor, project_id).await?;
    let response = runtime_request(
        &state,
        &actor,
        project_id,
        Method::GET,
        &["v1", "sessions", &id.to_string()],
        None,
        None,
        &["runtime:sessions:read"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    persist_runtime_session_view(&state, id, &response.body).await?;
    let row = sqlx::query("SELECT * FROM runtime_sessions WHERE id=$1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: session_view(row),
    }))
}

pub async fn stop_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let project_id = session_project(&state, id).await?;
    require_project_write(&state, &actor, project_id).await?;
    let response = runtime_request(
        &state,
        &actor,
        project_id,
        Method::POST,
        &["v1", "sessions", &id.to_string(), "stop"],
        Some(&json!({})),
        None,
        &["runtime:sessions:write"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    persist_runtime_session_view(&state, id, &response.body).await?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "runtime.session_stop",
        "runtime_session",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    let row = sqlx::query("SELECT * FROM runtime_sessions WHERE id=$1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: session_view(row),
    }))
}

pub async fn launch_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let row = sqlx::query("SELECT project_id,created_by_user_id FROM runtime_sessions WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    let project_id: Uuid = row.get("project_id");
    if row.get::<Uuid, _>("created_by_user_id") != actor.id {
        return Err(ApiError::not_found());
    }
    require_project_read(&state, &actor, project_id).await?;
    let ide_origin = state
        .config
        .ide_public_origin
        .as_ref()
        .ok_or_else(ide_unavailable)?;
    let response = runtime_request(
        &state,
        &actor,
        project_id,
        Method::GET,
        &["v1", "sessions", &id.to_string()],
        None,
        None,
        &["runtime:sessions:read"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    let expected_proxy_path = format!("/v1/sessions/{id}/proxy/");
    if runtime_status(&response.body, "") != "running"
        || response.body.get("proxy_path").and_then(Value::as_str)
            != Some(expected_proxy_path.as_str())
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "session_not_ready",
            "Runtime Session is not ready for IDE access",
        ));
    }
    let runtime_expires_at = response
        .body
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<chrono::DateTime<Utc>>().ok())
        .ok_or_else(|| upstream_invalid("runtime session response omitted expires_at"))?;
    let now = Utc::now();
    let expires_at = std::cmp::min(now + ChronoDuration::seconds(60), runtime_expires_at);
    if expires_at <= now {
        return Err(ApiError::conflict("Runtime Session has expired"));
    }
    persist_runtime_session_view(&state, id, &response.body).await?;
    let ticket = random_secret(32);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE ide_launch_tickets SET used_at=NOW() \
         WHERE runtime_session_id=$1 AND user_id=$2 AND used_at IS NULL",
    )
    .bind(id)
    .bind(actor.id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    sqlx::query(
        "INSERT INTO ide_launch_tickets(\
           id,runtime_session_id,user_id,issued_from_session_id,token_hash,expires_at\
         ) VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(actor.id)
    .bind(actor.session_id)
    .bind(sha256(&ticket))
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let mut launch_url = ide_origin.clone();
    launch_url.set_path("/__shennong/launch");
    launch_url
        .query_pairs_mut()
        .clear()
        .append_pair("ticket", &ticket);
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "runtime.session_launch_ticket",
        "runtime_session",
        Some(id.to_string()),
        json!({"expires_at":expires_at}),
    )
    .await?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert("cache-control", HeaderValue::from_static("no-store"));
    response_headers.insert("pragma", HeaderValue::from_static("no-cache"));
    Ok((
        response_headers,
        Json(Envelope {
            data: json!({"launch_url":launch_url.as_str(),"expires_at":expires_at}),
        }),
    )
        .into_response())
}

pub async fn redeem_ide_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IdeLaunchQuery>,
) -> Result<Response, ApiError> {
    let ide_origin = require_ide_host(&state, &headers)?.clone();
    if !(32..=256).contains(&query.ticket.len()) {
        return Err(ApiError::unauthorized());
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let row = sqlx::query(
        "UPDATE ide_launch_tickets ticket SET used_at=NOW() \
         FROM runtime_sessions runtime_session,sessions os_session,users actor \
         WHERE ticket.token_hash=$1 AND ticket.used_at IS NULL AND ticket.expires_at>NOW() \
           AND runtime_session.id=ticket.runtime_session_id \
           AND runtime_session.created_by_user_id=ticket.user_id \
           AND runtime_session.status='running' AND runtime_session.expires_at>NOW() \
           AND os_session.id=ticket.issued_from_session_id \
           AND os_session.user_id=ticket.user_id AND os_session.revoked_at IS NULL \
           AND os_session.expires_at>NOW() AND actor.id=ticket.user_id AND actor.status='active' \
         RETURNING ticket.runtime_session_id,ticket.user_id,runtime_session.project_id, \
                   runtime_session.expires_at,actor.email,actor.display_name,actor.role",
    )
    .bind(sha256(&query.ticket))
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::unauthorized)?;
    let runtime_session_id: Uuid = row.get("runtime_session_id");
    let user_id: Uuid = row.get("user_id");
    let project_id: Uuid = row.get("project_id");
    let access_expires_at: chrono::DateTime<Utc> = row.get("expires_at");
    let actor = AuthUser::internal(
        user_id,
        row.get("email"),
        row.get("display_name"),
        row.get("role"),
    );
    require_project_read(&state, &actor, project_id).await?;
    let access_token = random_secret(32);
    sqlx::query(
        "INSERT INTO ide_access_sessions(\
           id,runtime_session_id,user_id,token_hash,expires_at\
         ) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(runtime_session_id)
    .bind(user_id)
    .bind(sha256(&access_token))
    .bind(access_expires_at)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let max_age = (access_expires_at - Utc::now()).num_seconds().max(1);
    ide_launch_interstitial_response(
        runtime_session_id,
        &access_token,
        max_age,
        ide_origin.scheme() == "https",
        &random_secret(24),
    )
}

fn ide_launch_interstitial_response(
    runtime_session_id: Uuid,
    access_token: &str,
    max_age: i64,
    secure_cookie: bool,
    nonce: &str,
) -> Result<Response, ApiError> {
    if nonce.len() < 16
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::invalid("invalid IDE launch nonce"));
    }
    let proxy_path = format!("/v1/sessions/{runtime_session_id}/proxy");
    let target = format!("{proxy_path}/");
    let target_json = serde_json::to_string(&target)
        .map_err(|_| ApiError::invalid("invalid IDE launch target"))?;
    let secure = if secure_cookie { "; Secure" } else { "" };
    let cookie = format!(
        "shennong_ide_session={access_token}; Path={proxy_path}; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        max_age.max(1)
    );
    let content_security_policy = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; script-src-attr 'none'; \
         connect-src 'none'; img-src 'none'; style-src 'none'; font-src 'none'; \
         object-src 'none'; media-src 'none'; manifest-src 'none'; worker-src 'none'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    // The cookie deliberately remains SameSite=Strict. A direct cross-site 303
    // from the OS origin would therefore omit it on the redirected request.
    // This minimal IDE-origin document establishes the same-site context first,
    // then navigates only to the server-derived, relative Session proxy path.
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>Opening IDE</title>\
         <noscript><meta http-equiv=\"refresh\" content=\"0;url={target}\"></noscript>\
         </head><body><script nonce=\"{nonce}\">window.location.replace({target_json});</script>\
         <noscript><p><a href=\"{target}\">Continue to the IDE</a></p></noscript>\
         </body></html>"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(
            SET_COOKIE,
            HeaderValue::from_str(&cookie)
                .map_err(|_| ApiError::invalid("invalid IDE access cookie"))?,
        )
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .header("pragma", "no-cache")
        .header("referrer-policy", "no-referrer")
        .header("content-security-policy", content_security_policy)
        .header("x-content-type-options", "nosniff")
        .header("cross-origin-opener-policy", "same-origin")
        .header("cross-origin-resource-policy", "same-origin")
        .body(Body::from(body))
        .map_err(|_| ApiError::invalid("IDE launch response could not be created"))
}

pub async fn ide_proxy_root(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    OriginalUri(uri): OriginalUri,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    proxy_ide_request(state, id, None, uri, request).await
}

pub async fn ide_proxy_path(
    State(state): State<AppState>,
    Path((id, path)): Path<(Uuid, String)>,
    OriginalUri(uri): OriginalUri,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    proxy_ide_request(state, id, Some(path), uri, request).await
}

pub(crate) async fn submit_agent_job(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    run_id: Uuid,
    tool_call_id: &str,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let raw = arguments
        .get("job_spec")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::invalid("job_spec must be an object"))?;
    let argv = raw
        .get("argv")
        .cloned()
        .ok_or_else(|| ApiError::invalid("job_spec.argv is required"))?;
    let resources = raw
        .get("resources")
        .cloned()
        .unwrap_or_else(default_job_resources);
    let artifact_rules = raw
        .get("artifact_rules")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let workspace_files = raw
        .get("workspace_files")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let profile = match raw.get("worker_profile").and_then(Value::as_str) {
        None | Some("cpu-small") | Some("standard") => "cpu-small",
        Some(_) => return Err(ApiError::invalid("unsupported batch worker profile")),
    };
    let body = json!({
        "api_version":"shennong.dev/v1",
        "workspace_ref":workspace_ref(project_id),
        "worker_profile":profile,
        "argv":argv,
        "resources":resources,
        "network":"internet_only",
        "workspace_files":workspace_files,
        "artifact_rules":artifact_rules
    });
    let key = format!("agent-{run_id}-{tool_call_id}");
    if key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(ApiError::invalid(
            "tool call id cannot form a runtime idempotency key",
        ));
    }
    let response = runtime_request(
        state,
        actor,
        project_id,
        Method::POST,
        &["v1", "jobs"],
        Some(&body),
        Some(&key),
        &["runtime:jobs:write"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    let id: Uuid = response
        .body
        .get("id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| upstream_invalid("runtime job response omitted id"))?;
    let status = runtime_status(&response.body, "queued");
    sqlx::query(
        "INSERT INTO jobs(id,project_id,run_id,kind,status,spec,result,created_by_user_id) \
         VALUES($1,$2,$3,'analysis',$4,$5,$6,$7) \
         ON CONFLICT(id) DO UPDATE SET status=EXCLUDED.status,result=EXCLUDED.result,updated_at=NOW()",
    )
    .bind(id)
    .bind(project_id)
    .bind(run_id)
    .bind(status)
    .bind(body)
    .bind(&response.body)
    .bind(actor.id)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(response.body)
}

pub(crate) async fn agent_job_action(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    job_id: Uuid,
    action: &str,
    include_logs: bool,
) -> Result<Value, ApiError> {
    require_agent_job_project(state, project_id, job_id).await?;
    let mut value = runtime_job_action(state, actor, project_id, job_id, action).await?;
    persist_runtime_job_view(state, job_id, &value).await?;
    if action == "get" {
        let id_text = job_id.to_string();
        if include_logs {
            let logs = runtime_request_with_query(
                state,
                actor,
                project_id,
                Method::GET,
                &["v1", "jobs", &id_text, "logs"],
                &[("after", "0".to_string()), ("limit", "200".to_string())],
                None,
                None,
                &["runtime:jobs:read"],
            )
            .await?;
            ensure_upstream_success(logs.status, &logs.body)?;
            value
                .as_object_mut()
                .ok_or_else(|| upstream_invalid("runtime job response must be an object"))?
                .insert("logs".into(), logs.body);
        }
        value
            .as_object_mut()
            .ok_or_else(|| upstream_invalid("runtime job response must be an object"))?
            .insert(
                "artifacts".into(),
                Value::Array(agent_job_artifacts(state, actor, project_id, job_id).await?),
            );
    }
    Ok(value)
}

async fn require_agent_job_project(
    state: &AppState,
    project_id: Uuid,
    job_id: Uuid,
) -> Result<(), ApiError> {
    let row = sqlx::query("SELECT project_id FROM jobs WHERE id=$1")
        .bind(job_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    if row.get::<Uuid, _>("project_id") != project_id {
        return Err(ApiError::not_found());
    }
    Ok(())
}

pub(crate) async fn agent_job_artifacts(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    job_id: Uuid,
) -> Result<Vec<Value>, ApiError> {
    require_agent_job_project(state, project_id, job_id).await?;
    let id_text = job_id.to_string();
    let response = runtime_request(
        state,
        actor,
        project_id,
        Method::GET,
        &["v1", "jobs", &id_text, "artifacts"],
        None,
        None,
        &["runtime:jobs:read"],
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    response
        .body
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| upstream_invalid("runtime artifact response omitted artifacts"))
}

async fn runtime_job_action(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    id: Uuid,
    action: &str,
) -> Result<Value, ApiError> {
    let id_text = id.to_string();
    let (method, segments, scopes): (Method, Vec<&str>, &[&str]) = match action {
        "get" => (
            Method::GET,
            vec!["v1", "jobs", &id_text],
            &["runtime:jobs:read"],
        ),
        "cancel" => (
            Method::POST,
            vec!["v1", "jobs", &id_text, "cancel"],
            &["runtime:jobs:cancel"],
        ),
        _ => return Err(ApiError::invalid("invalid runtime job action")),
    };
    let body = (method == Method::POST).then(|| json!({}));
    let response = runtime_request(
        state,
        actor,
        project_id,
        method,
        &segments,
        body.as_ref(),
        None,
        scopes,
    )
    .await?;
    ensure_upstream_success(response.status, &response.body)?;
    Ok(response.body)
}

#[allow(clippy::too_many_arguments)]
async fn runtime_request(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    method: Method,
    segments: &[&str],
    body: Option<&Value>,
    idempotency_key: Option<&str>,
    scopes: &[&str],
) -> Result<JsonResponse, ApiError> {
    runtime_request_with_query(
        state,
        actor,
        project_id,
        method,
        segments,
        &[],
        body,
        idempotency_key,
        scopes,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn runtime_request_with_query(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    method: Method,
    segments: &[&str],
    query: &[(&str, String)],
    body: Option<&Value>,
    idempotency_key: Option<&str>,
    scopes: &[&str],
) -> Result<JsonResponse, ApiError> {
    let client = state.config.runtime_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "Shennong Runtime is not configured",
        )
    })?;
    let token = runtime_token(state, actor, project_id, scopes)?;
    client
        .request_json(
            method,
            segments,
            query,
            body,
            Some(("authorization", &format!("Bearer {token}"))),
            idempotency_key,
        )
        .await
        .map_err(map_upstream)
}

fn runtime_token(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    scopes: &[&str],
) -> Result<String, ApiError> {
    runtime_token_with_ttl(state, actor, project_id, scopes, 90)
}

fn runtime_token_with_ttl(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    scopes: &[&str],
    ttl_seconds: i64,
) -> Result<String, ApiError> {
    let signer = state.config.runtime_jwt_signer.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_auth_unavailable",
            "runtime signing key is not configured",
        )
    })?;
    let now = Utc::now().timestamp();
    let claims = RuntimeClaims {
        iss: state.config.runtime_jwt_issuer.clone(),
        aud: state.config.runtime_jwt_audience.clone(),
        sub: actor.id.to_string(),
        exp: now + ttl_seconds.clamp(10, 90),
        iat: now,
        jti: Uuid::new_v4().to_string(),
        scopes: scopes.iter().map(|value| (*value).to_owned()).collect(),
        workspace_refs: vec![workspace_ref(project_id)],
    };
    let (algorithm, key) = match signer {
        RuntimeJwtSigner::Ed25519(private_key) => (
            Algorithm::EdDSA,
            EncodingKey::from_ed_pem(private_key).map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "runtime_auth_failed",
                    "runtime signing key is invalid",
                )
            })?,
        ),
        RuntimeJwtSigner::Hs256(secret) => (Algorithm::HS256, EncodingKey::from_secret(secret)),
    };
    encode(&Header::new(algorithm), &claims, &key).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_auth_failed",
            "runtime authorization could not be issued",
        )
    })
}

async fn proxy_ide_request(
    state: AppState,
    session_id: Uuid,
    tail: Option<String>,
    uri: axum::http::Uri,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    let ide_origin = require_ide_host(&state, request.headers())?.clone();
    enforce_ide_request_origin(&ide_origin, request.method(), request.headers())?;
    let external_request_url = ide_external_request_url(&ide_origin, &uri)?;
    let access = authorize_ide_access(&state, session_id, request.headers()).await?;
    let runtime_token = runtime_token_with_ttl(
        &state,
        &access.actor,
        access.project_id,
        &["runtime:sessions:proxy"],
        30,
    )?;
    let client = state.config.runtime_client.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "Shennong Runtime is not configured",
        )
    })?;
    let mut owned_segments = vec![
        "v1".to_owned(),
        "sessions".to_owned(),
        session_id.to_string(),
        "proxy".to_owned(),
    ];
    if let Some(tail) = tail {
        owned_segments.extend(
            tail.split('/')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned),
        );
    }
    let segments = owned_segments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let target = client
        .streaming_url(&segments, uri.query())
        .map_err(map_upstream)?;
    let (mut parts, body) = request.into_parts();
    let browser_cookie = sanitized_ide_cookie(&parts.headers);
    let websocket = WebSocketUpgrade::from_request_parts(&mut parts, &state)
        .await
        .ok();
    if let Some(websocket) = websocket {
        return proxy_ide_websocket(
            websocket,
            target,
            &parts.headers,
            browser_cookie.as_deref(),
            &runtime_token,
            ide_origin.as_str().trim_end_matches('/'),
            &external_request_url,
        )
        .await;
    }
    let request = Request::from_parts(parts, body);
    proxy_ide_http(
        client,
        target,
        request,
        browser_cookie.as_deref(),
        &runtime_token,
        ide_origin.scheme() == "https",
        &external_request_url,
    )
    .await
}

fn ide_external_request_url(
    ide_origin: &url::Url,
    uri: &axum::http::Uri,
) -> Result<String, ApiError> {
    let path_and_query = uri
        .path_and_query()
        .map_or(uri.path(), axum::http::uri::PathAndQuery::as_str);
    let value = format!(
        "{}{}",
        ide_origin.as_str().trim_end_matches('/'),
        path_and_query
    );
    let parsed = url::Url::parse(&value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "ide_request_invalid",
            "IDE request URL could not be represented safely",
        )
    })?;
    if parsed.origin() != ide_origin.origin()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "ide_request_invalid",
            "IDE request URL could not be represented safely",
        ));
    }
    HeaderValue::from_str(&value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "ide_request_invalid",
            "IDE request URL could not be represented safely",
        )
    })?;
    Ok(value)
}

async fn authorize_ide_access(
    state: &AppState,
    session_id: Uuid,
    headers: &HeaderMap,
) -> Result<IdeAccess, ApiError> {
    let token = ide_access_token(headers).ok_or_else(ApiError::unauthorized)?;
    let row = sqlx::query(
        "SELECT access.id,access.user_id,runtime_session.project_id, \
                actor.email,actor.display_name,actor.role \
         FROM ide_access_sessions access \
         JOIN runtime_sessions runtime_session ON runtime_session.id=access.runtime_session_id \
         JOIN users actor ON actor.id=access.user_id \
         WHERE access.token_hash=$1 AND access.runtime_session_id=$2 \
           AND access.revoked_at IS NULL AND access.expires_at>NOW() \
           AND runtime_session.status='running' AND runtime_session.expires_at>NOW() \
           AND runtime_session.created_by_user_id=access.user_id AND actor.status='active'",
    )
    .bind(sha256(token))
    .bind(session_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::unauthorized)?;
    let access_id: Uuid = row.get("id");
    let actor = AuthUser::internal(
        row.get("user_id"),
        row.get("email"),
        row.get("display_name"),
        row.get("role"),
    );
    let project_id: Uuid = row.get("project_id");
    require_project_read(state, &actor, project_id).await?;
    sqlx::query(
        "UPDATE ide_access_sessions SET last_seen_at=NOW() \
         WHERE id=$1 AND last_seen_at<NOW()-INTERVAL '30 seconds'",
    )
    .bind(access_id)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(IdeAccess { actor, project_id })
}

async fn proxy_ide_http(
    client: &crate::clients::ServiceClient,
    target: reqwest::Url,
    request: Request<Body>,
    browser_cookie: Option<&str>,
    runtime_token: &str,
    secure_cookies: bool,
    external_request_url: &str,
) -> Result<Response, ApiError> {
    let method = request.method().clone();
    let headers = request.headers().clone();
    let body = reqwest::Body::wrap_stream(request.into_body().into_data_stream());
    let mut upstream = client
        .streaming_client()
        .request(method, target)
        .bearer_auth(runtime_token)
        .body(body);
    for (name, value) in &headers {
        if forward_ide_request_header(name) {
            upstream = upstream.header(name, value);
        }
    }
    upstream = upstream.header(RSTUDIO_REQUEST_HEADER, external_request_url);
    if let Some(cookie) = browser_cookie {
        upstream = upstream.header(COOKIE, cookie);
    }
    let upstream = upstream
        .send()
        .await
        .map_err(|error| map_upstream(UpstreamError::Request(error)))?;
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let mut builder = Response::builder().status(status);
    for (name, value) in &headers {
        if name == SET_COOKIE {
            if let Some(cookie) = sanitize_upstream_set_cookie(value, secure_cookies) {
                builder = builder.header(SET_COOKIE, cookie);
            }
        } else if forward_ide_response_header(name) {
            builder = builder.header(name, value);
        }
    }
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    builder.body(Body::from_stream(stream)).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ide_proxy_failed",
            "IDE proxy response could not be constructed",
        )
    })
}

async fn proxy_ide_websocket(
    websocket: WebSocketUpgrade,
    mut target: reqwest::Url,
    headers: &HeaderMap,
    browser_cookie: Option<&str>,
    runtime_token: &str,
    ide_origin: &str,
    external_request_url: &str,
) -> Result<Response, ApiError> {
    let scheme = if target.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    target.set_scheme(scheme).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ide_proxy_failed",
            "IDE WebSocket target could not be constructed",
        )
    })?;
    let mut outbound =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            target.as_str(),
        )
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "ide_proxy_failed",
                "IDE WebSocket request could not be constructed",
            )
        })?;
    outbound.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {runtime_token}"))
            .map_err(|_| ApiError::unauthorized())?,
    );
    outbound.headers_mut().insert(
        ORIGIN,
        HeaderValue::from_str(ide_origin).map_err(|_| ApiError::forbidden())?,
    );
    outbound.headers_mut().insert(
        HeaderName::from_static(RSTUDIO_REQUEST_HEADER),
        HeaderValue::from_str(external_request_url).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "ide_request_invalid",
                "IDE request URL could not be represented safely",
            )
        })?,
    );
    if let Some(cookie) = browser_cookie {
        outbound.headers_mut().insert(
            COOKIE,
            HeaderValue::from_str(cookie).map_err(|_| ApiError::unauthorized())?,
        );
    }
    if let Some(protocol) = headers.get("sec-websocket-protocol") {
        outbound
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.clone());
    }
    let (upstream, response) = connect_async(outbound).await.map_err(|error| {
        tracing::warn!(%error, "Runtime IDE WebSocket connection failed");
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ide_proxy_failed",
            "Runtime IDE WebSocket could not be reached",
        )
    })?;
    let selected_protocol = response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let websocket = if let Some(protocol) = selected_protocol {
        websocket.protocols([protocol])
    } else {
        websocket
    };
    Ok(websocket
        .on_upgrade(move |client| bridge_ide_websocket(client, upstream))
        .into_response())
}

async fn bridge_ide_websocket(
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    loop {
        tokio::select! {
            incoming = client.recv() => {
                let Some(incoming) = incoming else { break; };
                match incoming {
                    Ok(message) => {
                        if let Some(message) = axum_to_tungstenite(message)
                            && upstream.send(message).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "browser IDE WebSocket closed with error");
                        break;
                    }
                }
            }
            outgoing = upstream.next() => {
                let Some(outgoing) = outgoing else { break; };
                match outgoing {
                    Ok(message) => {
                        if let Some(message) = tungstenite_to_axum(message)
                            && client.send(message).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "Runtime IDE WebSocket closed with error");
                        break;
                    }
                }
            }
        }
    }
    let _ = upstream.close(None).await;
    let _ = client.close().await;
}

fn axum_to_tungstenite(message: AxumMessage) -> Option<tungstenite::Message> {
    match message {
        AxumMessage::Text(value) => Some(tungstenite::Message::Text(value.to_string().into())),
        AxumMessage::Binary(value) => Some(tungstenite::Message::Binary(value)),
        AxumMessage::Ping(value) => Some(tungstenite::Message::Ping(value)),
        AxumMessage::Pong(value) => Some(tungstenite::Message::Pong(value)),
        AxumMessage::Close(_) => Some(tungstenite::Message::Close(None)),
    }
}

fn tungstenite_to_axum(message: tungstenite::Message) -> Option<AxumMessage> {
    match message {
        tungstenite::Message::Text(value) => Some(AxumMessage::Text(value.to_string().into())),
        tungstenite::Message::Binary(value) => Some(AxumMessage::Binary(value)),
        tungstenite::Message::Ping(value) => Some(AxumMessage::Ping(value)),
        tungstenite::Message::Pong(value) => Some(AxumMessage::Pong(value)),
        tungstenite::Message::Close(_) => Some(AxumMessage::Close(None)),
        tungstenite::Message::Frame(_) => None,
    }
}

fn ide_access_token(headers: &HeaderMap) -> Option<&str> {
    let tokens = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| {
            cookie
                .trim()
                .strip_prefix("shennong_ide_session=")
                .filter(|value| (32..=256).contains(&value.len()))
        })
        .collect::<Vec<_>>();
    (tokens.len() == 1).then(|| tokens[0])
}

fn sanitized_ide_cookie(headers: &HeaderMap) -> Option<String> {
    let values = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .map(str::trim)
        .filter(|cookie| !cookie.is_empty())
        .filter(|cookie| {
            cookie
                .split_once('=')
                .map(|(name, _)| !name.to_ascii_lowercase().starts_with("shennong_"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join("; "))
}

fn forward_ide_request_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "authorization"
            | "connection"
            | "cookie"
            | "host"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-rstudio-request"
            | "x-rstudio-root-path"
    ) && !name.as_str().starts_with("x-forwarded-")
        && !name.as_str().starts_with("x-shennong-")
}

fn forward_ide_response_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn sanitize_upstream_set_cookie(value: &HeaderValue, secure: bool) -> Option<HeaderValue> {
    let raw = value.to_str().ok()?;
    let mut parts = raw.split(';');
    let cookie = parts.next()?.trim();
    let name = cookie.split_once('=')?.0.trim().to_ascii_lowercase();
    if name.starts_with("shennong_") {
        return None;
    }
    let mut output = vec![cookie.to_owned()];
    output.extend(parts.filter_map(|attribute| {
        let attribute = attribute.trim();
        let name = attribute
            .split_once('=')
            .map_or(attribute, |(name, _)| name)
            .trim();
        (!name.eq_ignore_ascii_case("domain")
            && !name.eq_ignore_ascii_case("samesite")
            && !name.eq_ignore_ascii_case("secure"))
        .then(|| attribute.to_owned())
    }));
    output.push("SameSite=Strict".into());
    if secure {
        output.push("Secure".into());
    }
    HeaderValue::from_str(&output.join("; ")).ok()
}

fn enforce_ide_request_origin(
    ide_origin: &url::Url,
    method: &Method,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected = ide_origin.as_str().trim_end_matches('/');
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if origin.is_some_and(|origin| origin.trim_end_matches('/') != expected) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "origin_denied",
            "IDE request origin is not allowed",
        ));
    }
    let is_websocket = headers
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if origin.is_none()
        && (is_websocket || !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS))
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "origin_required",
            "IDE mutations and WebSockets require an Origin header",
        ));
    }
    Ok(())
}

fn require_ide_host<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> Result<&'a url::Url, ApiError> {
    let origin = state
        .config
        .ide_public_origin
        .as_ref()
        .ok_or_else(ide_unavailable)?;
    if !request_targets_ide_host(state, headers) {
        return Err(ApiError::not_found());
    }
    Ok(origin)
}

pub(crate) fn request_targets_ide_host(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = state.config.ide_public_origin.as_ref() else {
        return false;
    };
    let Some(host) = effective_request_host(state, headers) else {
        return false;
    };
    host.parse::<axum::http::uri::Authority>()
        .ok()
        .zip(origin.host_str())
        .is_some_and(|(authority, configured)| {
            authority
                .host()
                .trim_end_matches('.')
                .eq_ignore_ascii_case(configured.trim_end_matches('.'))
        })
}

pub(crate) fn ide_host_path_allowed(path: &str) -> bool {
    if path == "/__shennong/launch" {
        return true;
    }
    let mut segments = path.trim_start_matches('/').split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next()
        ),
        (Some("v1"), Some("sessions"), Some(id), Some("proxy"))
            if id.parse::<Uuid>().is_ok()
    )
}

fn effective_request_host<'a>(state: &AppState, headers: &'a HeaderMap) -> Option<&'a str> {
    let header = if state.config.trust_proxy_headers {
        headers
            .get("x-forwarded-host")
            .or_else(|| headers.get(HOST))
    } else {
        headers.get(HOST)
    }?;
    header
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn ide_unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "ide_access_unavailable",
        "the isolated IDE public origin is not configured",
    )
}

fn workspace_ref(project_id: Uuid) -> String {
    format!("ws_{}", project_id.simple())
}

fn default_job_resources() -> Value {
    json!({
        "cpus":2.0,"memory_bytes":8589934592_i64,"pids":256,
        "timeout_seconds":1800,"tmpfs_bytes":1073741824_i64,
        "max_log_bytes":8388608_i64,"max_artifact_bytes":2147483648_i64,
        "max_workspace_bytes":21474836480_i64
    })
}

fn default_session_resources() -> Value {
    json!({
        "cpus":4.0,"memory_bytes":17179869184_i64,"pids":512,
        "timeout_seconds":28800,"tmpfs_bytes":2147483648_i64,
        "max_log_bytes":16777216_i64,"max_artifact_bytes":4294967296_i64,
        "max_workspace_bytes":53687091200_i64
    })
}

fn ensure_upstream_success(status: StatusCode, body: &Value) -> Result<(), ApiError> {
    if status.is_success() {
        return Ok(());
    }
    tracing::warn!(%status, "execution-plane request was rejected");
    let code = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "runtime_auth_rejected",
        StatusCode::NOT_FOUND => "runtime_resource_not_found",
        StatusCode::CONFLICT => "runtime_conflict",
        StatusCode::UNPROCESSABLE_ENTITY => "runtime_request_invalid",
        _ => "runtime_request_failed",
    };
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Shennong Runtime rejected the request");
    Err(ApiError::new(status, code, message))
}

fn map_upstream(error: UpstreamError) -> ApiError {
    tracing::error!(%error, "execution-plane request failed");
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "runtime_upstream_failed",
        "Shennong Runtime could not be reached safely",
    )
}

fn upstream_invalid(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "runtime_response_invalid", message)
}

fn runtime_status(value: &Value, fallback: &str) -> String {
    value
        .get("state")
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_owned()
}

async fn persist_runtime_job_view(
    state: &AppState,
    id: Uuid,
    value: &Value,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE jobs SET status=$2,result=$3,updated_at=NOW() WHERE id=$1")
        .bind(id)
        .bind(runtime_status(value, "failed"))
        .bind(value)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(())
}

async fn session_project(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar("SELECT project_id FROM runtime_sessions WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)
}

async fn persist_runtime_session_view(
    state: &AppState,
    id: Uuid,
    value: &Value,
) -> Result<(), ApiError> {
    let expires = value
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<chrono::DateTime<Utc>>().ok());
    sqlx::query(
        "UPDATE runtime_sessions SET status=$2,runtime_view=$3,expires_at=COALESCE($4,expires_at),updated_at=NOW() WHERE id=$1",
    )
    .bind(id)
    .bind(runtime_status(value, "failed"))
    .bind(value)
    .bind(expires)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

fn job_view(row: sqlx::postgres::PgRow) -> Value {
    let result: Value = row.get("result");
    json!({
        "id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),
        "run_id":row.get::<Option<Uuid>,_>("run_id"),"status":row.get::<String,_>("status"),
        "worker_profile":result.get("worker_profile").and_then(Value::as_str).unwrap_or("cpu-small"),
        "created_at":row.get::<chrono::DateTime<Utc>,_>("created_at"),
        "started_at":result.get("started_at"),"finished_at":result.get("finished_at"),
        "exit_code":result.get("exit_code")
    })
}

fn session_view(row: sqlx::postgres::PgRow) -> Value {
    json!({
        "id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),
        "kind":row.get::<String,_>("kind"),"status":row.get::<String,_>("status"),
        "worker_profile":row.get::<String,_>("worker_profile"),
        "created_at":row.get::<chrono::DateTime<Utc>,_>("created_at"),
        "expires_at":row.get::<Option<chrono::DateTime<Utc>>,_>("expires_at")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_resource_contract_includes_workspace_hard_limits() {
        let batch = default_job_resources();
        let session = default_session_resources();
        assert_eq!(batch.as_object().map(|value| value.len()), Some(8));
        assert_eq!(session.as_object().map(|value| value.len()), Some(8));
        assert_eq!(
            batch.get("max_workspace_bytes").and_then(Value::as_i64),
            Some(21_474_836_480)
        );
        assert_eq!(
            session.get("max_workspace_bytes").and_then(Value::as_i64),
            Some(53_687_091_200)
        );
    }

    #[test]
    fn ide_cookie_is_unique_and_never_forwarded_to_runtime() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            "shennong_os_session=os; shennong_ide_session=abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG; _xsrf=ide"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            ide_access_token(&headers),
            Some("abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG")
        );
        assert_eq!(sanitized_ide_cookie(&headers).as_deref(), Some("_xsrf=ide"));
        headers.append(
            COOKIE,
            "shennong_ide_session=duplicate-abcdefghijklmnopqrstuvwxyz0123456789"
                .parse()
                .unwrap(),
        );
        assert!(ide_access_token(&headers).is_none());
    }

    #[test]
    fn upstream_ide_cookie_is_forced_host_only_and_strict() {
        let raw = HeaderValue::from_static(
            "rstudio=abc; Domain=.example.test; Path=/; SameSite=None; Secure; HttpOnly",
        );
        let value = sanitize_upstream_set_cookie(&raw, true)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(!value.to_ascii_lowercase().contains("domain="));
        assert!(value.contains("SameSite=Strict"));
        assert!(value.contains("Secure"));
        assert!(value.contains("HttpOnly"));
        assert!(
            sanitize_upstream_set_cookie(
                &HeaderValue::from_static("shennong_os_session=attack"),
                true
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn ide_launch_interstitial_preserves_strict_cookie_without_reflecting_ticket() {
        let id = Uuid::parse_str("00000000-0000-4000-8000-000000000123").unwrap();
        let nonce = "fixed_urlsafe_nonce_123456";
        let access_token = "fixture-access-token-that-is-not-rendered";
        let response =
            ide_launch_interstitial_response(id, access_token, 3600, true, nonce).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let cookie = headers
            .get(SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap();
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("; Secure"));
        assert!(cookie.contains(&format!("Path=/v1/sessions/{id}/proxy")));
        let csp = headers
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .unwrap();
        assert!(csp.contains(&format!("script-src 'nonce-{nonce}'")));
        assert!(csp.contains("default-src 'none'"));
        assert!(!csp.contains("'unsafe-inline'"));
        assert_eq!(
            headers
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            headers
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            headers
                .get("referrer-policy")
                .and_then(|value| value.to_str().ok()),
            Some("no-referrer")
        );
        assert_eq!(
            headers
                .get("cross-origin-opener-policy")
                .and_then(|value| value.to_str().ok()),
            Some("same-origin")
        );
        assert_eq!(
            headers
                .get("cross-origin-resource-policy")
                .and_then(|value| value.to_str().ok()),
            Some("same-origin")
        );
        assert!(headers.get("location").is_none());
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        let target = format!("/v1/sessions/{id}/proxy/");
        assert!(body.contains(&format!(
            "window.location.replace({})",
            serde_json::to_string(&target).unwrap()
        )));
        assert!(body.contains(&format!(
            "http-equiv=\"refresh\" content=\"0;url={target}\""
        )));
        assert!(body.contains(&format!("href=\"{target}\"")));
        assert!(body.contains(&format!("nonce=\"{nonce}\"")));
        assert!(!body.contains("ticket"));
        assert!(!body.contains(access_token));
        assert!(!body.contains("://"));
    }

    #[test]
    fn ide_launch_interstitial_rejects_an_injectable_nonce() {
        assert!(
            ide_launch_interstitial_response(
                Uuid::nil(),
                "fixture-access-token",
                60,
                false,
                "bad-nonce\"<script>"
            )
            .is_err()
        );
    }

    #[test]
    fn ide_host_only_allows_ticket_and_session_proxy_paths() {
        let id = Uuid::new_v4();
        assert!(ide_host_path_allowed("/__shennong/launch"));
        assert!(ide_host_path_allowed(&format!(
            "/v1/sessions/{id}/proxy/api/status"
        )));
        assert!(!ide_host_path_allowed("/api/v1/auth/session"));
        assert!(!ide_host_path_allowed(&format!(
            "/v1/sessions/{id}/not-proxy"
        )));
    }

    #[test]
    fn ide_mutations_and_websockets_require_exact_origin() {
        let ide = url::Url::parse("https://ide.example.test").unwrap();
        let mut headers = HeaderMap::new();
        assert!(enforce_ide_request_origin(&ide, &Method::GET, &headers).is_ok());
        assert!(enforce_ide_request_origin(&ide, &Method::POST, &headers).is_err());
        headers.insert(ORIGIN, "https://os.example.test".parse().unwrap());
        assert!(enforce_ide_request_origin(&ide, &Method::POST, &headers).is_err());
        headers.insert(ORIGIN, "https://ide.example.test".parse().unwrap());
        assert!(enforce_ide_request_origin(&ide, &Method::POST, &headers).is_ok());
        headers.remove(ORIGIN);
        headers.insert("upgrade", "websocket".parse().unwrap());
        assert!(enforce_ide_request_origin(&ide, &Method::GET, &headers).is_err());
    }

    #[test]
    fn ide_external_request_url_preserves_the_exact_path_and_query() {
        let ide = url::Url::parse("https://ide.example.test:8443/").unwrap();
        let uri = "/v1/sessions/00000000-0000-4000-8000-000000000000/proxy/folder%20name/file%2Fpart?x=a%2Fb&empty="
            .parse()
            .unwrap();
        assert_eq!(
            ide_external_request_url(&ide, &uri).unwrap(),
            "https://ide.example.test:8443/v1/sessions/00000000-0000-4000-8000-000000000000/proxy/folder%20name/file%2Fpart?x=a%2Fb&empty="
        );
    }

    #[test]
    fn browser_cannot_supply_rstudio_or_private_proxy_headers() {
        for name in [
            "x-rstudio-request",
            "x-rstudio-root-path",
            RSTUDIO_REQUEST_HEADER,
        ] {
            assert!(!forward_ide_request_header(&HeaderName::from_static(name)));
        }
        assert!(forward_ide_request_header(&HeaderName::from_static(
            "accept-language"
        )));
    }
}
