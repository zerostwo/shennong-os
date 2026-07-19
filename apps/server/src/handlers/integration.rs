use super::projects::require_project_write;
use super::runtime_control::{agent_job_action, agent_job_artifacts, submit_agent_job};
use super::{Envelope, audit};
use crate::{
    AppState,
    auth::{AuthUser, authenticate, require_agent_runtime},
    crypto::{decrypt_secret, random_secret, sha256, sha256_hex, tool_arguments_digest},
    error::ApiError,
};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use chrono::{Duration, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgUiInput {
    thread_id: String,
    run_id: String,
    parent_run_id: Option<String>,
    resume: Option<Vec<AgUiResumeEntry>>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgUiResumeEntry {
    interrupt_id: String,
    status: String,
    payload: Option<Value>,
}

#[derive(Deserialize)]
pub struct InternalRunIdentity {
    thread_id: Uuid,
    run_id: Uuid,
    parent_run_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRequest {
    run_id: Uuid,
    user_id: Uuid,
    project_id: Option<Uuid>,
    tool_call_id: String,
    tool_name: String,
    arguments_digest: String,
    risk: String,
    run_capability_token: String,
    arguments: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionRequest {
    run_id: Uuid,
    user_id: Uuid,
    project_id: Option<Uuid>,
    tool_call_id: String,
    tool_name: String,
    arguments_digest: String,
    risk: String,
    run_capability_token: String,
    arguments: Value,
    execution_token: String,
}

#[derive(Clone, Copy)]
struct ToolDefinition {
    name: &'static str,
    risk: &'static str,
    project_required: bool,
}

const TOOLS: &[ToolDefinition] = &[
    tool("skill.load", "read", false),
    tool("plan.propose", "write", true),
    tool("plan.update", "write", true),
    tool("db.discover_resources", "read", false),
    tool("db.inspect_resource", "read", false),
    tool("db.query_resource", "read", false),
    tool("db.get_provenance", "read", false),
    tool("project.list_files", "read", true),
    tool("project.read_file", "read", true),
    tool("project.write_file", "write", true),
    tool("environment.plan", "read", true),
    tool("runtime.submit_job", "compute", true),
    tool("runtime.get_job", "read", true),
    tool("runtime.cancel_job", "destructive", true),
    tool("artifact.register", "write", true),
    tool("analysis.validate", "read", false),
];

const fn tool(name: &'static str, risk: &'static str, project_required: bool) -> ToolDefinition {
    ToolDefinition {
        name,
        risk,
        project_required,
    }
}

pub async fn public_config(
    State(state): State<AppState>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let mode: String =
        sqlx::query_scalar("SELECT registration_mode FROM os_settings WHERE singleton=TRUE")
            .fetch_one(&state.pool)
            .await
            .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: json!({
            "registration_mode":mode,
            "registration_enabled":mode != "disabled",
            "invite_required":mode == "invite_only",
            "public_origin":state.config.public_origin.as_str().trim_end_matches('/'),
            "ide_public_origin":state.config.ide_public_origin.as_ref().map(|origin| origin.as_str().trim_end_matches('/')),
            "api_version":"v1",
            "version":env!("CARGO_PKG_VERSION")
        }),
    }))
}

pub async fn capabilities(State(state): State<AppState>) -> Json<Envelope<Value>> {
    let ide_access = state.config.ide_public_origin.is_some()
        && state.config.runtime_client.is_some()
        && state.config.runtime_jwt_signer.is_some();
    let agent_gateway = if let Some(client) = state.config.agent_runtime_client.as_ref() {
        client.healthy().await
    } else {
        false
    };
    Json(Envelope {
        data: json!({
            "agent_gateway":agent_gateway,"thread_storage":true,"run_events":true,
            "task_plans":true,"skills":true,"memory":true,
            "runtime_jobs":state.config.runtime_client.is_some(),
            "runtime_sessions":state.config.runtime_client.is_some(),
            "isolated_ide_launch":ide_access,
            "data_plane_proxy":true,"api_version":"v1"
        }),
    })
}

pub async fn agent_gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let mut forwarded: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::invalid("invalid AG-UI RunAgentInput"))?;
    let identity: AgUiInput = serde_json::from_slice(&body)
        .map_err(|_| ApiError::invalid("invalid AG-UI RunAgentInput"))?;
    let thread_id = parse_runtime_uuid(&identity.thread_id, "threadId")?;
    let run_id = parse_runtime_uuid(&identity.run_id, "runId")?;
    let requested_project_id = headers
        .get("x-shennong-project-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<Uuid>().ok())
                .ok_or_else(|| ApiError::invalid("x-shennong-project-id must be a UUID"))
        })
        .transpose()?;
    let requested_provider_id = headers
        .get("x-shennong-provider-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<Uuid>().ok())
                .ok_or_else(|| ApiError::invalid("x-shennong-provider-id must be a UUID"))
        })
        .transpose()?;
    let thinking_level = headers
        .get("x-shennong-thinking-level")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("medium");
    if !matches!(
        thinking_level,
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
    ) {
        return Err(ApiError::invalid("x-shennong-thinking-level is invalid"));
    }
    if let Some(provider_id) = requested_provider_id {
        let available = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM model_providers WHERE id=$1 AND owner_user_id=$2 AND enabled)",
        )
        .bind(provider_id)
        .bind(actor.id)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
        if !available {
            return Err(ApiError::invalid("model provider is unavailable"));
        }
    }
    let supplied_parent_run_id = identity
        .parent_run_id
        .as_deref()
        .map(|value| parse_runtime_uuid(value, "parentRunId"))
        .transpose()?;
    if let Some(project_id) = requested_project_id {
        require_project_write(&state, &actor, project_id).await?;
        let project_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1 AND status='active')",
        )
        .bind(project_id)
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
        if !project_exists {
            return Err(ApiError::not_found());
        }
    }
    let existing =
        sqlx::query("SELECT project_id,owner_user_id FROM threads WHERE id=$1 AND status='active'")
            .bind(thread_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::database)?;
    let created = if let Some(existing) = existing {
        if existing.get::<Option<Uuid>, _>("project_id") != requested_project_id
            || (requested_project_id.is_none()
                && existing.get::<Uuid, _>("owner_user_id") != actor.id)
        {
            return Err(ApiError::not_found());
        }
        false
    } else {
        let inserted = sqlx::query(
            "INSERT INTO threads(id,project_id,owner_user_id,provider_id,title,status) \
             VALUES($1,$2,$3,$4,'New chat','active') ON CONFLICT(id) DO NOTHING",
        )
        .bind(thread_id)
        .bind(requested_project_id)
        .bind(actor.id)
        .bind(requested_provider_id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?
        .rows_affected();
        let actual = sqlx::query(
            "SELECT project_id,owner_user_id FROM threads WHERE id=$1 AND status='active'",
        )
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
        if actual.get::<Option<Uuid>, _>("project_id") != requested_project_id
            || (requested_project_id.is_none()
                && actual.get::<Uuid, _>("owner_user_id") != actor.id)
        {
            return Err(ApiError::not_found());
        }
        inserted == 1
    };
    let project_id = requested_project_id;
    if created {
        super::context::enable_default_thread_skills(&state, thread_id, project_id.is_some())
            .await?;
        audit(
            &state,
            Some(&actor),
            project_id,
            "thread.create_on_first_run",
            "thread",
            Some(thread_id.to_string()),
            json!({"source":"agent_gateway"}),
        )
        .await?;
    } else if let Some(provider_id) = requested_provider_id {
        sqlx::query("UPDATE threads SET provider_id=$2,updated_at=NOW() WHERE id=$1")
            .bind(thread_id)
            .bind(provider_id)
            .execute(&state.pool)
            .await
            .map_err(ApiError::database)?;
    }
    let parent_run_id = if let Some(resume) = identity.resume.as_deref() {
        if created {
            return Err(ApiError::forbidden());
        }
        let parent_run_id = resolve_agent_resume(
            &state,
            &actor,
            project_id,
            thread_id,
            run_id,
            supplied_parent_run_id,
            resume,
        )
        .await?;
        let forwarded = forwarded
            .as_object_mut()
            .ok_or_else(|| ApiError::invalid("invalid AG-UI RunAgentInput"))?;
        forwarded.insert("parentRunId".into(), json!(parent_run_id));
        parent_run_id.into()
    } else {
        let inserted = sqlx::query(
            "INSERT INTO runs(id,project_id,thread_id,parent_run_id,requested_by_user_id,status,input) \
             VALUES($1,$2,$3,$4,$5,'queued',$6) ON CONFLICT(id) DO NOTHING",
        )
        .bind(run_id)
        .bind(project_id)
        .bind(thread_id)
        .bind(supplied_parent_run_id)
        .bind(actor.id)
        .bind(json!({"source":"agent_gateway","thinking_level":thinking_level}))
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?
        .rows_affected();
        if inserted != 1 {
            return Err(ApiError::conflict("run id has already been used"));
        }
        supplied_parent_run_id
    };
    audit(
        &state,
        Some(&actor),
        project_id,
        "agent.run_requested",
        "run",
        Some(run_id.to_string()),
        json!({"thread_id":thread_id,"parent_run_id":parent_run_id}),
    )
    .await?;
    let forwarded_body = Bytes::from(
        serde_json::to_vec(&forwarded)
            .map_err(|_| ApiError::invalid("invalid AG-UI RunAgentInput"))?,
    );
    let Some(client) = state.config.agent_runtime_client.as_ref() else {
        sqlx::query(
            "UPDATE runs SET status='failed',error=$2,finished_at=NOW(),updated_at=NOW() \
             WHERE id=$1 AND status='queued'",
        )
        .bind(run_id)
        .bind(json!({"code":"agent_runtime_unavailable"}))
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_runtime_unavailable",
            "Agent Runtime is not configured",
        ));
    };
    let upstream = match client.run(forwarded_body).await {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, %run_id, "agent gateway upstream failed");
            sqlx::query("UPDATE runs SET status='failed',error=$2,finished_at=NOW(),updated_at=NOW() WHERE id=$1")
                .bind(run_id).bind(json!({"code":"agent_runtime_unavailable"}))
                .execute(&state.pool).await.map_err(ApiError::database)?;
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "agent_runtime_unavailable",
                "Agent Runtime could not be reached",
            ));
        }
    };
    let status = upstream.status();
    let content_type = upstream
        .headers()
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("text/event-stream; charset=utf-8"));
    if !status.is_success() {
        sqlx::query("UPDATE runs SET status='failed',error=$2,finished_at=NOW(),updated_at=NOW() WHERE id=$1")
            .bind(run_id).bind(json!({"code":"agent_runtime_rejected","status":status.as_u16()}))
            .execute(&state.pool).await.map_err(ApiError::database)?;
    }
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("cache-control", "no-cache, no-store, no-transform")
        .header("x-accel-buffering", "no")
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(stream))
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent_gateway_failed",
                "Agent stream could not be constructed",
            )
        })
}

