use crate::{
    AppState,
    crypto::{constant_time_secret_eq, hmac_sha256, random_secret, sha256},
    error::ApiError,
};
use axum::http::{
    HeaderMap, HeaderValue,
    header::{COOKIE, ORIGIN, SET_COOKIE},
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Serialize;
use sqlx::Row;
use std::net::SocketAddr;
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub role: String,
    #[serde(skip)]
    pub session_id: Uuid,
    #[serde(skip)]
    via_cookie: bool,
    #[serde(skip)]
    csrf_hash: Vec<u8>,
}

impl AuthUser {
    pub(crate) fn internal(id: Uuid, email: String, display_name: String, role: String) -> Self {
        let username = email.split('@').next().unwrap_or("user").to_owned();
        Self {
            id,
            email,
            display_name,
            username,
            avatar_url: None,
            role,
            session_id: Uuid::nil(),
            via_cookie: false,
            csrf_hash: Vec::new(),
        }
    }
}

pub struct IssuedSession {
    pub token: String,
    pub csrf: String,
    pub max_age: u64,
}

pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<AuthUser, ApiError> {
    let (token, via_cookie) = session_token(headers).ok_or_else(ApiError::unauthorized)?;
    let token_hash = sha256(token.as_bytes());
    let row = sqlx::query(
        "SELECT u.id,u.email,u.display_name,u.username,u.avatar_url,u.role,s.id AS session_id,s.csrf_hash \
         FROM sessions s JOIN users u ON u.id=s.user_id \
         WHERE s.token_hash=$1 AND s.revoked_at IS NULL AND s.expires_at>NOW() \
           AND u.status='active'",
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::unauthorized)?;
    let user = AuthUser {
        id: row.try_get("id").map_err(ApiError::database)?,
        email: row.try_get("email").map_err(ApiError::database)?,
        display_name: row.try_get("display_name").map_err(ApiError::database)?,
        username: row.try_get("username").map_err(ApiError::database)?,
        avatar_url: row.try_get("avatar_url").map_err(ApiError::database)?,
        role: row.try_get("role").map_err(ApiError::database)?,
        session_id: row.try_get("session_id").map_err(ApiError::database)?,
        via_cookie,
        csrf_hash: row.try_get("csrf_hash").map_err(ApiError::database)?,
    };
    if mutation {
        if !state
            .mutation_rate
            .allow(&format!("user:{}", user.id))
            .await
        {
            return Err(ApiError::new(
                http::StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many mutation requests",
            ));
        }
        enforce_origin(state, headers)?;
        if user.via_cookie {
            let csrf = headers
                .get("x-csrf-token")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    ApiError::new(
                        http::StatusCode::FORBIDDEN,
                        "csrf_required",
                        "CSRF token is required",
                    )
                })?;
            if !bool::from(sha256(csrf).ct_eq(&user.csrf_hash)) {
                return Err(ApiError::new(
                    http::StatusCode::FORBIDDEN,
                    "csrf_invalid",
                    "CSRF token is invalid",
                ));
            }
        }
    }
    Ok(user)
}

pub async fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<AuthUser, ApiError> {
    let user = authenticate(state, headers, mutation).await?;
    if user.role != "admin" {
        return Err(ApiError::forbidden());
    }
    Ok(user)
}

pub fn require_agent_runtime(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.config.os_service_token.as_deref().ok_or_else(|| {
        ApiError::new(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "internal_auth_unconfigured",
            "internal callback authentication is unavailable",
        )
    })?;
    let service = headers
        .get("x-shennong-service")
        .and_then(|value| value.to_str().ok());
    let provided = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if service != Some("agent-runtime")
        || !provided.is_some_and(|value| constant_time_secret_eq(value, expected))
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

pub fn enforce_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    match origin {
        Some(origin)
            if state
                .config
                .allowed_origins
                .contains(origin.trim_end_matches('/')) =>
        {
            Ok(())
        }
        Some(_) => Err(ApiError::new(
            http::StatusCode::FORBIDDEN,
            "origin_denied",
            "request origin is not allowed",
        )),
        None if headers.contains_key(COOKIE) => Err(ApiError::new(
            http::StatusCode::FORBIDDEN,
            "origin_required",
            "browser mutations require an Origin header",
        )),
        None => Ok(()),
    }
}

pub async fn issue_session(
    state: &AppState,
    user_id: Uuid,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<IssuedSession, ApiError> {
    let token = random_secret(32);
    let csrf = random_secret(24);
    let token_hash = sha256(&token);
    let csrf_hash = sha256(&csrf);
    let max_age = state.config.session_ttl.as_secs();
    let expires_at = Utc::now() + ChronoDuration::seconds(max_age as i64);
    let ip_hash =
        client_ip(state, headers, peer).map(|ip| hmac_sha256(&state.config.invite_hmac_key, &ip));
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());
    sqlx::query("INSERT INTO sessions(id,user_id,token_hash,csrf_hash,expires_at,ip_hash,user_agent) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::new_v4()).bind(user_id).bind(token_hash).bind(csrf_hash).bind(expires_at).bind(ip_hash).bind(user_agent)
        .execute(&state.pool).await.map_err(ApiError::database)?;
    Ok(IssuedSession {
        token,
        csrf,
        max_age,
    })
}

pub fn append_session_cookies(
    headers: &mut HeaderMap,
    state: &AppState,
    session: &IssuedSession,
) -> Result<(), ApiError> {
    let secure = if state.config.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let session_cookie = format!(
        "shennong_os_session={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        session.token, session.max_age, secure
    );
    let csrf_cookie = format!(
        "shennong_os_csrf={}; Path=/; SameSite=Strict; Max-Age={}{}",
        session.csrf, session.max_age, secure
    );
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie)
            .map_err(|_| ApiError::invalid("invalid session cookie"))?,
    );
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie)
            .map_err(|_| ApiError::invalid("invalid CSRF cookie"))?,
    );
    Ok(())
}

pub fn append_expired_cookies(headers: &mut HeaderMap, state: &AppState) -> Result<(), ApiError> {
    let secure = if state.config.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    for value in [
        format!("shennong_os_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"),
        format!("shennong_os_csrf=; Path=/; SameSite=Strict; Max-Age=0{secure}"),
    ] {
        headers.append(
            SET_COOKIE,
            HeaderValue::from_str(&value)
                .map_err(|_| ApiError::invalid("invalid expired cookie"))?,
        );
    }
    Ok(())
}

pub fn session_token(headers: &HeaderMap) -> Option<(&str, bool)> {
    if let Some(token) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
    {
        return Some((token, false));
    }
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .find_map(|cookie| cookie.strip_prefix("shennong_os_session="))
                .filter(|value| !value.is_empty())
                .map(|value| (value, true))
        })
}

pub fn client_ip(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Option<String> {
    if state.config.trust_proxy_headers {
        for name in ["cf-connecting-ip", "x-real-ip", "x-forwarded-for"] {
            if let Some(value) = headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(',').next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_owned());
            }
        }
    }
    peer.map(|value| value.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_precedes_cookie_and_cookie_is_marked() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "shennong_os_session=cookie-value".parse().unwrap());
        assert_eq!(session_token(&headers), Some(("cookie-value", true)));
        headers.insert("authorization", "Bearer api-value".parse().unwrap());
        assert_eq!(session_token(&headers), Some(("api-value", false)));
    }
}
