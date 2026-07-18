use super::projects::{require_project_read, require_project_write};
use super::{Envelope, audit};
use crate::{
    AppState,
    auth::{authenticate, require_agent_runtime},
    crypto::sha256_hex,
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::HeaderName},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::VecDeque,
    convert::Infallible,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ScopedQuery {
    project_id: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ThreadCreate {
    project_id: Uuid,
    title: Option<String>,
    provider_id: Option<Uuid>,
    scope: Option<String>,
}

#[derive(Deserialize)]
pub struct ThreadUpdate {
    title: Option<String>,
    status: Option<String>,
    provider_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct MessageCreate {
    id: Option<String>,
    run_id: Option<Uuid>,
    role: Option<String>,
    #[serde(alias = "content")]
    content_json: Option<Value>,
    status: Option<String>,
    #[serde(default = "empty_array")]
    attachments: Value,
    #[serde(default = "empty_object")]
    metadata: Value,
}

fn empty_array() -> Value {
    json!([])
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize)]
pub struct RunCreate {
    #[serde(default)]
    input: Value,
}

#[derive(Deserialize)]
pub struct RunUpdate {
    status: String,
    #[serde(default)]
    output: Value,
    #[serde(default)]
    error: Value,
}

#[derive(Deserialize)]
pub struct RunEventCreate {
    event_type: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Deserialize)]
pub struct RunEventsQuery {
    after: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct PlanWrite {
    items: Value,
}

#[derive(Deserialize)]
pub struct JobCreate {
    kind: String,
    run_id: Option<Uuid>,
    #[serde(default)]
    spec: Value,
}

#[derive(Deserialize)]
pub struct JobUpdate {
    status: String,
    #[serde(default)]
    result: Value,
}

#[derive(Deserialize)]
pub struct ArtifactCreate {
    job_id: Option<Uuid>,
    kind: String,
    name: String,
    locator: String,
    media_type: Option<String>,
    size_bytes: Option<i64>,
    content_sha256: Option<String>,
    #[serde(default)]
    metadata: Value,
}

pub async fn list_threads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ScopedQuery>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let rows = if let Some(project_id) = query.project_id {
        require_project_read(&state, &actor, project_id).await?;
        sqlx::query(
            "SELECT * FROM threads WHERE project_id=$1 ORDER BY updated_at DESC,id LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    } else if actor.role == "admin" {
        sqlx::query("SELECT * FROM threads ORDER BY updated_at DESC,id LIMIT $1")
            .bind(limit)
            .fetch_all(&state.pool)
            .await
    } else {
        sqlx::query(
            "SELECT t.* FROM threads t JOIN project_members pm ON pm.project_id=t.project_id \
             WHERE pm.user_id=$1 ORDER BY t.updated_at DESC,t.id LIMIT $2",
        )
        .bind(actor.id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(thread_json).collect(),
    }))
}

pub async fn create_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<ThreadCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, value.project_id).await?;
    if value
        .scope
        .as_deref()
        .is_some_and(|scope| scope != "project")
    {
        return Err(ApiError::invalid("thread scope must be project"));
    }
    let title = validate_title(value.title.as_deref().unwrap_or("New chat"))?;
    if let Some(provider_id) = value.provider_id {
        let owned = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM model_providers WHERE id=$1 AND owner_user_id=$2 AND enabled)")
            .bind(provider_id).bind(actor.id).fetch_one(&state.pool).await.map_err(ApiError::database)?;
        if !owned {
            return Err(ApiError::invalid("model provider is unavailable"));
        }
    }
    let id = Uuid::new_v4();
    let row = sqlx::query("INSERT INTO threads(id,project_id,owner_user_id,provider_id,title) VALUES($1,$2,$3,$4,$5) RETURNING *")
        .bind(id).bind(value.project_id).bind(actor.id).bind(value.provider_id).bind(title)
        .fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(value.project_id),
        "thread.create",
        "thread",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: thread_json(row),
        }),
    ))
}