async fn resolve_agent_resume(
    state: &AppState,
    actor: &AuthUser,
    project_id: Option<Uuid>,
    thread_id: Uuid,
    resumed_run_id: Uuid,
    supplied_parent_run_id: Option<Uuid>,
    resume: &[AgUiResumeEntry],
) -> Result<Uuid, ApiError> {
    let [entry] = resume else {
        return Err(ApiError::invalid(
            "V1 supports exactly one AG-UI interrupt response per continuation",
        ));
    };
    let approval_id = parse_runtime_uuid(&entry.interrupt_id, "interruptId")?;
    let approved = match entry.status.as_str() {
        "resolved" => {
            let payload = entry
                .payload
                .as_ref()
                .and_then(Value::as_object)
                .ok_or_else(|| ApiError::invalid("resolved approval payload must be an object"))?;
            if payload.len() != 1 || payload.get("approved").and_then(Value::as_bool) != Some(true)
            {
                return Err(ApiError::invalid(
                    "V1 approvals only accept the exact {approved:true} payload",
                ));
            }
            true
        }
        "cancelled" => {
            if entry
                .payload
                .as_ref()
                .is_some_and(|payload| !payload.is_null())
            {
                return Err(ApiError::invalid(
                    "cancelled approval responses cannot include a payload",
                ));
            }
            false
        }
        _ => return Err(ApiError::invalid("invalid AG-UI interrupt response status")),
    };

    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let approval = sqlx::query(
        "SELECT a.run_id,a.status,a.expires_at,a.requested_by_user_id,a.resumed_run_id, \
                r.project_id,r.thread_id,r.status AS run_status \
         FROM run_approvals a JOIN runs r ON r.id=a.run_id \
         WHERE a.id=$1 FOR UPDATE OF a,r",
    )
    .bind(approval_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::forbidden)?;
    let original_run_id: Uuid = approval.get("run_id");
    if approval.get::<Uuid, _>("requested_by_user_id") != actor.id
        || approval.get::<Option<Uuid>, _>("project_id") != project_id
        || approval.get::<Uuid, _>("thread_id") != thread_id
        || supplied_parent_run_id.is_some_and(|id| id != original_run_id)
    {
        return Err(ApiError::forbidden());
    }
    if approval.get::<String, _>("status") != "pending"
        || approval.get::<String, _>("run_status") != "waiting_approval"
        || approval.get::<Option<Uuid>, _>("resumed_run_id").is_some()
    {
        return Err(ApiError::conflict("approval is no longer pending"));
    }
    if approval.get::<chrono::DateTime<Utc>, _>("expires_at") <= Utc::now() {
        sqlx::query("UPDATE run_approvals SET status='expired',updated_at=NOW() WHERE id=$1;")
            .bind(approval_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::database)?;
        sqlx::query(
            "UPDATE runs SET status='cancelled',error=$2,finished_at=NOW(),updated_at=NOW() \
             WHERE id=$1 AND status='waiting_approval'",
        )
        .bind(original_run_id)
        .bind(json!({"code":"approval_expired"}))
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        super::audit_tx(
            &mut tx,
            Some(actor.id),
            project_id,
            "agent.approval_expired",
            "run_approval",
            Some(approval_id.to_string()),
            json!({"run_id":original_run_id}),
            None,
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        return Err(ApiError::conflict("approval has expired"));
    }

    let response_payload = entry
        .payload
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let resume_status = if approved { "resolved" } else { "cancelled" };
    let inserted = sqlx::query(
        "INSERT INTO runs(id,project_id,thread_id,parent_run_id,requested_by_user_id,status,input) \
         VALUES($1,$2,$3,$4,$5,'queued',$6) ON CONFLICT(id) DO NOTHING",
    )
    .bind(resumed_run_id)
    .bind(project_id)
    .bind(thread_id)
    .bind(original_run_id)
    .bind(actor.id)
    .bind(json!({
        "source":"agent_gateway_resume",
        "resume_approval_id":approval_id,
        "resume_status":resume_status
    }))
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .rows_affected();
    if inserted != 1 {
        return Err(ApiError::conflict("run id has already been used"));
    }
    let approval_status = if approved { "approved" } else { "rejected" };
    sqlx::query(
        "UPDATE run_approvals SET status=$2,response_payload=$3,decided_by_user_id=$4, \
         decided_at=NOW(),resumed_run_id=$5,updated_at=NOW() WHERE id=$1",
    )
    .bind(approval_id)
    .bind(approval_status)
    .bind(response_payload)
    .bind(actor.id)
    .bind(resumed_run_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let original_status = if approved { "succeeded" } else { "cancelled" };
    sqlx::query(
        "UPDATE runs SET status=$2,output=$3,capability_token_hash=NULL,capability_expires_at=NULL, \
         finished_at=NOW(),updated_at=NOW() WHERE id=$1 AND status='waiting_approval'",
    )
    .bind(original_run_id)
    .bind(original_status)
    .bind(json!({"stopReason":if approved {"interrupted"} else {"approval_rejected"},"resumedRunId":resumed_run_id}))
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    super::audit_tx(
        &mut tx,
        Some(actor.id),
        project_id,
        if approved {
            "agent.approval_approved"
        } else {
            "agent.approval_rejected"
        },
        "run_approval",
        Some(approval_id.to_string()),
        json!({"run_id":original_run_id,"resumed_run_id":resumed_run_id}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(original_run_id)
}

const BOOTSTRAP_RUN_QUERY: &str = "SELECT r.id,r.project_id,r.thread_id,r.parent_run_id,r.requested_by_user_id,r.status,r.input, \
            u.email,u.display_name,u.role AS user_role,t.provider_id, \
            p.name AS project_name,p.description AS project_description, \
            COALESCE(pm.role,'') AS project_role \
     FROM runs r JOIN users u ON u.id=r.requested_by_user_id \
     JOIN threads t ON t.id=r.thread_id LEFT JOIN projects p ON p.id=r.project_id \
     LEFT JOIN project_members pm ON pm.project_id=r.project_id AND pm.user_id=r.requested_by_user_id \
     WHERE r.id=$1 AND u.status='active' AND (r.project_id IS NULL OR p.status='active') \
       AND (r.project_id IS NOT NULL OR t.owner_user_id=r.requested_by_user_id)";

pub async fn bootstrap_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<InternalRunIdentity>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_agent_runtime(&state, &headers)?;
    let row = sqlx::query(BOOTSTRAP_RUN_QUERY)
        .bind(value.run_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    if row.get::<Uuid, _>("thread_id") != value.thread_id
        || row.get::<Option<Uuid>, _>("parent_run_id") != value.parent_run_id
        || !matches!(
            row.get::<String, _>("status").as_str(),
            "queued" | "running"
        )
    {
        return Err(ApiError::conflict(
            "run bootstrap identity or state mismatch",
        ));
    }
    let user_id: Uuid = row.get("requested_by_user_id");
    let project_id: Option<Uuid> = row.get("project_id");
    let user_role: String = row.get("user_role");
    let project_role: String = row.get("project_role");
    if project_id.is_some() && user_role != "admin" && project_role.is_empty() {
        return Err(ApiError::not_found());
    }
    let provider = load_provider(&state, user_id, row.get("provider_id")).await?;
    let skills = load_run_skills(&state, value.thread_id, user_id).await?;
    let allowed_tools = allowed_tools(&user_role, &project_role, project_id.is_some(), &skills);
    let allowed_project_read = declared_string_permissions(&skills, "projectRead");
    let allowed_project_write = declared_string_permissions(&skills, "projectWrite");
    let allowed_compute_profiles = declared_string_permissions(&skills, "computeProfiles");
    let required_approvals = declared_string_permissions(&skills, "approvals");
    let capability = random_secret(32);
    let expires_at = Utc::now() + Duration::minutes(15);
    sqlx::query(
        "UPDATE runs SET status='running',capability_token_hash=$2,capability_expires_at=$3, \
         input=input || jsonb_build_object( \
           'allowed_tools',$4::jsonb,'tool_profile',$5::jsonb, \
           'allowed_project_read',$6::jsonb,'allowed_project_write',$7::jsonb, \
           'allowed_compute_profiles',$8::jsonb,'required_approvals',$9::jsonb), \
         started_at=COALESCE(started_at,NOW()),updated_at=NOW() WHERE id=$1",
    )
    .bind(value.run_id)
    .bind(sha256(&capability))
    .bind(expires_at)
    .bind(json!(allowed_tools))
    .bind(json!(tool_profile(
        &project_role,
        &user_role,
        project_id.is_some()
    )))
    .bind(json!(allowed_project_read))
    .bind(json!(allowed_project_write))
    .bind(json!(allowed_compute_profiles))
    .bind(json!(required_approvals))
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let resume_approval = prepare_resume_approval(
        &state,
        value.run_id,
        value.parent_run_id,
        user_id,
        row.get("input"),
        &allowed_tools,
    )
    .await?;
    let messages = load_messages(&state, value.thread_id).await?;
    let memories = load_memories(&state, user_id, project_id).await?;
    let artifacts = if let Some(project_id) = project_id {
        load_artifacts(&state, project_id).await?
    } else {
        Vec::new()
    };
    let mut scope = json!({
        "userId":user_id,"threadId":value.thread_id,
        "role":if user_role == "admin" {"admin"} else {"user"},
        "providerDataPolicy":provider["dataPolicy"]
    });
    if let Some(project_id) = project_id {
        scope["projectId"] = json!(project_id);
    }
    let project = project_id.map(|project_id| {
        json!({
            "id":project_id,
            "name":row.get::<Option<String>,_>("project_name").unwrap_or_default(),
            "description":row.get::<Option<String>,_>("project_description").unwrap_or_default()
        })
    });
    Ok(Json(Envelope {
        data: json!({
            "runId":value.run_id,
            "parentRunId":value.parent_run_id,
            "scope":scope,
            "runCapabilityToken":capability,
            "provider":provider,
            "messages":messages,
            "context":{
                "project":project,
                "memories":memories,"artifacts":artifacts,"selectedSkills":skills
            },
            "toolProfile":tool_profile(&project_role, &user_role, project_id.is_some()),
            "thinkingLevel":row.get::<Value,_>("input").get("thinking_level").and_then(Value::as_str).unwrap_or("medium"),"timeoutMs":600000,
            "resumeApproval":resume_approval
        }),
    }))
}

async fn prepare_resume_approval(
    state: &AppState,
    resumed_run_id: Uuid,
    parent_run_id: Option<Uuid>,
    user_id: Uuid,
    run_input: Value,
    allowed_tools: &[&str],
) -> Result<Option<Value>, ApiError> {
    let Some(raw_approval_id) = run_input.get("resume_approval_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let approval_id = raw_approval_id
        .parse::<Uuid>()
        .map_err(|_| ApiError::forbidden())?;
    let parent_run_id = parent_run_id.ok_or_else(ApiError::forbidden)?;
    let row = sqlx::query(
        "SELECT id,run_id,status,tool_call_id,tool_name,arguments_digest,arguments,risk, \
                approval_scope,expires_at \
         FROM run_approvals WHERE id=$1 AND run_id=$2 AND resumed_run_id=$3 \
         AND requested_by_user_id=$4",
    )
    .bind(approval_id)
    .bind(parent_run_id)
    .bind(resumed_run_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::forbidden)?;
    let status: String = row.get("status");
    let expected_resume_status = run_input
        .get("resume_status")
        .and_then(Value::as_str)
        .ok_or_else(ApiError::forbidden)?;
    if (status == "approved" && expected_resume_status != "resolved")
        || (status == "rejected" && expected_resume_status != "cancelled")
    {
        return Err(ApiError::forbidden());
    }
    let base = json!({
        "originalRunId":parent_run_id,
        "interruptId":approval_id,
        "status":expected_resume_status,
        "toolCallId":row.get::<String,_>("tool_call_id"),
        "toolName":row.get::<String,_>("tool_name"),
        "argumentsDigest":row.get::<String,_>("arguments_digest"),
        "risk":row.get::<String,_>("risk"),
        "approvalScope":row.get::<String,_>("approval_scope"),
        "expiresAt":row.get::<chrono::DateTime<Utc>,_>("expires_at")
    });
    if status == "rejected" {
        return Ok(Some(base));
    }
    if status != "approved" {
        return Err(ApiError::forbidden());
    }
    let tool_name: String = row.get("tool_name");
    if !allowed_tools.iter().any(|allowed| *allowed == tool_name) {
        return Err(ApiError::forbidden());
    }
    let arguments: Value = row.get("arguments");
    let digest: String = row.get("arguments_digest");
    if tool_arguments_digest(&tool_name, &arguments) != digest {
        return Err(ApiError::forbidden());
    }
    let execution_token = random_secret(32);
    let grant_id = Uuid::new_v4();
    let grant = sqlx::query(
        "INSERT INTO run_tool_grants(id,run_id,tool_call_id,tool_name,arguments_digest,risk,execution_token_hash,decision,expires_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,'allowed',NOW()+INTERVAL '2 minutes') \
         ON CONFLICT(run_id,tool_call_id) DO UPDATE SET execution_token_hash=EXCLUDED.execution_token_hash, \
         expires_at=EXCLUDED.expires_at WHERE run_tool_grants.used_at IS NULL \
         AND run_tool_grants.tool_name=EXCLUDED.tool_name \
         AND run_tool_grants.arguments_digest=EXCLUDED.arguments_digest \
         AND run_tool_grants.risk=EXCLUDED.risk RETURNING id",
    )
    .bind(grant_id)
    .bind(resumed_run_id)
    .bind(row.get::<String, _>("tool_call_id"))
    .bind(&tool_name)
    .bind(&digest)
    .bind(row.get::<String, _>("risk"))
    .bind(sha256(&execution_token))
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if grant.is_none() {
        return Err(ApiError::conflict(
            "approved operation has already been consumed",
        ));
    }
    let mut resumed = base;
    let resumed = resumed
        .as_object_mut()
        .expect("resume approval payload is an object");
    resumed.insert("arguments".into(), arguments);
    resumed.insert("executionToken".into(), json!(execution_token));
    Ok(Some(Value::Object(resumed.clone())))
}

pub async fn record_run_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<Value>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_agent_runtime(&state, &headers)?;
    validate_callback_run_id(id, &value)?;
    let updated = sqlx::query("UPDATE runs SET metadata=$2,updated_at=NOW() WHERE id=$1")
        .bind(id)
        .bind(&value)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?
        .rows_affected();
    if updated != 1 {
        return Err(ApiError::not_found());
    }
    Ok(Json(Envelope { data: json!({}) }))
}

pub async fn append_run_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<Value>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    require_agent_runtime(&state, &headers)?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| ApiError::invalid("event type is required"))?;
    let row = sqlx::query(
        "INSERT INTO run_events(run_id,event_type,payload) VALUES($1,$2,$3) RETURNING id,created_at",
    )
    .bind(id)
    .bind(event_type)
    .bind(&value)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let cursor: i64 = row.get("id");
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"cursor":cursor.to_string()}),
        }),
    ))
}

