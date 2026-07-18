use super::{Envelope, audit_tx};
use crate::{AppState, auth::require_admin, crypto::normalize_email, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct AdminUserUpdate {
    display_name: Option<String>,
    email: Option<String>,
    role: Option<String>,
    status: Option<String>,
}

pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let row = sqlx::query(
        "SELECT \
         (SELECT COUNT(*) FROM users) AS users, \
         (SELECT COUNT(*) FROM users WHERE status='active') AS active_users, \
         (SELECT COUNT(*) FROM projects) AS projects, \
         (SELECT COUNT(*) FROM threads) AS threads, \
         (SELECT COUNT(*) FROM runs) AS runs, \
         (SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','waiting_approval')) AS active_runs, \
         (SELECT COUNT(*) FROM model_providers) AS model_providers, \
         (SELECT COUNT(*) FROM model_providers WHERE enabled=TRUE) AS enabled_model_providers, \
         (SELECT MAX(created_at) FROM audit_events) AS last_audit_at",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: json!({
            "users": row.get::<i64,_>("users"),
            "active_users": row.get::<i64,_>("active_users"),
            "projects": row.get::<i64,_>("projects"),
            "threads": row.get::<i64,_>("threads"),
            "runs": row.get::<i64,_>("runs"),
            "active_runs": row.get::<i64,_>("active_runs"),
            "model_providers": row.get::<i64,_>("model_providers"),
            "enabled_model_providers": row.get::<i64,_>("enabled_model_providers"),
            "last_audit_at": row.get::<Option<chrono::DateTime<chrono::Utc>>,_>("last_audit_at"),
        }),
    }))
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let rows = sqlx::query(
        "SELECT u.id,u.email,u.display_name,u.role,u.status,u.created_at,u.updated_at, \
         (SELECT COUNT(*) FROM sessions s WHERE s.user_id=u.id AND s.revoked_at IS NULL AND s.expires_at>NOW()) AS active_sessions, \
         (SELECT COUNT(*) FROM projects p WHERE p.owner_user_id=u.id) AS owned_projects, \
         (SELECT COUNT(*) FROM model_providers mp WHERE mp.owner_user_id=u.id AND mp.enabled=TRUE) AS enabled_providers \
         FROM users u ORDER BY u.updated_at DESC LIMIT 1000",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(user_json).collect(),
    }))
}

pub async fn get_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let row = sqlx::query(
        "SELECT u.id,u.email,u.display_name,u.role,u.status,u.created_at,u.updated_at, \
         (SELECT COUNT(*) FROM sessions s WHERE s.user_id=u.id AND s.revoked_at IS NULL AND s.expires_at>NOW()) AS active_sessions, \
         (SELECT COUNT(*) FROM projects p WHERE p.owner_user_id=u.id) AS owned_projects, \
         (SELECT COUNT(*) FROM model_providers mp WHERE mp.owner_user_id=u.id AND mp.enabled=TRUE) AS enabled_providers \
         FROM users u WHERE u.id=$1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    Ok(Json(Envelope {
        data: user_json(row),
    }))
}

pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<AdminUserUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = require_admin(&state, &headers, true).await?;
    if value
        .role
        .as_deref()
        .is_some_and(|role| !matches!(role, "admin" | "user"))
    {
        return Err(ApiError::invalid("role must be admin or user"));
    }
    if value
        .status
        .as_deref()
        .is_some_and(|status| !matches!(status, "active" | "disabled"))
    {
        return Err(ApiError::invalid("status must be active or disabled"));
    }
    let display_name = value.display_name.as_deref().map(str::trim);
    if display_name.is_some_and(|name| name.is_empty() || name.len() > 128) {
        return Err(ApiError::invalid("display name must be 1..128 characters"));
    }
    let email = value.email.as_deref().map(normalize_email).transpose()?;
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let current = sqlx::query("SELECT role,status FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    let current_role: String = current.get("role");
    let current_status: String = current.get("status");
    let next_role = value.role.as_deref().unwrap_or(&current_role);
    let next_status = value.status.as_deref().unwrap_or(&current_status);
    if current_role == "admin"
        && current_status == "active"
        && (next_role != "admin" || next_status != "active")
    {
        let active_admins =
            sqlx::query("SELECT id FROM users WHERE role='admin' AND status='active' FOR UPDATE")
                .fetch_all(&mut *tx)
                .await
                .map_err(ApiError::database)?;
        if active_admins.len() <= 1 {
            return Err(ApiError::invalid(
                "the last active administrator cannot be disabled or demoted",
            ));
        }
    }
    let row = sqlx::query(
        "UPDATE users SET display_name=COALESCE($2,display_name),email=COALESCE($3,email),email_normalized=COALESCE($3,email_normalized),role=COALESCE($4,role),status=COALESCE($5,status),updated_at=NOW() WHERE id=$1 \
         RETURNING id,email,display_name,role,status,created_at,updated_at,0::BIGINT AS active_sessions,0::BIGINT AS owned_projects,0::BIGINT AS enabled_providers",
    )
    .bind(id).bind(display_name).bind(email).bind(&value.role).bind(&value.status)
    .fetch_one(&mut *tx).await.map_err(ApiError::database)?;
    if next_status == "disabled" {
        sqlx::query("UPDATE sessions SET revoked_at=COALESCE(revoked_at,NOW()) WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(id).execute(&mut *tx).await.map_err(ApiError::database)?;
    }
    audit_tx(
        &mut tx,
        Some(actor.id),
        None,
        "admin.user.update",
        "user",
        Some(id.to_string()),
        json!({"role":next_role,"status":next_status}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: user_json(row),
    }))
}

pub async fn list_model_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let rows = sqlx::query(
        "SELECT p.id,p.owner_user_id,u.display_name AS owner_name,p.name,p.provider_kind,p.base_url,p.model,p.data_policy, \
         p.encrypted_api_key IS NOT NULL AS has_api_key,p.enabled,p.is_default,p.created_at,p.updated_at \
         FROM model_providers p JOIN users u ON u.id=p.owner_user_id ORDER BY p.updated_at DESC LIMIT 1000",
    ).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope { data: rows.into_iter().map(|row| json!({
        "id":row.get::<Uuid,_>("id"),"owner_user_id":row.get::<Uuid,_>("owner_user_id"),"owner_name":row.get::<String,_>("owner_name"),
        "name":row.get::<String,_>("name"),"provider_kind":row.get::<String,_>("provider_kind"),"base_url":row.get::<String,_>("base_url"),
        "model":row.get::<String,_>("model"),"data_policy":row.get::<String,_>("data_policy"),"has_api_key":row.get::<bool,_>("has_api_key"),
        "enabled":row.get::<bool,_>("enabled"),"is_default":row.get::<bool,_>("is_default"),
        "created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")
    })).collect() }))
}

fn user_json(row: sqlx::postgres::PgRow) -> Value {
    json!({
        "id":row.get::<Uuid,_>("id"),"email":row.get::<String,_>("email"),"display_name":row.get::<String,_>("display_name"),
        "role":row.get::<String,_>("role"),"status":row.get::<String,_>("status"),
        "active_sessions":row.get::<i64,_>("active_sessions"),"owned_projects":row.get::<i64,_>("owned_projects"),
        "enabled_providers":row.get::<i64,_>("enabled_providers"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),
        "updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")
    })
}