pub async fn get_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let row = find_thread(&state, id).await?;
    require_project_read(&state, &actor, row.get("project_id")).await?;
    Ok(Json(Envelope {
        data: thread_json(row),
    }))
}

pub async fn update_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<ThreadUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_thread(&state, id).await?;
    let project_id: Uuid = current.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    let title = value.title.as_deref().map(validate_title).transpose()?;
    let status = match value.status.as_deref() {
        Some("regular" | "active") => Some("active"),
        Some("archived") => Some("archived"),
        Some(_) => return Err(ApiError::invalid("invalid thread status")),
        None => None,
    };
    if let Some(provider_id) = value.provider_id {
        let owned = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM model_providers WHERE id=$1 AND owner_user_id=$2 AND enabled)")
            .bind(provider_id).bind(actor.id).fetch_one(&state.pool).await.map_err(ApiError::database)?;
        if !owned {
            return Err(ApiError::invalid("model provider is unavailable"));
        }
    }
    let row = sqlx::query("UPDATE threads SET title=COALESCE($2,title),status=COALESCE($3,status),provider_id=COALESCE($4,provider_id),updated_at=NOW() WHERE id=$1 RETURNING *")
        .bind(id).bind(title).bind(status).bind(value.provider_id).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "thread.update",
        "thread",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(Json(Envelope {
        data: thread_json(row),
    }))
}

pub async fn delete_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_thread(&state, id).await?;
    let project_id: Uuid = current.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    sqlx::query("UPDATE threads SET status='archived',updated_at=NOW() WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "thread.archive",
        "thread",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let thread = find_thread(&state, id).await?;
    require_project_read(&state, &actor, thread.get("project_id")).await?;
    let rows = sqlx::query("SELECT id,thread_id,role,content_json,status,attachments,metadata,created_at FROM messages WHERE thread_id=$1 ORDER BY created_at,id LIMIT 1000")
        .bind(id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(message_json).collect(),
    }))
}

pub async fn get_active_thread_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let thread = find_thread(&state, id).await?;
    let project_id: Uuid = thread.get("project_id");
    require_project_read(&state, &actor, project_id).await?;
    let row = sqlx::query(
        "SELECT r.* FROM runs r JOIN projects p ON p.id=r.project_id \
         WHERE r.thread_id=$1 AND r.project_id=$2 AND p.status='active' \
         AND r.status IN ('queued','running','waiting_approval') \
         ORDER BY r.created_at DESC,r.id DESC LIMIT 1",
    )
    .bind(id)
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let data = if let Some(row) = row {
        let run_id: Uuid = row.get("id");
        let last_cursor: Option<i64> =
            sqlx::query_scalar("SELECT MAX(id) FROM run_events WHERE run_id=$1")
                .bind(run_id)
                .fetch_one(&state.pool)
                .await
                .map_err(ApiError::database)?;
        json!({
            "run": run_json(row),
            "last_event_cursor": last_cursor.map(|cursor| cursor.to_string())
        })
    } else {
        json!({"run":null,"last_event_cursor":null})
    };
    Ok(Json(Envelope { data }))
}

