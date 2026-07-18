use super::Envelope;
use crate::{AppState, auth::require_admin, error::ApiError};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    version: &'static str,
    database: &'static str,
}

pub async fn healthz(State(state): State<AppState>) -> Response {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(Health {
                status: "ok",
                version: env!("CARGO_PKG_VERSION"),
                database: "ready",
            }),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "health database probe failed");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"status":"unavailable","version":env!("CARGO_PKG_VERSION"),"database":"unavailable"}))).into_response()
        }
    }
}

pub async fn version() -> Json<Value> {
    Json(json!({"name":"shennong-os","version":env!("CARGO_PKG_VERSION"),"api_version":"v1"}))
}

pub async fn openapi() -> impl IntoResponse {
    (
        [("content-type", "application/yaml; charset=utf-8")],
        include_str!("../../../../openapi/os-api.yaml"),
    )
}

pub async fn dependencies(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let (db, runtime) = tokio::join!(
        async {
            match &state.config.db_client {
                Some(client) => Some(client.info().await),
                None => None,
            }
        },
        async {
            match &state.config.runtime_client {
                Some(client) => Some(client.info().await),
                None => None,
            }
        },
    );
    Ok(Json(Envelope {
        data: json!({"shennong_db":db,"shennong_runtime":runtime}),
    }))
}

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<i64>,
}

pub async fn audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows = sqlx::query("SELECT id,actor_user_id,project_id,action,target_type,target_id,request_id,details,created_at FROM audit_events ORDER BY id DESC LIMIT $1")
        .bind(limit).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    let data = rows.into_iter().map(|row| json!({
        "id":row.get::<i64,_>("id"),"actor_user_id":row.get::<Option<uuid::Uuid>,_>("actor_user_id"),
        "project_id":row.get::<Option<uuid::Uuid>,_>("project_id"),"action":row.get::<String,_>("action"),
        "target_type":row.get::<String,_>("target_type"),"target_id":row.get::<Option<String>,_>("target_id"),
        "request_id":row.get::<Option<uuid::Uuid>,_>("request_id"),"details":row.get::<Value,_>("details"),
        "created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")
    })).collect();
    Ok(Json(Envelope { data }))
}