pub async fn finish_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<Value>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_agent_runtime(&state, &headers)?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("finish status is required"))?;
    if !matches!(
        status,
        "succeeded" | "failed" | "failed_validation" | "cancelled"
    ) {
        return Err(ApiError::invalid("invalid finish status"));
    }
    let result = value
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let error = value
        .get("error")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let run = sqlx::query(
        "UPDATE runs SET status=$2,output=$3,error=$4,capability_token_hash=NULL, \
         capability_expires_at=NULL,finished_at=NOW(),updated_at=NOW() \
         WHERE id=$1 AND (status='running' OR (status='queued' AND $2='failed')) \
         RETURNING thread_id,project_id,requested_by_user_id",
    )
    .bind(id)
    .bind(status)
    .bind(&result)
    .bind(&error)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    if status == "succeeded"
        && let Some(content) = result.get("content").and_then(Value::as_str)
    {
        let message_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO messages(id,thread_id,role,content_json,status,attachments,metadata,idempotency_key) \
             VALUES($1,$2,'assistant',$3,'completed','[]'::jsonb,$4,$5) \
             ON CONFLICT(thread_id,idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
        )
        .bind(message_id)
        .bind(run.get::<Uuid, _>("thread_id"))
        .bind(json!(content))
        .bind(json!({"run_id":id,"evidence":result.get("evidence"),"validation_reports":result.get("validationReports")}))
        .bind(format!("run-final:{id}"))
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    }
    sqlx::query(
        "INSERT INTO audit_events(actor_user_id,project_id,action,target_type,target_id,details) \
         VALUES($1,$2,'agent.run_finished','run',$3,$4)",
    )
    .bind(run.get::<Uuid, _>("requested_by_user_id"))
    .bind(run.get::<Option<Uuid>, _>("project_id"))
    .bind(id.to_string())
    .bind(json!({"status":status}))
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(Envelope { data: json!({}) }))
}

pub async fn verify_capability(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<CapabilityRequest>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_agent_runtime(&state, &headers)?;
    if id != value.run_id {
        return Err(ApiError::invalid("run identity mismatch"));
    }
    if !value.arguments.is_object()
        || serde_json::to_vec(&value.arguments).is_ok_and(|bytes| bytes.len() > 1024 * 1024)
        || tool_arguments_digest(&value.tool_name, &value.arguments) != value.arguments_digest
    {
        return Err(ApiError::forbidden());
    }
    let context = load_capability_context(&state, id).await?;
    let decision = capability_decision(&context, &value);
    if let Err(reason) = decision {
        return Ok(Json(Envelope {
            data: json!({"allowed":false,"reason":reason}),
        }));
    }
    if let Some(approval_scope) = tool_approval_scope(&value.tool_name)
        && context
            .required_approvals
            .iter()
            .any(|required| required == approval_scope)
    {
        return request_tool_approval(&state, &context, &value, approval_scope).await;
    }
    issue_execution_grant(&state, &value).await
}

fn tool_approval_scope(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "runtime.submit_job" => Some("runtime.compute"),
        "project.write_file" => Some("project.write"),
        "runtime.cancel_job" => Some("runtime.cancel"),
        "artifact.register" => Some("artifact.register"),
        _ => None,
    }
}