pub async fn create_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<MessageCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    if headers
        .get("x-shennong-service")
        .and_then(|header| header.to_str().ok())
        == Some("agent-runtime")
    {
        require_agent_runtime(&state, &headers)?;
        return create_internal_user_message(&state, id, value).await;
    }
    let actor = authenticate(&state, &headers, true).await?;
    let thread = find_thread(&state, id).await?;
    let project_id: Uuid = thread.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    let key = headers
        .get(HeaderName::from_static("idempotency-key"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| (8..=200).contains(&value.len()))
        .ok_or_else(|| ApiError::invalid("Idempotency-Key header must be 8..200 characters"))?;
    let role = value.role.as_deref().unwrap_or("user");
    if !matches!(role, "user" | "assistant" | "tool" | "system")
        || (role != "user" && actor.role != "admin")
    {
        return Err(ApiError::forbidden());
    }
    let status = value.status.as_deref().unwrap_or("completed");
    if !matches!(status, "pending" | "completed" | "failed" | "cancelled") {
        return Err(ApiError::invalid("invalid message status"));
    }
    let content_json = value
        .content_json
        .ok_or_else(|| ApiError::invalid("content_json is required"))?;
    if content_json.is_null() {
        return Err(ApiError::invalid("content_json is required"));
    }
    if !value.attachments.is_array() || !value.metadata.is_object() {
        return Err(ApiError::invalid(
            "attachments must be an array and metadata must be an object",
        ));
    }
    let message_id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO messages(id,thread_id,role,content_json,status,attachments,metadata,idempotency_key) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8) \
         ON CONFLICT(thread_id,idempotency_key) WHERE idempotency_key IS NOT NULL \
         DO UPDATE SET idempotency_key=messages.idempotency_key \
         RETURNING id,thread_id,role,content_json,status,attachments,metadata,created_at"
    ).bind(message_id).bind(id).bind(role).bind(content_json).bind(status).bind(value.attachments).bind(value.metadata).bind(key)
        .fetch_one(&state.pool).await.map_err(ApiError::database)?;
    let returned_id: Uuid = row.get("id");
    sqlx::query("UPDATE threads SET updated_at=NOW() WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    if returned_id == message_id {
        audit(
            &state,
            Some(&actor),
            Some(project_id),
            "message.create",
            "message",
            Some(message_id.to_string()),
            json!({"thread_id":id}),
        )
        .await?;
    }
    Ok((
        if returned_id == message_id {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(Envelope {
            data: message_json(row),
        }),
    ))
}

async fn create_internal_user_message(
    state: &AppState,
    thread_id: Uuid,
    value: MessageCreate,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let run_id = value
        .run_id
        .ok_or_else(|| ApiError::invalid("run_id is required for an internal message"))?;
    let run = sqlx::query(
        "SELECT project_id,requested_by_user_id FROM runs WHERE id=$1 AND thread_id=$2",
    )
    .bind(run_id)
    .bind(thread_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    if value.role.as_deref().unwrap_or("user") != "user" {
        return Err(ApiError::forbidden());
    }
    let content = value
        .content_json
        .ok_or_else(|| ApiError::invalid("content is required"))?;
    if content.is_null() {
        return Err(ApiError::invalid("content is required"));
    }
    let source_id = value.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let message_id = source_id.parse().unwrap_or_else(|_| Uuid::new_v4());
    let idempotency_key = format!("runtime:{}", sha256_hex(format!("{run_id}\0{source_id}")));
    let metadata = json!({
        "source": "agent-runtime",
        "run_id": run_id,
        "source_message_id": source_id
    });
    let row = sqlx::query(
        "INSERT INTO messages(id,thread_id,role,content_json,status,attachments,metadata,idempotency_key) \
         VALUES($1,$2,'user',$3,'completed','[]'::jsonb,$4,$5) \
         ON CONFLICT(thread_id,idempotency_key) WHERE idempotency_key IS NOT NULL \
         DO UPDATE SET idempotency_key=messages.idempotency_key \
         RETURNING id,thread_id,role,content_json,status,attachments,metadata,created_at",
    )
    .bind(message_id)
    .bind(thread_id)
    .bind(content)
    .bind(metadata)
    .bind(idempotency_key)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let returned_id: Uuid = row.get("id");
    if returned_id == message_id {
        sqlx::query(
            "INSERT INTO audit_events(actor_user_id,project_id,action,target_type,target_id,details) \
             VALUES($1,$2,'message.persist_internal','message',$3,$4)",
        )
        .bind(run.get::<Uuid, _>("requested_by_user_id"))
        .bind(run.get::<Uuid, _>("project_id"))
        .bind(message_id.to_string())
        .bind(json!({"run_id":run_id,"thread_id":thread_id}))
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    }
    Ok((
        if returned_id == message_id {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(Envelope {
            data: message_json(row),
        }),
    ))
}

pub async fn create_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(thread_id): Path<Uuid>,
    Json(value): Json<RunCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let thread = find_thread(&state, thread_id).await?;
    let project_id: Uuid = thread.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    if !value.input.is_object() {
        return Err(ApiError::invalid("run input must be an object"));
    }
    let id = Uuid::new_v4();
    let row = sqlx::query("INSERT INTO runs(id,project_id,thread_id,requested_by_user_id,input) VALUES($1,$2,$3,$4,$5) RETURNING *")
        .bind(id).bind(project_id).bind(thread_id).bind(actor.id).bind(value.input).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "run.create",
        "run",
        Some(id.to_string()),
        json!({"thread_id":thread_id}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: run_json(row),
        }),
    ))
}

