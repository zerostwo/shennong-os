use super::{Envelope, audit, audit_tx, ip_hash};
use crate::{
    AppState,
    auth::{
        append_expired_cookies, append_session_cookies, authenticate, enforce_origin,
        issue_session, require_admin,
    },
    crypto::{
        constant_time_secret_eq, hash_password, hmac_sha256, normalize_email, random_secret,
        verify_password,
    },
    error::ApiError,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct AccountRequest {
    display_name: String,
    email: String,
    password: String,
    invite_code: Option<String>,
}

#[derive(Deserialize)]
pub struct SignInRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub struct ProfileWrite {
    display_name: String,
    username: String,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
pub struct InviteCreate {
    email_constraint: Option<String>,
    max_uses: Option<i32>,
    expires_in_seconds: Option<i64>,
    note: Option<String>,
}

#[derive(Deserialize)]
pub struct RegistrationPolicyWrite {
    registration_mode: String,
}

pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let needs_setup = !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(&state.pool)
        .await
        .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: json!({"needs_setup":needs_setup}),
    }))
}

pub async fn registration_policy(
    State(state): State<AppState>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let mode = sqlx::query_scalar::<_, String>(
        "SELECT registration_mode FROM os_settings WHERE singleton=TRUE",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: json!({"registration_mode":mode,"invite_required":mode=="invite_only"}),
    }))
}

pub async fn update_registration_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<RegistrationPolicyWrite>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = require_admin(&state, &headers, true).await?;
    if !matches!(
        value.registration_mode.as_str(),
        "disabled" | "invite_only" | "open"
    ) {
        return Err(ApiError::invalid("invalid registration mode"));
    }
    sqlx::query(
        "UPDATE os_settings SET registration_mode=$1,updated_at=NOW() WHERE singleton=TRUE",
    )
    .bind(&value.registration_mode)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        None,
        "registration_policy.update",
        "os_settings",
        Some("registration".into()),
        json!({"registration_mode":value.registration_mode}),
    )
    .await?;
    Ok(Json(Envelope {
        data: json!({"registration_mode":value.registration_mode,"invite_required":value.registration_mode=="invite_only"}),
    }))
}

pub async fn setup_admin(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(value): Json<AccountRequest>,
) -> Result<Response, ApiError> {
    enforce_origin(&state, &headers)?;
    rate_auth(&state, &headers, Some(peer), "bootstrap").await?;
    let provided = headers
        .get("x-shennong-bootstrap-token")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "bootstrap_token_required",
                "bootstrap token is required",
            )
        })?;
    if !constant_time_secret_eq(provided, &state.config.bootstrap_token) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "bootstrap_token_invalid",
            "bootstrap token is invalid",
        ));
    }
    let display_name = validate_display_name(&value.display_name)?;
    let email = normalize_email(&value.email)?;
    let password = value.password;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "password_hash_failed",
                "password could not be secured",
            )
        })??;
    let user_id = Uuid::new_v4();
    let peer_addr = Some(peer);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(8_613_764_269_i64)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?
    {
        tx.rollback().await.map_err(ApiError::database)?;
        return Err(ApiError::conflict("instance is already configured"));
    }
    let username = generated_username(user_id);
    sqlx::query("INSERT INTO users(id,email,email_normalized,display_name,username,password_hash,role,status) VALUES($1,$2,$3,$4,$5,$6,'admin','active')")
        .bind(user_id).bind(&email).bind(&email).bind(&display_name).bind(&username).bind(password_hash)
        .execute(&mut *tx).await.map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(user_id),
        None,
        "setup.admin_created",
        "user",
        Some(user_id.to_string()),
        json!({"role":"admin"}),
        ip_hash(&state, &headers, peer_addr),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    let session = issue_session(&state, user_id, &headers, peer_addr).await?;
    session_response(
        &state,
        StatusCode::CREATED,
        json!({"id":user_id,"email":email,"display_name":display_name,"username":username,"avatar_url":Value::Null,"role":"admin","csrf_token":session.csrf}),
        &session,
    )
}

pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(value): Json<AccountRequest>,
) -> Result<Response, ApiError> {
    enforce_origin(&state, &headers)?;
    rate_auth(&state, &headers, Some(peer), "register").await?;
    let display_name = validate_display_name(&value.display_name)?;
    let email = normalize_email(&value.email)?;
    let mode = sqlx::query_scalar::<_, String>(
        "SELECT registration_mode FROM os_settings WHERE singleton=TRUE",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if mode == "disabled" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "registration_disabled",
            "registration is unavailable",
        ));
    }
    let invite_hash = if mode == "invite_only" {
        let code = value
            .invite_code
            .as_deref()
            .filter(|code| (12..=256).contains(&code.len()))
            .ok_or_else(invite_error)?;
        Some(hmac_sha256(&state.config.invite_hmac_key, code))
    } else {
        None
    };
    let password = value.password;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "password_hash_failed",
                "password could not be secured",
            )
        })??;
    let user_id = Uuid::new_v4();
    let peer_addr = Some(peer);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let invite_id = if let Some(code_hash) = invite_hash {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM registration_invites WHERE code_hash=$1 AND revoked_at IS NULL \
             AND expires_at>NOW() AND use_count<max_uses \
             AND (email_constraint IS NULL OR email_constraint=$2) FOR UPDATE",
        )
        .bind(code_hash)
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(invite_error)
        .map(Some)?
    } else {
        None
    };
    let username = generated_username(user_id);
    let inserted = sqlx::query("INSERT INTO users(id,email,email_normalized,display_name,username,password_hash,role,status) VALUES($1,$2,$3,$4,$5,$6,'user','active')")
        .bind(user_id).bind(&email).bind(&email).bind(&display_name).bind(&username).bind(password_hash).execute(&mut *tx).await;
    if let Err(error) = inserted {
        let duplicate = error
            .as_database_error()
            .and_then(|value| value.code())
            .as_deref()
            == Some("23505");
        tx.rollback().await.map_err(ApiError::database)?;
        return Err(if duplicate {
            ApiError::conflict("account is already registered")
        } else {
            ApiError::database(error)
        });
    }
    if let Some(invite_id) = invite_id {
        let updated = sqlx::query("UPDATE registration_invites SET use_count=use_count+1 WHERE id=$1 AND revoked_at IS NULL AND expires_at>NOW() AND use_count<max_uses")
            .bind(invite_id).execute(&mut *tx).await.map_err(ApiError::database)?;
        if updated.rows_affected() != 1 {
            tx.rollback().await.map_err(ApiError::database)?;
            return Err(invite_error());
        }
        sqlx::query("INSERT INTO registration_invite_redemptions(invite_id,user_id,normalized_email,ip_hash) VALUES($1,$2,$3,$4)")
            .bind(invite_id).bind(user_id).bind(&email).bind(ip_hash(&state, &headers, peer_addr))
            .execute(&mut *tx).await.map_err(ApiError::database)?;
    }
    audit_tx(
        &mut tx,
        Some(user_id),
        None,
        "auth.register",
        "user",
        Some(user_id.to_string()),
        json!({"registration_mode":mode}),
        ip_hash(&state, &headers, peer_addr),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    let session = issue_session(&state, user_id, &headers, peer_addr).await?;
    session_response(
        &state,
        StatusCode::CREATED,
        json!({"id":user_id,"email":email,"display_name":display_name,"username":username,"avatar_url":Value::Null,"role":"user","csrf_token":session.csrf}),
        &session,
    )
}

pub async fn sign_in(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(value): Json<SignInRequest>,
) -> Result<Response, ApiError> {
    enforce_origin(&state, &headers)?;
    rate_auth(&state, &headers, Some(peer), "sign-in").await?;
    let email = normalize_email(&value.email).map_err(|_| invalid_credentials())?;
    if value.password.len() > 1024 {
        return Err(invalid_credentials());
    }
    let row = sqlx::query("SELECT id,email,display_name,username,avatar_url,role,status,password_hash FROM users WHERE email_normalized=$1")
        .bind(&email).fetch_optional(&state.pool).await.map_err(ApiError::database)?;
    let encoded = row.as_ref().map(|row| row.get::<String,_>("password_hash"))
        .unwrap_or_else(|| "$argon2id$v=19$m=19456,t=2,p=1$MDEyMzQ1Njc4OWFiY2RlZg$qy9jBN9aUkHNvRijRT88Yn2m2QG1d2HfPvYxGfXsJOI".into());
    let password = value.password;
    let valid = tokio::task::spawn_blocking(move || verify_password(&password, &encoded))
        .await
        .unwrap_or(false);
    let row = row.filter(|row| row.get::<String, _>("status") == "active");
    if !valid || row.is_none() {
        return Err(invalid_credentials());
    }
    let row = row.expect("checked");
    let user_id: Uuid = row.get("id");
    let peer_addr = Some(peer);
    let session = issue_session(&state, user_id, &headers, peer_addr).await?;
    audit(
        &state,
        None,
        None,
        "auth.sign_in",
        "user",
        Some(user_id.to_string()),
        json!({}),
    )
    .await?;
    session_response(
        &state,
        StatusCode::OK,
        json!({"id":user_id,"email":row.get::<String,_>("email"),"display_name":row.get::<String,_>("display_name"),"username":row.get::<String,_>("username"),"avatar_url":row.get::<Option<String>,_>("avatar_url"),"role":row.get::<String,_>("role"),"csrf_token":session.csrf}),
        &session,
    )
}