async fn request_tool_approval(
    state: &AppState,
    context: &CapabilityContext,
    value: &CapabilityRequest,
    approval_scope: &str,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let approval_id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::minutes(15);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let transitioned = sqlx::query(
        "UPDATE runs SET status='waiting_approval',capability_token_hash=NULL, \
         capability_expires_at=NULL,updated_at=NOW() WHERE id=$1 AND status='running'",
    )
    .bind(value.run_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .rows_affected();
    if transitioned != 1 {
        return Ok(Json(Envelope {
            data: json!({"allowed":false,"reason":"run_not_approvable"}),
        }));
    }
    sqlx::query(
        "INSERT INTO run_approvals(id,run_id,requested_by_user_id,tool_call_id,tool_name, \
         arguments_digest,arguments,risk,approval_scope,expires_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(approval_id)
    .bind(value.run_id)
    .bind(context.user_id)
    .bind(&value.tool_call_id)
    .bind(&value.tool_name)
    .bind(&value.arguments_digest)
    .bind(&value.arguments)
    .bind(&value.risk)
    .bind(approval_scope)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    sqlx::query(
        "INSERT INTO audit_events(actor_user_id,project_id,action,target_type,target_id,details) \
         VALUES($1,$2,'agent.approval_requested','run_approval',$3,$4)",
    )
    .bind(context.user_id)
    .bind(context.project_id)
    .bind(approval_id.to_string())
    .bind(json!({"run_id":value.run_id,"tool_call_id":value.tool_call_id,"tool_name":value.tool_name,"arguments_digest":value.arguments_digest,"approval_scope":approval_scope}))
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: json!({
            "allowed":false,
            "reason":"approval_required",
            "approvalId":approval_id,
            "approvalScope":approval_scope,
            "expiresAt":expires_at
        }),
    }))
}

async fn issue_execution_grant(
    state: &AppState,
    value: &CapabilityRequest,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let execution_token = random_secret(32);
    let grant_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO run_tool_grants(id,run_id,tool_call_id,tool_name,arguments_digest,risk,execution_token_hash,decision,expires_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,'allowed',NOW()+INTERVAL '2 minutes') \
         ON CONFLICT(run_id,tool_call_id) DO NOTHING",
    )
    .bind(grant_id)
    .bind(value.run_id)
    .bind(&value.tool_call_id)
    .bind(&value.tool_name)
    .bind(&value.arguments_digest)
    .bind(&value.risk)
    .bind(sha256(&execution_token))
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?
    .rows_affected();
    if inserted != 1 {
        return Ok(Json(Envelope {
            data: json!({"allowed":false,"reason":"tool_call_reused"}),
        }));
    }
    Ok(Json(Envelope {
        data: json!({"allowed":true,"executionToken":execution_token}),
    }))
}

pub async fn execute_tool(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<ToolExecutionRequest>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_agent_runtime(&state, &headers)?;
    if id != value.run_id
        || tool_arguments_digest(&value.tool_name, &value.arguments) != value.arguments_digest
    {
        return Err(ApiError::forbidden());
    }
    let context = load_capability_context(&state, id).await?;
    let capability = CapabilityRequest {
        run_id: value.run_id,
        user_id: value.user_id,
        project_id: value.project_id,
        tool_call_id: value.tool_call_id.clone(),
        tool_name: value.tool_name.clone(),
        arguments_digest: value.arguments_digest.clone(),
        risk: value.risk.clone(),
        run_capability_token: value.run_capability_token.clone(),
        arguments: value.arguments.clone(),
    };
    capability_decision(&context, &capability).map_err(|_| ApiError::forbidden())?;
    consume_execution_grant(&state, &value).await?;
    let actor = AuthUser::internal(
        context.user_id,
        context.email.clone(),
        context.display_name.clone(),
        context.user_role.clone(),
    );
    let content = dispatch_tool(
        &state,
        &actor,
        &context,
        id,
        &value.tool_call_id,
        &value.tool_name,
        &value.arguments,
    )
    .await?;
    let evidence = backend_evidence(
        id,
        context.project_id,
        &value.tool_call_id,
        &value.tool_name,
        &value.arguments,
        &content,
    );
    Ok(Json(Envelope {
        data: json!({
            "content":content,
            "evidence":evidence,
            "activity":{"tool":value.tool_name,"run_id":id}
        }),
    }))
}

fn backend_evidence(
    run_id: Uuid,
    project_id: Option<Uuid>,
    tool_call_id: &str,
    tool_name: &str,
    arguments: &Value,
    content: &Value,
) -> Vec<Value> {
    if tool_name == "runtime.get_job" {
        if content.get("state").and_then(Value::as_str) != Some("succeeded") {
            return Vec::new();
        }
        let job_id = content
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(tool_call_id);
        return content
            .get("artifacts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artifact| {
                let artifact_id = artifact.get("id")?.as_str()?;
                let digest = artifact.get("sha256")?.as_str()?.to_ascii_lowercase();
                let size_bytes = artifact.get("size_bytes")?.as_i64()?;
                if size_bytes <= 0
                    || digest.len() != 64
                    || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return None;
                }
                let evidence_digest =
                    sha256_hex(format!("{run_id}:{tool_call_id}:{artifact_id}:{digest}"));
                Some(json!({
                    "id":format!("ev-{evidence_digest}"),
                    "kind":"artifact",
                    "runId":run_id,
                    "sourceId":artifact_id,
                    "digest":format!("sha256:{digest}"),
                    "locator":format!("runtime://jobs/{job_id}/artifacts/{artifact_id}"),
                    "toolCallId":tool_call_id,
                    "metadata":{
                        "projectId":project_id,
                        "tool":tool_name,
                        "issuer":"shennong-os",
                        "jobId":job_id,
                        "relativePath":artifact.get("relative_path"),
                        "sizeBytes":size_bytes
                    }
                }))
            })
            .collect();
    }
    if tool_name == "artifact.register" {
        let source_id = content.get("id").and_then(Value::as_str);
        let digest = content
            .get("content_sha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase);
        let size_bytes = content.get("size_bytes").and_then(Value::as_i64);
        let (Some(source_id), Some(digest), Some(size_bytes)) = (source_id, digest, size_bytes)
        else {
            return Vec::new();
        };
        if size_bytes <= 0
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Vec::new();
        }
        let evidence_digest = sha256_hex(format!("{run_id}:{tool_call_id}:{source_id}:{digest}"));
        return vec![json!({
            "id":format!("ev-{evidence_digest}"),
            "kind":"artifact",
            "runId":run_id,
            "sourceId":source_id,
            "digest":format!("sha256:{digest}"),
            "locator":content.get("locator"),
            "toolCallId":tool_call_id,
            "metadata":{
                "projectId":project_id,
                "tool":tool_name,
                "issuer":"shennong-os",
                "sizeBytes":size_bytes
            }
        })];
    }
    let kind = match tool_name {
        "db.discover_resources" | "db.query_resource" => "query",
        "db.inspect_resource" | "db.get_provenance" => "dataset",
        "project.read_file" | "project.write_file" | "environment.plan" => "tool-result",
        _ => return Vec::new(),
    };
    let canonical = serde_json::to_vec(content).expect("tool result is JSON serializable");
    let digest = sha256_hex(canonical);
    let source_id = arguments
        .get("resource")
        .and_then(Value::as_str)
        .or_else(|| {
            ["id", "job_id", "jobId", "resource", "uri", "plan_id"]
                .iter()
                .find_map(|key| content.get(key).and_then(Value::as_str))
        })
        .unwrap_or(tool_call_id);
    vec![json!({
        "id":format!("ev-{digest}"),
        "kind":kind,
        "runId":run_id,
        "sourceId":source_id,
        "digest":format!("sha256:{digest}"),
        "toolCallId":tool_call_id,
        "metadata":{
            "projectId":project_id,
            "tool":tool_name,
            "issuer":"shennong-os",
            "operation":arguments.get("operation")
        }
    })]
}

struct CapabilityContext {
    project_id: Option<Uuid>,
    user_id: Uuid,
    email: String,
    display_name: String,
    user_role: String,
    project_role: String,
    status: String,
    token_hash: Option<Vec<u8>>,
    token_expires: Option<chrono::DateTime<Utc>>,
    allowed_tools: Vec<String>,
    allowed_project_read: Vec<String>,
    allowed_project_write: Vec<String>,
    allowed_compute_profiles: Vec<String>,
    required_approvals: Vec<String>,
}

const CAPABILITY_CONTEXT_QUERY: &str = "SELECT r.project_id,r.requested_by_user_id,r.status,r.capability_token_hash,r.capability_expires_at,r.input, \
            u.email,u.display_name,u.role AS user_role,COALESCE(pm.role,'') AS project_role \
     FROM runs r JOIN users u ON u.id=r.requested_by_user_id \
     LEFT JOIN projects p ON p.id=r.project_id \
     LEFT JOIN project_members pm ON pm.project_id=r.project_id AND pm.user_id=r.requested_by_user_id \
     WHERE r.id=$1 AND u.status='active' AND (r.project_id IS NULL OR p.status='active')";

