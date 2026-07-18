use crate::clients::{AgentRuntimeClient, ServiceClient};
use std::{collections::HashSet, env, fs, net::SocketAddr, str::FromStr, time::Duration};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("{0} must contain at least 32 characters")]
    SecretTooShort(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("set only one of {0} or {0}_FILE")]
    AmbiguousSecret(&'static str),
    #[error("cannot read secret file configured by {0}_FILE")]
    SecretFile(&'static str),
    #[error(transparent)]
    Client(#[from] crate::clients::ClientConfigError),
}

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub bind: SocketAddr,
    pub public_origin: Url,
    pub ide_public_origin: Option<Url>,
    pub bootstrap_token: String,
    pub invite_hmac_key: Vec<u8>,
    pub provider_encryption_key: Vec<u8>,
    pub allowed_origins: HashSet<String>,
    pub cookie_secure: bool,
    pub session_ttl: Duration,
    pub trust_proxy_headers: bool,
    pub run_migrations: bool,
    pub db_client: Option<ServiceClient>,
    pub runtime_client: Option<ServiceClient>,
    pub agent_runtime_client: Option<AgentRuntimeClient>,
    pub os_service_token: Option<String>,
    pub db_admin_key: Option<String>,
    pub runtime_jwt_signer: Option<RuntimeJwtSigner>,
    pub runtime_jwt_issuer: String,
    pub runtime_jwt_audience: String,
    pub max_upload_bytes: usize,
    pub upload_timeout: Duration,
}

#[derive(Clone)]
pub enum RuntimeJwtSigner {
    Ed25519(Vec<u8>),
    Hs256(Vec<u8>),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = required("SHENNONG_OS_DATABASE_URL")?;
        let bootstrap_token = secret("SHENNONG_OS_BOOTSTRAP_TOKEN")?;
        let invite_hmac_key = secret("SHENNONG_OS_INVITE_HMAC_KEY")?.into_bytes();
        let provider_encryption_key = secret("SHENNONG_OS_PROVIDER_ENCRYPTION_KEY")?.into_bytes();
        let bind = env::var("SHENNONG_OS_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8080".into())
            .parse()
            .map_err(|_| ConfigError::Invalid("SHENNONG_OS_BIND"))?;
        let allowed_origins = env::var("SHENNONG_OS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".into())
            .split(',')
            .filter_map(|value| normalize_origin(value).ok())
            .collect::<HashSet<_>>();
        if allowed_origins.is_empty() {
            return Err(ConfigError::Invalid("SHENNONG_OS_ALLOWED_ORIGINS"));
        }
        let public_origin = normalize_origin_url(&required("SHENNONG_PUBLIC_ORIGIN")?)?;
        if !allowed_origins.contains(public_origin.as_str().trim_end_matches('/')) {
            return Err(ConfigError::Invalid(
                "SHENNONG_PUBLIC_ORIGIN must be in SHENNONG_OS_ALLOWED_ORIGINS",
            ));
        }
        let ide_public_origin = env::var("SHENNONG_IDE_PUBLIC_ORIGIN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| normalize_origin_url(&value))
            .transpose()?;
        if let Some(ide_origin) = ide_public_origin.as_ref() {
            validate_ide_origin_pair(&public_origin, ide_origin)?;
        }
        let cookie_secure = bool_env("SHENNONG_OS_COOKIE_SECURE", true);
        if cookie_secure
            && ide_public_origin
                .as_ref()
                .is_some_and(|origin| origin.scheme() != "https")
        {
            return Err(ConfigError::Invalid(
                "SHENNONG_IDE_PUBLIC_ORIGIN must use https when secure cookies are enabled",
            ));
        }
        let session_ttl = Duration::from_secs(
            env::var("SHENNONG_OS_SESSION_TTL_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(28_800_u64)
                .clamp(300, 2_592_000),
        );
        let max_upload_bytes = env::var("SHENNONG_OS_MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(50 * 1024 * 1024 * 1024)
            .clamp(1, 1024 * 1024 * 1024 * 1024);
        let max_upload_bytes = usize::try_from(max_upload_bytes)
            .map_err(|_| ConfigError::Invalid("SHENNONG_OS_MAX_UPLOAD_BYTES"))?;
        let upload_timeout = Duration::from_secs(
            env::var("SHENNONG_OS_UPLOAD_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(14_400_u64)
                .clamp(60, 86_400),
        );
        let db_client = optional_client("SHENNONG_DB_URL", "shennong-db")?;
        let runtime_client = optional_client("SHENNONG_RUNTIME_URL", "shennong-runtime")?;
        let db_admin_key = optional_secret("SHENNONG_DB_ADMIN_KEY")?;
        if db_client.is_some() && db_admin_key.is_none() {
            return Err(ConfigError::Missing("SHENNONG_DB_ADMIN_KEY(_FILE)"));
        }
        let runtime_jwt_ed25519 = optional_secret("SHENNONG_RUNTIME_JWT_ED25519_PRIVATE_KEY")?;
        let runtime_jwt_hs256 = optional_secret("SHENNONG_RUNTIME_JWT_HS256_SECRET")?;
        let runtime_jwt_signer = match (runtime_jwt_ed25519, runtime_jwt_hs256) {
            (Some(private_key), None) => {
                jsonwebtoken::EncodingKey::from_ed_pem(private_key.as_bytes())
                    .map_err(|_| ConfigError::Invalid("Runtime Ed25519 private key"))?;
                Some(RuntimeJwtSigner::Ed25519(private_key.into_bytes()))
            }
            (None, Some(secret)) => Some(RuntimeJwtSigner::Hs256(secret.into_bytes())),
            (Some(_), Some(_)) => {
                return Err(ConfigError::Invalid(
                    "set exactly one Runtime JWT signing key",
                ));
            }
            (None, None) => None,
        };
        if runtime_client.is_some() && runtime_jwt_signer.is_none() {
            return Err(ConfigError::Missing(
                "SHENNONG_RUNTIME_JWT_ED25519_PRIVATE_KEY(_FILE)",
            ));
        }
        if runtime_client.is_some() && ide_public_origin.is_none() {
            return Err(ConfigError::Missing("SHENNONG_IDE_PUBLIC_ORIGIN"));
        }
        let agent_url = env::var("SHENNONG_AGENT_RUNTIME_URL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let agent_secret = optional_secret("SHENNONG_AGENT_RUNTIME_SECRET")?;
        let os_service_token = optional_secret("SHENNONG_OS_SERVICE_TOKEN")?;
        let agent_runtime_client = match (agent_url, agent_secret) {
            (Some(url), Some(secret)) => Some(AgentRuntimeClient::new(&url, secret)?),
            (None, None) => None,
            _ => return Err(ConfigError::Invalid("agent runtime URL/secret pair")),
        };
        if agent_runtime_client.is_some() && os_service_token.is_none() {
            return Err(ConfigError::Missing("SHENNONG_OS_SERVICE_TOKEN(_FILE)"));
        }
        Ok(Self {
            database_url,
            bind,
            public_origin,
            ide_public_origin,
            bootstrap_token,
            invite_hmac_key,
            provider_encryption_key,
            allowed_origins,
            cookie_secure,
            session_ttl,
            trust_proxy_headers: bool_env("SHENNONG_OS_TRUST_PROXY_HEADERS", false),
            run_migrations: bool_env("SHENNONG_OS_RUN_MIGRATIONS", true),
            db_client,
            runtime_client,
            agent_runtime_client,
            os_service_token,
            db_admin_key,
            runtime_jwt_signer,
            runtime_jwt_issuer: env::var("SHENNONG_RUNTIME_JWT_ISSUER")
                .unwrap_or_else(|_| "shennong-os".into()),
            runtime_jwt_audience: env::var("SHENNONG_RUNTIME_JWT_AUDIENCE")
                .unwrap_or_else(|_| "shennong-runtime".into()),
            max_upload_bytes,
            upload_timeout,
        })
    }

    /// Deterministic defaults for isolated integration-test databases.
    ///
    /// Callers must still ensure `database_url` points at a disposable test
    /// database; the ignored PostgreSQL tests enforce that convention.
    pub fn for_test(database_url: String) -> Self {
        Self {
            database_url,
            bind: "127.0.0.1:0".parse().expect("test bind"),
            public_origin: Url::parse("https://os.test").expect("test OS origin"),
            ide_public_origin: Some(
                Url::parse("https://ide.test").expect("test IDE public origin"),
            ),
            bootstrap_token: "b".repeat(32),
            invite_hmac_key: vec![b'i'; 32],
            provider_encryption_key: vec![b'p'; 32],
            allowed_origins: ["https://os.test".to_owned()].into_iter().collect(),
            cookie_secure: true,
            session_ttl: Duration::from_secs(3600),
            trust_proxy_headers: false,
            run_migrations: true,
            db_client: None,
            runtime_client: None,
            agent_runtime_client: None,
            os_service_token: Some("o".repeat(32)),
            db_admin_key: None,
            runtime_jwt_signer: None,
            runtime_jwt_issuer: "shennong-os".into(),
            runtime_jwt_audience: "shennong-runtime".into(),
            max_upload_bytes: 64 * 1024 * 1024,
            upload_timeout: Duration::from_secs(300),
        }
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    sourced_value(name)?.ok_or(ConfigError::Missing(name))
}

fn secret(name: &'static str) -> Result<String, ConfigError> {
    let value = required(name)?;
    if value.len() < 32 {
        return Err(ConfigError::SecretTooShort(name));
    }
    Ok(value)
}

fn optional_secret(name: &'static str) -> Result<Option<String>, ConfigError> {
    let value = sourced_value(name)?;
    if value.as_ref().is_some_and(|value| value.len() < 32) {
        return Err(ConfigError::SecretTooShort(name));
    }
    Ok(value)
}

fn sourced_value(name: &'static str) -> Result<Option<String>, ConfigError> {
    let direct = env::var(name).ok().filter(|value| !value.is_empty());
    let file_name = format!("{name}_FILE");
    let file = env::var(&file_name)
        .ok()
        .filter(|value| !value.trim().is_empty());
    if direct.is_some() && file.is_some() {
        return Err(ConfigError::AmbiguousSecret(name));
    }
    if let Some(path) = file {
        let value = fs::read_to_string(path).map_err(|_| ConfigError::SecretFile(name))?;
        let value = value.trim_end_matches(['\r', '\n']).to_owned();
        return (!value.is_empty())
            .then_some(value)
            .ok_or(ConfigError::Missing(name))
            .map(Some);
    }
    Ok(direct)
}

fn bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(default)
}

fn optional_client(
    name: &'static str,
    service: &'static str,
) -> Result<Option<ServiceClient>, ConfigError> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| ServiceClient::new(service, &value))
        .transpose()
        .map_err(Into::into)
}

pub fn normalize_origin(value: &str) -> Result<String, ConfigError> {
    Ok(normalize_origin_url(value)?
        .as_str()
        .trim_end_matches('/')
        .to_owned())
}

fn normalize_origin_url(value: &str) -> Result<Url, ConfigError> {
    let parsed = Url::from_str(value.trim()).map_err(|_| ConfigError::Invalid("origin"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err(ConfigError::Invalid("origin"));
    }
    Ok(parsed)
}

fn validate_ide_origin_pair(public: &Url, ide: &Url) -> Result<(), ConfigError> {
    if public
        .host_str()
        .zip(ide.host_str())
        .is_none_or(|(public, ide)| public.eq_ignore_ascii_case(ide))
    {
        return Err(ConfigError::Invalid(
            "SHENNONG_IDE_PUBLIC_ORIGIN must use a different host than SHENNONG_PUBLIC_ORIGIN",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ide_origin_requires_a_distinct_host() {
        let public = Url::parse("https://os.example.test").unwrap();
        let same_host = Url::parse("https://os.example.test:8443").unwrap();
        let ide = Url::parse("https://ide.example.test").unwrap();
        assert!(validate_ide_origin_pair(&public, &same_host).is_err());
        assert!(validate_ide_origin_pair(&public, &ide).is_ok());
    }
}