pub async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = authenticate(&state, &headers, true).await?;
    sqlx::query("UPDATE sessions SET revoked_at=NOW() WHERE id=$1 AND revoked_at IS NULL")
        .bind(user.session_id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    audit(
        &state,
        Some(&user),
        None,
        "auth.sign_out",
        "session",
        Some(user.session_id.to_string()),
        json!({}),
    )
    .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    append_expired_cookies(response.headers_mut(), &state)?;
    Ok(response)
}

pub async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Value>>, ApiError> {
    match authenticate(&state, &headers, false).await {
        Ok(user) => Ok(Json(Envelope {
            data: json!({"authenticated":true,"user":user}),
        })),
        Err(error) if error.status == StatusCode::UNAUTHORIZED => Ok(Json(Envelope {
            data: json!({"authenticated":false}),
        })),
        Err(error) => Err(error),
    }
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<ProfileWrite>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let display_name = validate_display_name(&value.display_name)?;
    let username = validate_username(&value.username)?;
    let avatar_url = validate_avatar_url(value.avatar_url)?;
    let updated = sqlx::query(
        "UPDATE users SET display_name=$2,username=$3,avatar_url=$4,updated_at=NOW() \
         WHERE id=$1 RETURNING email,role",
    )
    .bind(actor.id)
    .bind(&display_name)
    .bind(&username)
    .bind(&avatar_url)
    .fetch_one(&state.pool)
    .await;
    let row = match updated {
        Ok(row) => row,
        Err(error)
            if error
                .as_database_error()
                .and_then(|value| value.code())
                .as_deref()
                == Some("23505") =>
        {
            return Err(ApiError::conflict("username is already in use"));
        }
        Err(error) => return Err(ApiError::database(error)),
    };
    audit(
        &state,
        Some(&actor),
        None,
        "auth.profile_updated",
        "user",
        Some(actor.id.to_string()),
        json!({"username":username,"avatar_updated":avatar_url.is_some()}),
    )
    .await?;
    Ok(Json(Envelope {
        data: json!({
            "authenticated":true,
            "user":{
                "id":actor.id,
                "email":row.get::<String,_>("email"),
                "display_name":display_name,
                "username":username,
                "avatar_url":avatar_url,
                "role":row.get::<String,_>("role")
            }
        }),
    }))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let rows = sqlx::query(
        "SELECT id,expires_at,created_at,last_seen_at,user_agent FROM sessions \
         WHERE user_id=$1 AND revoked_at IS NULL AND expires_at>NOW() ORDER BY last_seen_at DESC LIMIT 100",
    )
    .bind(actor.id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows
            .into_iter()
            .map(|row| {
                let id: Uuid = row.get("id");
                json!({
                    "id":id,"current":id==actor.session_id,
                    "expires_at":row.get::<chrono::DateTime<Utc>,_>("expires_at"),
                    "created_at":row.get::<chrono::DateTime<Utc>,_>("created_at"),
                    "last_seen_at":row.get::<chrono::DateTime<Utc>,_>("last_seen_at"),
                    "user_agent":row.get::<Option<String>,_>("user_agent")
                })
            })
            .collect(),
    }))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let changed = sqlx::query(
        "UPDATE sessions SET revoked_at=NOW() WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(actor.id)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?
    .rows_affected();
    if changed != 1 {
        return Err(ApiError::not_found());
    }
    audit(
        &state,
        Some(&actor),
        None,
        "session.revoke",
        "session",
        Some(id.to_string()),
        json!({"current":id==actor.session_id}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<InviteCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = require_admin(&state, &headers, true).await?;
    let max_uses = value.max_uses.unwrap_or(1);
    if !(1..=10_000).contains(&max_uses) {
        return Err(ApiError::invalid("max_uses must be 1..10000"));
    }
    let expires_in = value.expires_in_seconds.unwrap_or(7 * 86_400);
    if !(300..=365 * 86_400).contains(&expires_in) {
        return Err(ApiError::invalid(
            "invite expiry must be 5 minutes..365 days",
        ));
    }
    let email_constraint = value
        .email_constraint
        .as_deref()
        .map(normalize_email)
        .transpose()?;
    let note = value.note.unwrap_or_default();
    if note.len() > 1024 {
        return Err(ApiError::invalid("invite note is too long"));
    }
    let code = format!("sni_{}", random_secret(24));
    let prefix: String = code.chars().take(12).collect();
    let id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::seconds(expires_in);
    sqlx::query("INSERT INTO registration_invites(id,code_hash,code_prefix,created_by_user_id,email_constraint,max_uses,expires_at,note) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(id).bind(hmac_sha256(&state.config.invite_hmac_key, &code)).bind(&prefix).bind(actor.id).bind(&email_constraint).bind(max_uses).bind(expires_at).bind(&note)
        .execute(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        None,
        "invite.create",
        "registration_invite",
        Some(id.to_string()),
        json!({"max_uses":max_uses,"email_constraint":email_constraint}),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"id":id,"code":code,"code_prefix":prefix,"email_constraint":email_constraint,"max_uses":max_uses,"expires_at":expires_at,"note":note}),
        }),
    ))
}