async fn load_capability_context(
    state: &AppState,
    run_id: Uuid,
) -> Result<CapabilityContext, ApiError> {
    let row = sqlx::query(CAPABILITY_CONTEXT_QUERY)
        .bind(run_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    let input: Value = row.get("input");
    let allowed_tools = input
        .get("allowed_tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let permission_list = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    Ok(CapabilityContext {
        project_id: row.get("project_id"),
        user_id: row.get("requested_by_user_id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        user_role: row.get("user_role"),
        project_role: row.get("project_role"),
        status: row.get("status"),
        token_hash: row.get("capability_token_hash"),
        token_expires: row.get("capability_expires_at"),
        allowed_tools,
        allowed_project_read: permission_list("allowed_project_read"),
        allowed_project_write: permission_list("allowed_project_write"),
        allowed_compute_profiles: permission_list("allowed_compute_profiles"),
        required_approvals: permission_list("required_approvals"),
    })
}

fn capability_decision(
    context: &CapabilityContext,
    request: &CapabilityRequest,
) -> Result<(), &'static str> {
    if context.status != "running"
        || request.user_id != context.user_id
        || request.project_id != context.project_id
        || context
            .token_expires
            .is_none_or(|expires| expires <= Utc::now())
        || context.token_hash.as_deref().is_none_or(|expected| {
            !bool::from(sha256(&request.run_capability_token).ct_eq(expected))
        })
    {
        return Err("run_capability_invalid");
    }
    if request.arguments_digest.len() != 64
        || !request
            .arguments_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("arguments_digest_invalid");
    }
    let definition = TOOLS
        .iter()
        .find(|definition| definition.name == request.tool_name)
        .ok_or("tool_not_registered")?;
    if definition.risk != request.risk
        || (definition.project_required && request.project_id.is_none())
    {
        return Err("tool_contract_mismatch");
    }
    if !context
        .allowed_tools
        .iter()
        .any(|name| name == definition.name)
    {
        return Err("tool_not_in_run_capability");
    }
    if context.project_id.is_some()
        && context.user_role != "admin"
        && context.project_role.is_empty()
    {
        return Err("project_access_revoked");
    }
    match definition.risk {
        "read" => Ok(()),
        "write" | "network" | "compute" | "destructive"
            if context.user_role == "admin"
                || matches!(context.project_role.as_str(), "owner" | "admin" | "editor") =>
        {
            Ok(())
        }
        "admin" if context.user_role == "admin" => Ok(()),
        _ => Err("project_write_role_required"),
    }
}

async fn consume_execution_grant(
    state: &AppState,
    request: &ToolExecutionRequest,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let row = sqlx::query(
        "SELECT execution_token_hash,tool_name,arguments_digest,risk,used_at,expires_at \
         FROM run_tool_grants WHERE run_id=$1 AND tool_call_id=$2 FOR UPDATE",
    )
    .bind(request.run_id)
    .bind(&request.tool_call_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::forbidden)?;
    let expected: Vec<u8> = row.get("execution_token_hash");
    if !bool::from(sha256(&request.execution_token).ct_eq(&expected))
        || row.get::<String, _>("tool_name") != request.tool_name
        || row.get::<String, _>("arguments_digest") != request.arguments_digest
        || row.get::<String, _>("risk") != request.risk
        || row
            .get::<Option<chrono::DateTime<Utc>>, _>("used_at")
            .is_some()
        || row.get::<chrono::DateTime<Utc>, _>("expires_at") <= Utc::now()
    {
        return Err(ApiError::forbidden());
    }
    sqlx::query("UPDATE run_tool_grants SET used_at=NOW() WHERE run_id=$1 AND tool_call_id=$2")
        .bind(request.run_id)
        .bind(&request.tool_call_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(())
}

async fn dispatch_tool(
    state: &AppState,
    actor: &AuthUser,
    context: &CapabilityContext,
    run_id: Uuid,
    tool_call_id: &str,
    name: &str,
    arguments: &Value,
) -> Result<Value, ApiError> {
    match name {
        "skill.load" => load_skill_tool(state, actor, arguments).await,
        "plan.propose" => propose_plan_tool(state, run_id, arguments).await,
        "plan.update" => update_plan_tool(state, run_id, arguments).await,
        "db.discover_resources"
        | "db.inspect_resource"
        | "db.query_resource"
        | "db.get_provenance" => {
            if let Some(project_id) = context.project_id {
                super::data_plane::execute_db_tool(state, project_id, name, arguments).await
            } else {
                super::data_plane::execute_public_db_tool(state, name, arguments).await
            }
        }
        "project.list_files" => {
            let project_id = context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?;
            list_project_files_tool(state, project_id, &context.allowed_project_read, arguments)
                .await
        }
        "project.read_file" => {
            let project_id = context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?;
            read_project_file_tool(state, project_id, &context.allowed_project_read, arguments)
                .await
        }
        "project.write_file" => {
            let project_id = context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?;
            write_project_file_tool(
                state,
                actor,
                project_id,
                &context.allowed_project_write,
                arguments,
            )
            .await
        }
        "environment.plan" => plan_environment_tool(
            context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?,
            arguments,
        ),
        "runtime.submit_job" => {
            let project_id = context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?;
            let requested_profile = arguments
                .pointer("/job_spec/worker_profile")
                .and_then(Value::as_str)
                .unwrap_or("cpu-small");
            let requested_profile = if requested_profile == "standard" {
                "cpu-small"
            } else {
                requested_profile
            };
            if !context
                .allowed_compute_profiles
                .iter()
                .any(|profile| profile == requested_profile)
            {
                return Err(ApiError::forbidden());
            }
            let resolved_arguments = resolve_runtime_project_files(
                state,
                project_id,
                &context.allowed_project_read,
                arguments,
            )
            .await?;
            submit_agent_job(
                state,
                actor,
                project_id,
                run_id,
                tool_call_id,
                &resolved_arguments,
            )
            .await
        }
        "runtime.get_job" | "runtime.cancel_job" => {
            let project_id = context
                .project_id
                .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?;
            let id = arguments
                .get("job_id")
                .and_then(Value::as_str)
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| ApiError::invalid("job_id must be a UUID"))?;
            agent_job_action(
                state,
                actor,
                project_id,
                id,
                if name.ends_with("cancel_job") {
                    "cancel"
                } else {
                    "get"
                },
                arguments
                    .get("include_logs")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await
        }
        "artifact.register" => {
            register_artifact_tool(
                state,
                actor,
                context
                    .project_id
                    .ok_or_else(|| ApiError::invalid("this tool requires a Project"))?,
                arguments,
            )
            .await
        }
        // analysis.validate is executed deterministically inside Agent Runtime.
        "analysis.validate" => Err(ApiError::invalid(
            "analysis.validate must execute in the deterministic runtime validator",
        )),
        _ => Err(ApiError::forbidden()),
    }
}

async fn resolve_runtime_project_files(
    state: &AppState,
    project_id: Uuid,
    allowed_read: &[String],
    arguments: &Value,
) -> Result<Value, ApiError> {
    let mut resolved = arguments.clone();
    let job_spec = resolved
        .get_mut("job_spec")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ApiError::invalid("job_spec must be an object"))?;
    if job_spec.contains_key("workspace_files") {
        return Err(ApiError::invalid(
            "workspace_files is OS-resolved and cannot be supplied by an Agent",
        ));
    }
    let declarations = job_spec
        .remove("project_files")
        .unwrap_or_else(|| json!([]));
    let declarations = declarations
        .as_array()
        .ok_or_else(|| ApiError::invalid("job_spec.project_files must be an array"))?;
    if declarations.len() > 32 {
        return Err(ApiError::invalid(
            "job_spec.project_files accepts at most 32 governed files",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut workspace_files = Vec::with_capacity(declarations.len());
    let mut replacements = std::collections::HashMap::new();
    let mut total_bytes = 0_usize;
    for declaration in declarations {
        let requested = declaration.as_str().ok_or_else(|| {
            ApiError::invalid("job_spec.project_files entries must be project URI strings")
        })?;
        let uri = normalize_project_uri(requested, false)?;
        if !seen.insert(uri.clone()) {
            return Err(ApiError::invalid("duplicate project file input"));
        }
        let file = read_project_file_tool(
            state,
            project_id,
            allowed_read,
            &json!({"uri":uri,"max_bytes":1_048_576}),
        )
        .await?;
        if file.get("truncated").and_then(Value::as_bool) != Some(false) {
            return Err(ApiError::invalid(
                "project file exceeds the 1 MiB runtime staging limit",
            ));
        }
        let content = file
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("project file content is unavailable"))?;
        total_bytes = total_bytes.saturating_add(content.len());
        if total_bytes > 1_048_576 {
            return Err(ApiError::invalid(
                "project files exceed the 1 MiB runtime staging limit",
            ));
        }
        let path = uri
            .strip_prefix("project://current/")
            .ok_or_else(|| ApiError::invalid("project file URI is invalid"))?;
        let digest = file
            .get("content_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("project file digest is unavailable"))?;
        workspace_files.push(json!({"path":path,"content":content,"sha256":digest}));
        replacements.insert(uri.clone(), format!("workspace-input://{path}"));
    }
    let argv = job_spec
        .get_mut("argv")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ApiError::invalid("job_spec.argv is required"))?;
    let mut replacement_count = 0_usize;
    for argument in argv {
        if let Some(value) = argument.as_str()
            && let Some(replacement) = replacements.get(value)
        {
            *argument = json!(replacement);
            replacement_count += 1;
        }
    }
    if !workspace_files.is_empty() && replacement_count == 0 {
        return Err(ApiError::invalid(
            "at least one argv value must reference a staged project file URI",
        ));
    }
    job_spec.insert("workspace_files".into(), Value::Array(workspace_files));
    Ok(resolved)
}

async fn load_provider(
    state: &AppState,
    user_id: Uuid,
    selected: Option<Uuid>,
) -> Result<Value, ApiError> {
    let row = sqlx::query(
        "SELECT id,provider_kind,base_url,model,data_policy,encrypted_api_key FROM model_providers \
         WHERE owner_user_id=$1 AND enabled=TRUE AND id=COALESCE($2,(SELECT id FROM model_providers \
         WHERE owner_user_id=$1 AND enabled=TRUE ORDER BY is_default DESC,updated_at DESC LIMIT 1))",
    )
    .bind(user_id)
    .bind(selected)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(|| ApiError::conflict("no enabled model provider is configured"))?;
    let id: Uuid = row.get("id");
    let encrypted: Option<Vec<u8>> = row.get("encrypted_api_key");
    let api_key = encrypted
        .as_deref()
        .map(|encrypted| {
            decrypt_secret(
                &state.config.provider_encryption_key,
                format!("{user_id}:{id}").as_bytes(),
                encrypted,
            )
        })
        .transpose()?;
    Ok(provider_payload(
        row.get("provider_kind"),
        row.get("base_url"),
        row.get("model"),
        row.get("data_policy"),
        api_key,
    ))
}

fn provider_payload(
    kind: String,
    base_url: String,
    model: String,
    data_policy: String,
    api_key: Option<String>,
) -> Value {
    let mut provider = json!({
        "kind":kind,"baseUrl":base_url,"model":model,
        "capabilities":["tools","thinking"],"dataPolicy":data_policy
    });
    if let Some(api_key) = api_key {
        provider["apiKey"] = json!(api_key);
    }
    provider
}

async fn load_messages(state: &AppState, thread_id: Uuid) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT id,role,content_json,created_at FROM messages WHERE thread_id=$1 \
         AND role IN ('user','assistant') AND status='completed' ORDER BY created_at,id LIMIT 200",
    )
    .bind(thread_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "id":row.get::<Uuid,_>("id"),"role":row.get::<String,_>("role"),
                "content":row.get::<Value,_>("content_json"),
                "timestamp":row.get::<chrono::DateTime<Utc>,_>("created_at").timestamp_millis()
            })
        })
        .collect())
}