pub async fn list_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ScopedQuery>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let rows = if let Some(project_id) = query.project_id {
        require_project_read(&state, &actor, project_id).await?;
        sqlx::query("SELECT * FROM runs WHERE project_id=$1 ORDER BY created_at DESC LIMIT $2")
            .bind(project_id)
            .bind(limit)
            .fetch_all(&state.pool)
            .await
    } else if actor.role == "admin" {
        sqlx::query("SELECT * FROM runs ORDER BY created_at DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&state.pool)
            .await
    } else {
        sqlx::query(
            "SELECT r.* FROM runs r JOIN project_members pm ON pm.project_id=r.project_id \
             WHERE pm.user_id=$1 ORDER BY r.created_at DESC LIMIT $2",
        )
        .bind(actor.id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(run_json).collect(),
    }))
}

pub async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let row = find_run(&state, id).await?;
    require_project_read(&state, &actor, row.get("project_id")).await?;
    Ok(Json(Envelope {
        data: run_json(row),
    }))
}

pub async fn update_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<RunUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_run(&state, id).await?;
    let project_id: Uuid = current.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    if !matches!(
        value.status.as_str(),
        "queued" | "running" | "waiting_approval" | "succeeded" | "failed" | "cancelled"
    ) || !value.output.is_object()
        || !value.error.is_object()
    {
        return Err(ApiError::invalid("invalid run update"));
    }
    let row = sqlx::query("UPDATE runs SET status=$2,output=$3,error=$4,started_at=CASE WHEN $2='running' THEN COALESCE(started_at,NOW()) ELSE started_at END,finished_at=CASE WHEN $2 IN ('succeeded','failed','cancelled') THEN NOW() ELSE NULL END,updated_at=NOW() WHERE id=$1 RETURNING *")
        .bind(id).bind(&value.status).bind(value.output).bind(value.error).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "run.update",
        "run",
        Some(id.to_string()),
        json!({"status":value.status}),
    )
    .await?;
    Ok(Json(Envelope {
        data: run_json(row),
    }))
}