pub async fn list_invites(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    require_admin(&state, &headers, false).await?;
    let rows = sqlx::query("SELECT id,code_prefix,email_constraint,max_uses,use_count,expires_at,revoked_at,note,created_by_user_id,created_at FROM registration_invites ORDER BY created_at DESC LIMIT 500")
        .fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope { data: rows.into_iter().map(|row| json!({
        "id":row.get::<Uuid,_>("id"),"code_prefix":row.get::<String,_>("code_prefix"),"email_constraint":row.get::<Option<String>,_>("email_constraint"),
        "max_uses":row.get::<i32,_>("max_uses"),"use_count":row.get::<i32,_>("use_count"),"expires_at":row.get::<chrono::DateTime<Utc>,_>("expires_at"),
        "revoked_at":row.get::<Option<chrono::DateTime<Utc>>,_>("revoked_at"),"note":row.get::<String,_>("note"),"created_by_user_id":row.get::<Uuid,_>("created_by_user_id"),"created_at":row.get::<chrono::DateTime<Utc>,_>("created_at")
    })).collect() }))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = require_admin(&state, &headers, true).await?;
    let result = sqlx::query(
        "UPDATE registration_invites SET revoked_at=COALESCE(revoked_at,NOW()) WHERE id=$1",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found());
    }
    audit(
        &state,
        Some(&actor),
        None,
        "invite.revoke",
        "registration_invite",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rate_auth(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    action: &str,
) -> Result<(), ApiError> {
    let key = crate::auth::client_ip(state, headers, peer).unwrap_or_else(|| "unknown".into());
    if !state.auth_rate.allow(&format!("{action}:{key}")).await {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many authentication attempts",
        ));
    }
    Ok(())
}

fn validate_display_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err(ApiError::invalid("display name must be 1..128 characters"));
    }
    Ok(value.to_owned())
}

fn generated_username(id: Uuid) -> String {
    format!("user-{}", id.simple())[..32].to_owned()
}

fn validate_username(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    let valid = (3..=32).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        });
    if !valid {
        return Err(ApiError::invalid(
            "username must be 3..32 lowercase letters, numbers, dots, underscores, or hyphens",
        ));
    }
    Ok(value)
}

fn validate_avatar_url(value: Option<String>) -> Result<Option<String>, ApiError> {
    let value = value
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty());
    if let Some(value) = value.as_deref()
        && (value.len() > 700_000
            || !(value.starts_with("data:image/png;base64,")
                || value.starts_with("data:image/jpeg;base64,")
                || value.starts_with("data:image/webp;base64,")
                || value.starts_with("https://")))
    {
        return Err(ApiError::invalid(
            "avatar must be a PNG, JPEG, or WebP image under 500 KB",
        ));
    }
    Ok(value)
}

fn invite_error() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "invite_unavailable",
        "registration invitation is unavailable",
    )
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "invalid email or password",
    )
}

fn session_response(
    state: &AppState,
    status: StatusCode,
    value: Value,
    session: &crate::auth::IssuedSession,
) -> Result<Response, ApiError> {
    let mut response = (status, Json(Envelope { data: value })).into_response();
    append_session_cookies(response.headers_mut(), state, session)?;
    Ok(response)
}