async fn load_run_skills(
    state: &AppState,
    thread_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT s.id,s.name,s.description,s.manifest,sv.version,sv.package_version,sv.content,sv.content_sha256 \
         FROM thread_skills ts JOIN skills s ON s.id=ts.skill_id \
         JOIN skill_versions sv ON sv.skill_id=s.id AND sv.version=ts.skill_version \
         WHERE ts.thread_id=$1 AND ts.enabled=TRUE AND s.lifecycle='active' \
         AND (s.owner_user_id IS NULL OR s.owner_user_id=$2) ORDER BY s.slug",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let manifest: Value = row.get("manifest");
            let manifest_id = manifest
                .pointer("/metadata/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| row.get::<Uuid, _>("id").to_string());
            let version = manifest
                .pointer("/metadata/version")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| row.get::<String, _>("package_version"));
            let database_id = row.get::<Uuid, _>("id");
            let database_version = row.get::<i32, _>("version");
            json!({
                "id":manifest_id,"version":version,
                "digest":format!("sha256:{}",row.get::<String,_>("content_sha256")),
                "loadRef":format!("{database_id}:{database_version}"),
                "name":row.get::<String,_>("name"),"description":row.get::<String,_>("description"),
                "content":row.get::<String,_>("content"),
                "permissions":manifest.pointer("/spec/permissions").cloned().unwrap_or_else(||json!({
                    "tools":[],"projectRead":[],"projectWrite":[],"datasetAccess":"public",
                    "networkHosts":[],"computeProfiles":[],"approvals":[]
                })),
                "_db_version":database_version,"_manifest":manifest
            })
        })
        .collect())
}

fn allowed_tools(
    user_role: &str,
    project_role: &str,
    has_project: bool,
    skills: &[Value],
) -> Vec<&'static str> {
    let declared = skills
        .iter()
        .flat_map(|skill| {
            skill
                .pointer("/permissions/tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    TOOLS
        .iter()
        .filter(|definition| {
            let scope_allows = has_project
                || matches!(
                    definition.name,
                    "skill.load"
                        | "analysis.validate"
                        | "db.discover_resources"
                        | "db.inspect_resource"
                        | "db.query_resource"
                        | "db.get_provenance"
                );
            let role_allows = definition.risk == "read"
                || user_role == "admin"
                || matches!(project_role, "owner" | "admin" | "editor");
            let skill_allows = declared.contains(&definition.name)
                || matches!(
                    definition.name,
                    "skill.load" | "plan.propose" | "plan.update" | "analysis.validate"
                );
            scope_allows && role_allows && skill_allows
        })
        .map(|definition| definition.name)
        .collect()
}

fn declared_string_permissions(skills: &[Value], key: &str) -> Vec<String> {
    let mut permissions = skills
        .iter()
        .flat_map(|skill| {
            skill
                .pointer(&format!("/permissions/{key}"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    permissions.sort();
    permissions.dedup();
    permissions
}

fn tool_profile(project_role: &str, user_role: &str, has_project: bool) -> &'static str {
    if !has_project {
        "global-read"
    } else if user_role == "admin" || matches!(project_role, "owner" | "admin" | "editor") {
        "project-write"
    } else {
        "project-analysis"
    }
}

async fn load_memories(
    state: &AppState,
    user_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT m.id,m.title,v.content,v.content_sha256 FROM memories m JOIN memory_versions v \
         ON v.memory_id=m.id AND v.version=m.current_version WHERE m.owner_user_id=$1 \
         AND m.lifecycle='active' AND (m.project_id IS NULL OR ($2::uuid IS NOT NULL AND m.project_id=$2)) \
         ORDER BY m.updated_at DESC LIMIT 50",
    )
    .bind(user_id)
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({"id":row.get::<Uuid,_>("id"),"title":row.get::<String,_>("title"),
                "content":row.get::<String,_>("content"),"digest":row.get::<String,_>("content_sha256")})
        })
        .collect())
}

async fn load_artifacts(state: &AppState, project_id: Uuid) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT id,kind,name,locator,content_sha256,metadata FROM artifacts WHERE project_id=$1 \
         ORDER BY created_at DESC LIMIT 50",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({"id":row.get::<Uuid,_>("id"),"kind":row.get::<String,_>("kind"),
                "name":row.get::<String,_>("name"),"locator":row.get::<String,_>("locator"),
                "digest":row.get::<Option<String>,_>("content_sha256"),"metadata":row.get::<Value,_>("metadata")})
        })
        .collect())
}

async fn load_skill_tool(
    state: &AppState,
    actor: &AuthUser,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let reference = arguments
        .get("load_ref")
        .or_else(|| arguments.get("skill_version_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("load_ref is required"))?;
    let (id, version) = reference
        .split_once(':')
        .ok_or_else(|| ApiError::invalid("load_ref must be UUID:version"))?;
    let id: Uuid = id
        .parse()
        .map_err(|_| ApiError::invalid("load_ref contains an invalid UUID"))?;
    let version: i32 = version
        .parse()
        .map_err(|_| ApiError::invalid("skill version is invalid"))?;
    let row = sqlx::query(
        "SELECT s.id,s.name,s.manifest,s.lifecycle,s.trust_level,s.current_version,s.owner_user_id, \
                sv.version,sv.content,sv.content_sha256,sv.package_version \
         FROM skills s JOIN skill_versions sv ON sv.skill_id=s.id WHERE s.id=$1 AND sv.version=$2 \
         AND (s.owner_user_id IS NULL OR s.owner_user_id=$3)",
    )
    .bind(id)
    .bind(version)
    .bind(actor.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    if row.get::<String, _>("lifecycle") != "active" {
        return Err(ApiError::forbidden());
    }
    if row.get::<String, _>("trust_level") == "builtin_signed"
        && row.get::<i32, _>("current_version") != version
    {
        return Err(ApiError::forbidden());
    }
    Ok(json!({
        "id":id,"version":version,"name":row.get::<String,_>("name"),
        "manifest":row.get::<Value,_>("manifest"),"content":row.get::<String,_>("content"),
        "content_sha256":row.get::<String,_>("content_sha256"),
        "package_version":row.get::<String,_>("package_version")
    }))
}

async fn propose_plan_tool(
    state: &AppState,
    run_id: Uuid,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let steps = arguments
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty() && steps.len() <= 32)
        .ok_or_else(|| ApiError::invalid("plan steps must contain 1..32 items"))?;
    let items = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            json!({"id":format!("step-{}",index+1),"title":step.get("title"),"type":step.get("type"),"status":"pending"})
        })
        .collect::<Vec<_>>();
    let row = sqlx::query(
        "INSERT INTO task_plans(run_id,version,items) VALUES($1,1,$2) \
         ON CONFLICT(run_id) DO UPDATE SET version=task_plans.version+1,items=EXCLUDED.items,updated_at=NOW() \
         RETURNING version,updated_at",
    )
    .bind(run_id)
    .bind(json!(items))
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(json!({"plan_id":run_id,"version":row.get::<i32,_>("version"),"items":items}))
}

async fn update_plan_tool(
    state: &AppState,
    run_id: Uuid,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let step_id = arguments
        .get("step_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("step_id is required"))?;
    let status = arguments
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| matches!(*status, "pending" | "in_progress" | "completed" | "failed"))
        .ok_or_else(|| ApiError::invalid("invalid plan step status"))?;
    let row = sqlx::query("SELECT version,items FROM task_plans WHERE run_id=$1 FOR UPDATE")
        .bind(run_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    let mut items: Vec<Value> = serde_json::from_value(row.get("items"))
        .map_err(|_| ApiError::invalid("stored task plan is invalid"))?;
    let item = items
        .iter_mut()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(step_id))
        .ok_or_else(ApiError::not_found)?;
    item.as_object_mut()
        .expect("plan item was created as an object")
        .insert("status".into(), json!(status));
    if let Some(note) = arguments.get("note").and_then(Value::as_str) {
        item.as_object_mut()
            .expect("plan item object")
            .insert("note".into(), json!(note));
    }
    let version = row.get::<i32, _>("version") + 1;
    sqlx::query("UPDATE task_plans SET version=$2,items=$3,updated_at=NOW() WHERE run_id=$1")
        .bind(run_id)
        .bind(version)
        .bind(json!(items))
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(json!({"plan_id":run_id,"version":version,"items":items}))
}

fn normalize_project_uri(raw: &str, directory: bool) -> Result<String, ApiError> {
    const PREFIX: &str = "project://current/";
    let raw = raw.trim();
    let relative = raw
        .strip_prefix(PREFIX)
        .ok_or_else(|| ApiError::invalid("project URI must use project://current/"))?;
    if raw.len() > 1024
        || raw.chars().any(char::is_control)
        || raw.contains(['\\', '?', '#'])
        || relative.split('/').any(|part| matches!(part, "." | ".."))
        || (!directory && (relative.is_empty() || relative.ends_with('/')))
        || (directory && !relative.is_empty() && !relative.ends_with('/'))
    {
        return Err(ApiError::invalid("invalid project-relative URI"));
    }
    Ok(raw.to_owned())
}

