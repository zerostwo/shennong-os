pub mod agent;
pub mod authentication;
pub mod context;
pub mod data_plane;
pub mod integration;
pub mod projects;
pub mod runtime_control;
pub mod system;

use crate::{AppState, auth::AuthUser, crypto::hmac_sha256, error::ApiError};
use axum::http::HeaderMap;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Serialize)]
pub struct Envelope<T> {
    pub data: T,
}

pub async fn audit(
    state: &AppState,
    actor: Option<&AuthUser>,
    project_id: Option<Uuid>,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    details: Value,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_events(actor_user_id,project_id,action,target_type,target_id,details) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(actor.map(|value| value.id)).bind(project_id).bind(action).bind(target_type).bind(target_id).bind(details)
        .execute(&state.pool).await.map_err(ApiError::database)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn audit_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor_user_id: Option<Uuid>,
    project_id: Option<Uuid>,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    details: Value,
    ip_hash: Option<Vec<u8>>,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_events(actor_user_id,project_id,action,target_type,target_id,details,ip_hash) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(actor_user_id).bind(project_id).bind(action).bind(target_type).bind(target_id).bind(details).bind(ip_hash)
        .execute(&mut **tx).await.map_err(ApiError::database)?;
    Ok(())
}

pub fn ip_hash(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<Vec<u8>> {
    crate::auth::client_ip(state, headers, peer)
        .map(|value| hmac_sha256(&state.config.invite_hmac_key, &value))
}