pub async fn list_run_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<RunEventsQuery>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    authorize_run_read(&state, &actor, id).await?;
    let after = resolve_event_cursor(&headers, query.after.as_deref())?;
    validate_event_cursor(&state, id, after).await?;
    let limit = validate_event_limit(query.limit)?;
    let rows = sqlx::query(
        "SELECT id,run_id,event_type,payload,created_at FROM run_events \
         WHERE run_id=$1 AND id>$2 ORDER BY id ASC LIMIT $3",
    )
    .bind(id)
    .bind(after)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope { data: rows.into_iter().map(|row| json!({"id":row.get::<i64,_>("id"),"run_id":row.get::<Uuid,_>("run_id"),"event_type":row.get::<String,_>("event_type"),"payload":row.get::<Value,_>("payload"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")})).collect() }))
}

pub async fn stream_run_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<RunEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    authorize_run_read(&state, &actor, id).await?;
    let after = resolve_event_cursor(&headers, query.after.as_deref())?;
    validate_event_cursor(&state, id, after).await?;
    let batch_limit = validate_event_limit(query.limit)?.min(500);
    let replay = RunEventReplay {
        pool: state.pool.clone(),
        run_id: id,
        cursor: after,
        batch_limit,
        pending: VecDeque::new(),
        terminal_seen: false,
        actor_id: actor.id,
        session_id: actor.session_id,
        last_authorization_check: Instant::now(),
    };
    let stream = stream::unfold(replay, |mut replay| async move {
        loop {
            if let Some(item) = replay.pending.pop_front() {
                if item.terminal {
                    replay.terminal_seen = true;
                }
                let event = Event::default()
                    .id(item.id.to_string())
                    .data(item.payload.to_string());
                return Some((Ok(event), replay));
            }
            if replay.terminal_seen {
                return None;
            }
            if replay.last_authorization_check.elapsed() >= Duration::from_secs(2) {
                match replay_authorized(
                    &replay.pool,
                    replay.run_id,
                    replay.actor_id,
                    replay.session_id,
                )
                .await
                {
                    Ok(true) => replay.last_authorization_check = Instant::now(),
                    Ok(false) => return None,
                    Err(error) => {
                        tracing::warn!(%error, run_id=%replay.run_id, "run event authorization recheck failed");
                        return None;
                    }
                }
            }

            match sqlx::query(
                "SELECT id,event_type,payload FROM run_events \
                 WHERE run_id=$1 AND id>$2 ORDER BY id ASC LIMIT $3",
            )
            .bind(replay.run_id)
            .bind(replay.cursor)
            .bind(replay.batch_limit)
            .fetch_all(&replay.pool)
            .await
            {
                Ok(rows) if !rows.is_empty() => {
                    for row in rows {
                        let id: i64 = row.get("id");
                        let event_type: String = row.get("event_type");
                        let payload: Value = row.get("payload");
                        replay.cursor = id;
                        replay.pending.push_back(ReplayEvent {
                            id,
                            payload,
                            terminal: is_terminal_run_event(&event_type),
                        });
                        if is_terminal_run_event(&event_type) {
                            break;
                        }
                    }
                    continue;
                }
                Ok(_) => {
                    let status =
                        sqlx::query_scalar::<_, String>("SELECT status FROM runs WHERE id=$1")
                            .bind(replay.run_id)
                            .fetch_optional(&replay.pool)
                            .await;
                    match status {
                        Ok(Some(status)) if is_terminal_run_status(&status) => return None,
                        Ok(None) => return None,
                        Ok(Some(_)) => {}
                        Err(error) => {
                            tracing::warn!(%error, run_id=%replay.run_id, "run event status poll failed");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, run_id=%replay.run_id, "run event replay poll failed");
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

struct RunEventReplay {
    pool: sqlx::PgPool,
    run_id: Uuid,
    cursor: i64,
    batch_limit: i64,
    pending: VecDeque<ReplayEvent>,
    terminal_seen: bool,
    actor_id: Uuid,
    session_id: Uuid,
    last_authorization_check: Instant,
}

struct ReplayEvent {
    id: i64,
    payload: Value,
    terminal: bool,
}

fn validate_event_limit(limit: Option<i64>) -> Result<i64, ApiError> {
    let limit = limit.unwrap_or(500);
    if !(1..=1000).contains(&limit) {
        return Err(ApiError::invalid("limit must be between 1 and 1000"));
    }
    Ok(limit)
}

fn resolve_event_cursor(headers: &HeaderMap, query_after: Option<&str>) -> Result<i64, ApiError> {
    let header_after = headers
        .get("last-event-id")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::invalid("Last-Event-ID must be an event cursor"))
        })
        .transpose()?;
    let query_cursor = query_after.map(parse_event_cursor).transpose()?;
    let header_cursor = header_after.map(parse_event_cursor).transpose()?;
    if let (Some(query_cursor), Some(header_cursor)) = (query_cursor, header_cursor)
        && query_cursor != header_cursor
    {
        return Err(ApiError::invalid(
            "after and Last-Event-ID must identify the same event",
        ));
    }
    Ok(query_cursor.or(header_cursor).unwrap_or(0))
}

fn parse_event_cursor(value: &str) -> Result<i64, ApiError> {
    if value == "0" {
        return Ok(0);
    }
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ApiError::invalid(
            "event cursor must be a canonical non-negative integer",
        ));
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|cursor| *cursor > 0)
        .ok_or_else(|| ApiError::invalid("event cursor is out of range"))
}

async fn validate_event_cursor(
    state: &AppState,
    run_id: Uuid,
    cursor: i64,
) -> Result<(), ApiError> {
    if cursor == 0 {
        return Ok(());
    }
    let belongs_to_run: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_events WHERE id=$1 AND run_id=$2)")
            .bind(cursor)
            .bind(run_id)
            .fetch_one(&state.pool)
            .await
            .map_err(ApiError::database)?;
    if !belongs_to_run {
        return Err(ApiError::invalid(
            "event cursor does not belong to this run",
        ));
    }
    Ok(())
}

async fn authorize_run_read(
    state: &AppState,
    actor: &crate::auth::AuthUser,
    run_id: Uuid,
) -> Result<(), ApiError> {
    let project_id: Uuid = sqlx::query_scalar(
        "SELECT r.project_id FROM runs r JOIN projects p ON p.id=r.project_id \
         WHERE r.id=$1 AND p.status='active'",
    )
    .bind(run_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    require_project_read(state, actor, project_id).await?;
    Ok(())
}

async fn replay_authorized(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    actor_id: Uuid,
    session_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id \
           JOIN runs r ON r.id=$1 JOIN projects p ON p.id=r.project_id \
           LEFT JOIN project_members pm ON pm.project_id=p.id AND pm.user_id=u.id \
           WHERE s.id=$2 AND u.id=$3 AND s.revoked_at IS NULL AND s.expires_at>NOW() \
             AND u.status='active' AND p.status='active' \
             AND (u.role='admin' OR pm.user_id=u.id) \
         )",
    )
    .bind(run_id)
    .bind(session_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
}

fn is_terminal_run_event(event_type: &str) -> bool {
    matches!(event_type, "RUN_FINISHED" | "RUN_ERROR" | "RUN_CANCELLED")
}

fn is_terminal_run_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "failed_validation" | "cancelled"
    )
}

pub async fn create_run_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<RunEventCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let run = find_run(&state, id).await?;
    let project_id: Uuid = run.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    if value.event_type.trim().is_empty()
        || value.event_type.len() > 128
        || !value.payload.is_object()
    {
        return Err(ApiError::invalid("invalid run event"));
    }
    let row = sqlx::query("INSERT INTO run_events(run_id,event_type,payload) VALUES($1,$2,$3) RETURNING id,run_id,event_type,payload,created_at")
        .bind(id).bind(value.event_type.trim()).bind(value.payload).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"id":row.get::<i64,_>("id"),"run_id":id,"event_type":row.get::<String,_>("event_type"),"payload":row.get::<Value,_>("payload"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")}),
        }),
    ))
}