fn project_scope_allows(uri: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        normalize_project_uri(scope, scope.ends_with('/'))
            .ok()
            .is_some_and(|scope| uri == scope || (scope.ends_with('/') && uri.starts_with(&scope)))
    })
}

fn project_scope_intersects(directory: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        normalize_project_uri(scope, scope.ends_with('/'))
            .ok()
            .is_some_and(|scope| scope.starts_with(directory) || directory.starts_with(&scope))
    })
}

fn project_resource_artifact_uri(uri: &str) -> Option<(&str, &str)> {
    let segments = uri
        .strip_prefix("project://current/")?
        .split('/')
        .collect::<Vec<_>>();
    match segments.as_slice() {
        ["resources", resource_id, "artifacts", artifact_id]
            if !resource_id.is_empty() && !artifact_id.is_empty() =>
        {
            Some((resource_id, artifact_id))
        }
        _ => None,
    }
}

async fn list_project_files_tool(
    state: &AppState,
    project_id: Uuid,
    allowed_read: &[String],
    arguments: &Value,
) -> Result<Value, ApiError> {
    let uri = normalize_project_uri(
        arguments
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or("project://current/"),
        true,
    )?;
    if !project_scope_intersects(&uri, allowed_read) {
        return Err(ApiError::forbidden());
    }
    let rows = sqlx::query(
        "SELECT path,octet_length(content)::bigint AS size_bytes,content_sha256,version,updated_at \
         FROM project_files pf WHERE project_id=$1 AND left(path,length($2))=$2 \
         AND EXISTS (SELECT 1 FROM unnest($3::text[]) AS scope WHERE path=scope \
           OR (right(scope,1)='/' AND left(path,length(scope))=scope)) \
         ORDER BY path LIMIT 200",
    )
    .bind(project_id)
    .bind(&uri)
    .bind(allowed_read)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(json!({
        "uri":uri,
        "files":rows.into_iter().map(|row| json!({
            "uri":row.get::<String,_>("path"),
            "size_bytes":row.get::<i64,_>("size_bytes"),
            "content_sha256":row.get::<String,_>("content_sha256"),
            "version":row.get::<i32,_>("version"),
            "updated_at":row.get::<chrono::DateTime<Utc>,_>("updated_at")
        })).collect::<Vec<_>>(),
        "limit":200
    }))
}

async fn read_project_file_tool(
    state: &AppState,
    project_id: Uuid,
    allowed_read: &[String],
    arguments: &Value,
) -> Result<Value, ApiError> {
    let uri = normalize_project_uri(
        arguments
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("uri is required"))?,
        false,
    )?;
    if !project_scope_allows(&uri, allowed_read) {
        return Err(ApiError::forbidden());
    }
    let max_bytes = arguments
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65_536);
    if !(1..=1_048_576).contains(&max_bytes) {
        return Err(ApiError::invalid("max_bytes must be 1..1048576"));
    }
    if let Some((resource_id, artifact_id)) = project_resource_artifact_uri(&uri) {
        return super::data_plane::read_project_artifact_text(
            state,
            project_id,
            resource_id,
            artifact_id,
            usize::try_from(max_bytes).unwrap_or(1_048_576),
        )
        .await;
    }
    let row = sqlx::query(
        "SELECT content,content_sha256,version,updated_at FROM project_files \
         WHERE project_id=$1 AND path=$2",
    )
    .bind(project_id)
    .bind(&uri)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    let content: String = row.get("content");
    let original_bytes = content.len();
    let mut end = usize::try_from(max_bytes)
        .unwrap_or(1_048_576)
        .min(original_bytes);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    Ok(json!({
        "uri":uri,
        "content":&content[..end],
        "size_bytes":original_bytes,
        "truncated":end < original_bytes,
        "content_sha256":row.get::<String,_>("content_sha256"),
        "version":row.get::<i32,_>("version"),
        "updated_at":row.get::<chrono::DateTime<Utc>,_>("updated_at"),
        "content_is_untrusted":true
    }))
}