pub async fn get_task_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let run = find_run(&state, id).await?;
    require_project_read(&state, &actor, run.get("project_id")).await?;
    let row = sqlx::query("SELECT run_id,version,items,updated_at FROM task_plans WHERE run_id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(Envelope {
        data: json!({"run_id":id,"version":row.get::<i32,_>("version"),"items":row.get::<Value,_>("items"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")}),
    }))
}

pub async fn put_task_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<PlanWrite>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let run = find_run(&state, id).await?;
    let project_id: Uuid = run.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    if !value.items.is_array()
        || value
            .items
            .as_array()
            .is_some_and(|items| items.len() > 200)
    {
        return Err(ApiError::invalid(
            "task plan items must be an array of at most 200 items",
        ));
    }
    let row = sqlx::query("INSERT INTO task_plans(run_id,items) VALUES($1,$2) ON CONFLICT(run_id) DO UPDATE SET version=task_plans.version+1,items=EXCLUDED.items,updated_at=NOW() RETURNING run_id,version,items,updated_at")
        .bind(id).bind(value.items).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "task_plan.update",
        "run",
        Some(id.to_string()),
        json!({"version":row.get::<i32,_>("version")}),
    )
    .await?;
    Ok(Json(Envelope {
        data: json!({"run_id":id,"version":row.get::<i32,_>("version"),"items":row.get::<Value,_>("items"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")}),
    }))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, project_id).await?;
    let rows =
        sqlx::query("SELECT * FROM jobs WHERE project_id=$1 ORDER BY created_at DESC LIMIT 500")
            .bind(project_id)
            .fetch_all(&state.pool)
            .await
            .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(job_json).collect(),
    }))
}

pub async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(value): Json<JobCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, project_id).await?;
    if value.kind.trim().is_empty() || value.kind.len() > 128 || !value.spec.is_object() {
        return Err(ApiError::invalid("invalid job"));
    }
    let id = Uuid::new_v4();
    let row = sqlx::query("INSERT INTO jobs(id,project_id,run_id,kind,spec,created_by_user_id) VALUES($1,$2,$3,$4,$5,$6) RETURNING *")
        .bind(id).bind(project_id).bind(value.run_id).bind(value.kind.trim()).bind(value.spec).bind(actor.id).fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "job.create",
        "job",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: job_json(row),
        }),
    ))
}

pub async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let row = find_job(&state, id).await?;
    require_project_read(&state, &actor, row.get("project_id")).await?;
    Ok(Json(Envelope {
        data: job_json(row),
    }))
}

pub async fn update_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<JobUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_job(&state, id).await?;
    let project_id: Uuid = current.get("project_id");
    require_project_write(&state, &actor, project_id).await?;
    if !matches!(
        value.status.as_str(),
        "queued" | "running" | "succeeded" | "failed" | "cancelled"
    ) || !value.result.is_object()
    {
        return Err(ApiError::invalid("invalid job update"));
    }
    let row =
        sqlx::query("UPDATE jobs SET status=$2,result=$3,updated_at=NOW() WHERE id=$1 RETURNING *")
            .bind(id)
            .bind(&value.status)
            .bind(value.result)
            .fetch_one(&state.pool)
            .await
            .map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "job.update",
        "job",
        Some(id.to_string()),
        json!({"status":value.status}),
    )
    .await?;
    Ok(Json(Envelope {
        data: job_json(row),
    }))
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, project_id).await?;
    let rows = sqlx::query(
        "SELECT * FROM artifacts WHERE project_id=$1 ORDER BY created_at DESC LIMIT 1000",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(artifact_json).collect(),
    }))
}

pub async fn create_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(value): Json<ArtifactCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_write(&state, &actor, project_id).await?;
    if value.kind.trim().is_empty()
        || value.kind.len() > 128
        || value.name.trim().is_empty()
        || value.name.len() > 256
        || value.locator.is_empty()
        || value.locator.len() > 4096
        || !value.metadata.is_object()
        || value.size_bytes.is_some_and(|size| size < 0)
        || value.content_sha256.as_deref().is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|value| value.is_ascii_hexdigit())
        })
    {
        return Err(ApiError::invalid("invalid artifact"));
    }
    let id = Uuid::new_v4();
    let row = sqlx::query("INSERT INTO artifacts(id,project_id,job_id,kind,name,locator,media_type,size_bytes,content_sha256,metadata,created_by_user_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *")
        .bind(id).bind(project_id).bind(value.job_id).bind(value.kind.trim()).bind(value.name.trim()).bind(value.locator).bind(value.media_type).bind(value.size_bytes).bind(value.content_sha256.map(|hash| hash.to_ascii_lowercase())).bind(value.metadata).bind(actor.id)
        .fetch_one(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "artifact.create",
        "artifact",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: artifact_json(row),
        }),
    ))
}