async fn write_project_file_tool(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    allowed_write: &[String],
    arguments: &Value,
) -> Result<Value, ApiError> {
    let uri = normalize_project_uri(
        arguments
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("uri is required"))?,
        false,
    )?;
    if !project_scope_allows(&uri, allowed_write) {
        return Err(ApiError::forbidden());
    }
    let content = arguments
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("content is required"))?;
    if content.len() > 1_048_576 {
        return Err(ApiError::invalid("project file exceeds 1 MiB"));
    }
    let overwrite = arguments
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let digest = sha256_hex(content);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT version FROM project_files WHERE project_id=$1 AND path=$2 FOR UPDATE",
    )
    .bind(project_id)
    .bind(&uri)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if existing.is_some() && !overwrite {
        return Err(ApiError::conflict(
            "project file exists; overwrite=true is required",
        ));
    }
    let version = if let Some(version) = existing {
        let next = version + 1;
        sqlx::query(
            "UPDATE project_files SET content=$3,content_sha256=$4,version=$5, \
             updated_by_user_id=$6,updated_at=NOW() WHERE project_id=$1 AND path=$2",
        )
        .bind(project_id)
        .bind(&uri)
        .bind(content)
        .bind(&digest)
        .bind(next)
        .bind(actor.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        next
    } else {
        sqlx::query(
            "INSERT INTO project_files(project_id,path,content,content_sha256,created_by_user_id,updated_by_user_id) \
             VALUES($1,$2,$3,$4,$5,$5)",
        )
        .bind(project_id)
        .bind(&uri)
        .bind(content)
        .bind(&digest)
        .bind(actor.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        1
    };
    super::audit_tx(
        &mut tx,
        Some(actor.id),
        Some(project_id),
        "project.file_write",
        "project_file",
        Some(uri.clone()),
        json!({"version":version,"content_sha256":digest,"size_bytes":content.len()}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(json!({
        "uri":uri,"version":version,"content_sha256":digest,
        "size_bytes":content.len(),"content_is_untrusted":true
    }))
}

fn plan_environment_tool(project_id: Uuid, arguments: &Value) -> Result<Value, ApiError> {
    let packages = arguments
        .get("packages")
        .and_then(Value::as_array)
        .filter(|packages| !packages.is_empty() && packages.len() <= 100)
        .ok_or_else(|| ApiError::invalid("packages must contain 1..100 entries"))?;
    let mut normalized_packages = Vec::with_capacity(packages.len());
    for package in packages {
        let package = package
            .as_str()
            .map(str::trim)
            .filter(|package| {
                !package.is_empty()
                    && package.len() <= 256
                    && package
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._<>=!*,+:-".contains(&byte))
            })
            .ok_or_else(|| ApiError::invalid("invalid declarative package constraint"))?;
        if normalized_packages
            .iter()
            .any(|existing| existing == package)
        {
            return Err(ApiError::invalid("duplicate environment package"));
        }
        normalized_packages.push(package.to_owned());
    }
    let channels = arguments
        .get("channels")
        .and_then(Value::as_array)
        .map(|channels| {
            channels
                .iter()
                .map(|channel| {
                    channel
                        .as_str()
                        .filter(|channel| matches!(*channel, "conda-forge" | "bioconda"))
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            ApiError::invalid(
                                "V1 environment channels are limited to conda-forge and bioconda",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec!["conda-forge".to_owned(), "bioconda".to_owned()]);
    if channels.is_empty() || channels.len() > 2 {
        return Err(ApiError::invalid("channels must contain 1..2 entries"));
    }
    let mut unique_channels = channels.clone();
    unique_channels.sort();
    unique_channels.dedup();
    if unique_channels.len() != channels.len() {
        return Err(ApiError::invalid("duplicate environment channel"));
    }
    let plan = json!({
        "schema":"shennong.one/environment-plan/v1",
        "project_id":project_id,
        "resolver":"pixi",
        "packages":normalized_packages,
        "channels":channels,
        "network_policy":"internet_only"
    });
    let digest =
        sha256_hex(serde_json::to_vec(&plan).expect("environment plan contains JSON-safe values"));
    Ok(json!({
        "plan_id":format!("sha256:{digest}"),
        "content_sha256":digest,
        "status":"planned",
        "lock_state":"not_resolved",
        "plan":plan,
        "execution":"Submit a reviewed cpu-small Runtime job to resolve and materialize a lock."
    }))
}

async fn register_artifact_tool(
    state: &AppState,
    actor: &AuthUser,
    project_id: Uuid,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let requested_uri = arguments
        .get("uri")
        .and_then(Value::as_str)
        .filter(|uri| uri.len() <= 4096)
        .ok_or_else(|| ApiError::invalid("artifact uri is required"))?;
    let requested_kind = arguments
        .get("kind")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| ApiError::invalid("artifact kind is required"))?;
    let provenance = arguments
        .get("provenance")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| ApiError::invalid("artifact provenance must be an object"))?;
    let (uri, job_id, kind, name, media_type, size_bytes, content_sha256, metadata) =
        if requested_uri.starts_with("project://") {
            let uri = normalize_project_uri(requested_uri, false)?;
            let row = sqlx::query(
                "SELECT octet_length(content)::bigint AS size_bytes,content_sha256,version \
                 FROM project_files WHERE project_id=$1 AND path=$2",
            )
            .bind(project_id)
            .bind(&uri)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::database)?
            .ok_or_else(ApiError::not_found)?;
            let name = uri.rsplit('/').next().unwrap_or("artifact").to_owned();
            (
                uri,
                None,
                requested_kind.to_owned(),
                name,
                None,
                row.get::<i64, _>("size_bytes"),
                row.get::<String, _>("content_sha256"),
                json!({
                    "source":"governed-project-file",
                    "project_file_version":row.get::<i32,_>("version"),
                    "provenance":provenance
                }),
            )
        } else {
            let parts = requested_uri.split('/').collect::<Vec<_>>();
            if parts.len() != 6
                || parts[0] != "runtime:"
                || !parts[1].is_empty()
                || parts[2] != "jobs"
                || parts[4] != "artifacts"
                || parts[5].is_empty()
            {
                return Err(ApiError::invalid(
                    "runtime artifact uri must be runtime://jobs/{job_id}/artifacts/{artifact_id}",
                ));
            }
            let job_id = parse_runtime_uuid(parts[3], "runtime job id")?;
            let artifact_id = parse_runtime_uuid(parts[5], "runtime artifact id")?;
            let manifest = agent_job_artifacts(state, actor, project_id, job_id)
                .await?
                .into_iter()
                .find(|artifact| {
                    artifact
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse::<Uuid>().ok())
                        == Some(artifact_id)
                })
                .ok_or_else(ApiError::not_found)?;
            let kind = manifest
                .get("kind")
                .and_then(Value::as_str)
                .filter(|value| *value == requested_kind)
                .ok_or_else(|| ApiError::invalid("artifact kind does not match Runtime manifest"))?
                .to_owned();
            let relative_path = manifest
                .get("relative_path")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 512)
                .ok_or_else(|| ApiError::invalid("Runtime artifact path is invalid"))?;
            let name = relative_path
                .rsplit('/')
                .next()
                .unwrap_or("artifact")
                .to_owned();
            let size_bytes = manifest
                .get("size_bytes")
                .and_then(Value::as_i64)
                .filter(|value| *value > 0)
                .ok_or_else(|| ApiError::invalid("Runtime artifact is empty"))?;
            let content_sha256 = manifest
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|value| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(|| ApiError::invalid("Runtime artifact digest is invalid"))?
                .to_ascii_lowercase();
            (
                requested_uri.to_owned(),
                Some(job_id),
                kind,
                name,
                manifest
                    .get("media_type")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                size_bytes,
                content_sha256,
                json!({"source":"runtime-manifest","manifest":manifest,"provenance":provenance}),
            )
        };
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO artifacts(id,project_id,job_id,kind,name,locator,media_type,size_bytes,content_sha256,metadata,created_by_user_id) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(id)
    .bind(project_id)
    .bind(job_id)
    .bind(&kind)
    .bind(&name)
    .bind(&uri)
    .bind(&media_type)
    .bind(size_bytes)
    .bind(&content_sha256)
    .bind(metadata)
    .bind(actor.id)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(json!({
        "id":id,"project_id":project_id,"job_id":job_id,"kind":kind,"name":name,
        "locator":uri,"media_type":media_type,"size_bytes":size_bytes,
        "content_sha256":content_sha256
    }))
}

fn validate_callback_run_id(id: Uuid, value: &Value) -> Result<(), ApiError> {
    let body_id = value
        .get("runId")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok());
    if body_id != Some(id) {
        return Err(ApiError::invalid("run identity mismatch"));
    }
    Ok(())
}

fn parse_runtime_uuid(value: &str, field: &'static str) -> Result<Uuid, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::invalid(format!("{field} must be a UUID")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_uri_scope_is_canonical_and_prefix_bounded() {
        let scopes = vec![
            "project://current/README.md".to_owned(),
            "project://current/results/".to_owned(),
        ];
        assert!(project_scope_allows(
            "project://current/results/table.csv",
            &scopes
        ));
        assert!(project_scope_allows("project://current/README.md", &scopes));
        assert!(!project_scope_allows(
            "project://current/scripts/run.R",
            &scopes
        ));
        assert!(project_scope_intersects("project://current/", &scopes));
        assert!(normalize_project_uri("project://current/results/", true).is_ok());
        for unsafe_uri in [
            "file:///etc/passwd",
            "project://current/../secrets",
            "project://current/results\\escape",
            "project://current/results/data.csv?raw=1",
        ] {
            assert!(normalize_project_uri(unsafe_uri, false).is_err());
        }
        assert_eq!(
            project_resource_artifact_uri(
                "project://current/resources/cohort-a/artifacts/upload-1234"
            ),
            Some(("cohort-a", "upload-1234"))
        );
        for unsupported in [
            "project://current/resources/cohort-a",
            "project://current/resources/cohort-a/artifacts",
            "project://current/resources/cohort-a/artifacts/upload-1234/extra",
            "project://current/uploads/upload-1234",
        ] {
            assert!(project_resource_artifact_uri(unsupported).is_none());
        }
    }

    #[test]
    fn runtime_capability_queries_require_active_users_and_projects() {
        for query in [BOOTSTRAP_RUN_QUERY, CAPABILITY_CONTEXT_QUERY] {
            assert!(query.contains("u.status='active'"));
            assert!(query.contains("p.status='active'"));
            assert!(query.contains("JOIN projects p ON p.id=r.project_id"));
        }
    }

    #[test]
    fn public_resource_discovery_does_not_require_a_project_scope() {
        let definition = TOOLS
            .iter()
            .find(|definition| definition.name == "db.discover_resources")
            .expect("registered discovery tool");
        assert!(!definition.project_required);
    }

    #[test]
    fn provider_payload_omits_absent_optional_api_key() {
        let without_key = provider_payload(
            "ollama".into(),
            "http://ollama.test/v1".into(),
            "qwen3".into(),
            "allow_private".into(),
            None,
        );
        assert!(!without_key.as_object().unwrap().contains_key("apiKey"));

        let with_key = provider_payload(
            "openai".into(),
            "https://api.openai.com/v1".into(),
            "gpt-test".into(),
            "public_only".into(),
            Some("test-key".into()),
        );
        assert_eq!(with_key["apiKey"], "test-key");
    }

    #[test]
    fn environment_plan_is_declarative_and_channel_allowlisted() {
        let project_id = Uuid::nil();
        let result = plan_environment_tool(
            project_id,
            &json!({
                "packages":["r-base>=4.4,<5","bioconductor-seurat"],
                "channels":["conda-forge","bioconda"]
            }),
        )
        .expect("valid plan");
        assert_eq!(result["status"], "planned");
        assert_eq!(result["lock_state"], "not_resolved");
        assert_eq!(result["plan"]["network_policy"], "internet_only");
        assert!(
            plan_environment_tool(
                project_id,
                &json!({"packages":["r-base;touch-host"],"channels":["conda-forge"]})
            )
            .is_err()
        );
        assert!(
            plan_environment_tool(
                project_id,
                &json!({"packages":["r-base"],"channels":["https://example.test/channel"]})
            )
            .is_err()
        );
    }

    #[test]
    fn unskilled_runs_receive_only_core_governed_tools() {
        let names = allowed_tools("admin", "owner", true, &[]);
        assert_eq!(
            names,
            vec![
                "skill.load",
                "plan.propose",
                "plan.update",
                "analysis.validate"
            ]
        );
        let skills = vec![json!({"permissions":{
            "tools":["project.read_file"],
            "projectRead":["project://current/results/"],
            "projectWrite":[],
            "computeProfiles":[]
        }})];
        let names = allowed_tools("user", "viewer", true, &skills);
        assert!(names.contains(&"project.read_file"));
        assert!(!names.contains(&"project.write_file"));
        assert_eq!(
            declared_string_permissions(&skills, "projectRead"),
            vec!["project://current/results/"]
        );

        let personal = allowed_tools("admin", "", false, &skills);
        assert_eq!(personal, vec!["skill.load", "analysis.validate"]);
        assert_eq!(tool_profile("", "admin", false), "global-read");

        let discovery = vec![json!({"permissions":{"tools":[
            "db.discover_resources","db.inspect_resource","db.query_resource","db.get_provenance"
        ]}})];
        assert_eq!(
            allowed_tools("user", "", false, &discovery),
            vec![
                "skill.load",
                "db.discover_resources",
                "db.inspect_resource",
                "db.query_resource",
                "db.get_provenance",
                "analysis.validate"
            ]
        );
    }

    #[test]
    fn backend_evidence_is_run_scoped_and_only_issued_for_observed_results() {
        let run_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let query = backend_evidence(
            run_id,
            Some(project_id),
            "tool-query",
            "db.query_resource",
            &json!({"resource":"cohort-a","operation":"expression"}),
            &json!({"data":{"rows":[{"value":1}]}}),
        );
        assert_eq!(query.len(), 1);
        assert_eq!(query[0]["runId"], run_id.to_string());
        assert_eq!(query[0]["sourceId"], "cohort-a");
        assert_eq!(query[0]["kind"], "query");
        assert_eq!(query[0]["metadata"]["issuer"], "shennong-os");
        assert_eq!(query[0]["metadata"]["operation"], "expression");
        assert!(
            query[0]["digest"]
                .as_str()
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        assert!(
            backend_evidence(
                run_id,
                Some(project_id),
                "tool-job",
                "runtime.get_job",
                &json!({}),
                &json!({"id":Uuid::new_v4(),"status":"running"}),
            )
            .is_empty()
        );
        let job_id = Uuid::new_v4();
        let artifact_id = Uuid::new_v4();
        let runtime = backend_evidence(
            run_id,
            Some(project_id),
            "tool-job-complete",
            "runtime.get_job",
            &json!({}),
            &json!({
                "id":job_id,
                "state":"succeeded",
                "artifacts":[{
                    "id":artifact_id,
                    "relative_path":"results/table.csv",
                    "size_bytes":42,
                    "sha256":"a".repeat(64)
                }]
            }),
        );
        assert_eq!(runtime.len(), 1);
        assert_eq!(runtime[0]["sourceId"], artifact_id.to_string());
        assert_eq!(runtime[0]["digest"], format!("sha256:{}", "a".repeat(64)));
        assert_eq!(runtime[0]["metadata"]["jobId"], job_id.to_string());
        assert!(
            backend_evidence(
                run_id,
                Some(project_id),
                "tool-plan",
                "plan.propose",
                &json!({}),
                &json!({"id":Uuid::new_v4()}),
            )
            .is_empty()
        );
    }
}