async fn find_thread(state: &AppState, id: Uuid) -> Result<sqlx::postgres::PgRow, ApiError> {
    sqlx::query("SELECT * FROM threads WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)
}
async fn find_run(state: &AppState, id: Uuid) -> Result<sqlx::postgres::PgRow, ApiError> {
    sqlx::query("SELECT * FROM runs WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)
}
async fn find_job(state: &AppState, id: Uuid) -> Result<sqlx::postgres::PgRow, ApiError> {
    sqlx::query("SELECT * FROM jobs WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)
}
fn validate_title(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 {
        Err(ApiError::invalid("thread title must be 1..200 characters"))
    } else {
        Ok(value.into())
    }
}

fn thread_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),"scope":"project","owner_user_id":row.get::<Uuid,_>("owner_user_id"),"provider_id":row.get::<Option<Uuid>,_>("provider_id"),"title":row.get::<String,_>("title"),"status":row.get::<String,_>("status"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
fn message_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"thread_id":row.get::<Uuid,_>("thread_id"),"role":row.get::<String,_>("role"),"content_json":row.get::<Value,_>("content_json"),"status":row.get::<String,_>("status"),"attachments":row.get::<Value,_>("attachments"),"metadata":row.get::<Value,_>("metadata"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")})
}
fn run_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),"thread_id":row.get::<Uuid,_>("thread_id"),"requested_by_user_id":row.get::<Uuid,_>("requested_by_user_id"),"status":row.get::<String,_>("status"),"input":row.get::<Value,_>("input"),"output":row.get::<Value,_>("output"),"error":row.get::<Value,_>("error"),"started_at":row.get::<Option<chrono::DateTime<chrono::Utc>>,_>("started_at"),"finished_at":row.get::<Option<chrono::DateTime<chrono::Utc>>,_>("finished_at"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
fn job_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),"run_id":row.get::<Option<Uuid>,_>("run_id"),"kind":row.get::<String,_>("kind"),"status":row.get::<String,_>("status"),"spec":row.get::<Value,_>("spec"),"result":row.get::<Value,_>("result"),"created_by_user_id":row.get::<Uuid,_>("created_by_user_id"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
fn artifact_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"project_id":row.get::<Uuid,_>("project_id"),"job_id":row.get::<Option<Uuid>,_>("job_id"),"kind":row.get::<String,_>("kind"),"name":row.get::<String,_>("name"),"locator":row.get::<String,_>("locator"),"media_type":row.get::<Option<String>,_>("media_type"),"size_bytes":row.get::<Option<i64>,_>("size_bytes"),"content_sha256":row.get::<Option<String>,_>("content_sha256"),"metadata":row.get::<Value,_>("metadata"),"created_by_user_id":row.get::<Uuid,_>("created_by_user_id"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_cursor_is_canonical_and_query_header_must_agree() {
        assert_eq!(parse_event_cursor("0").expect("zero cursor"), 0);
        assert_eq!(parse_event_cursor("42").expect("positive cursor"), 42);
        for invalid in ["", "00", "01", "-1", "+1", " 1", "1 ", "1.0"] {
            assert_eq!(
                parse_event_cursor(invalid)
                    .expect_err("invalid cursor")
                    .status,
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }

        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "42".parse().expect("header"));
        assert_eq!(
            resolve_event_cursor(&headers, Some("42")).expect("matching cursors"),
            42
        );
        assert_eq!(
            resolve_event_cursor(&headers, Some("43"))
                .expect_err("mismatched cursors")
                .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn event_replay_limits_and_terminal_markers_are_bounded() {
        assert_eq!(validate_event_limit(None).expect("default limit"), 500);
        assert_eq!(
            validate_event_limit(Some(1000)).expect("maximum limit"),
            1000
        );
        assert!(validate_event_limit(Some(0)).is_err());
        assert!(validate_event_limit(Some(1001)).is_err());
        assert!(is_terminal_run_event("RUN_FINISHED"));
        assert!(is_terminal_run_event("RUN_ERROR"));
        assert!(!is_terminal_run_event("TEXT_MESSAGE_CONTENT"));
        assert!(is_terminal_run_status("failed_validation"));
        assert!(!is_terminal_run_status("running"));
    }
}
