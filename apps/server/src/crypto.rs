use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;

use crate::error::ApiError;

type HmacSha256 = Hmac<Sha256>;

pub fn normalize_email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() < 3
        || value.len() > 320
        || value.starts_with('@')
        || value.ends_with('@')
        || value.matches('@').count() != 1
    {
        return Err(ApiError::invalid("valid email is required"));
    }
    Ok(value)
}

pub fn validate_password(value: &str) -> Result<(), ApiError> {
    if !(12..=1024).contains(&value.len()) {
        return Err(ApiError::invalid("password must be 12..1024 characters"));
    }
    Ok(())
}

pub fn hash_password(value: &str) -> Result<String, ApiError> {
    validate_password(value)?;
    let params = Params::new(19_456, 2, 1, None)
        .map_err(|_| ApiError::invalid("password hashing parameters are invalid"))?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(
            value.as_bytes(),
            &SaltString::generate(&mut argon2::password_hash::rand_core::OsRng),
        )
        .map(|hash| hash.to_string())
        .map_err(|_| {
            ApiError::new(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "password_hash_failed",
                "password could not be secured",
            )
        })
}

pub fn verify_password(value: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(value.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn random_secret(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

pub fn sha256(value: impl AsRef<[u8]>) -> Vec<u8> {
    Sha256::digest(value.as_ref()).to_vec()
}

pub fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    hex::encode(sha256(value))
}

pub fn hmac_sha256(key: &[u8], value: &str) -> Vec<u8> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths");
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub fn constant_time_secret_eq(left: &str, right: &str) -> bool {
    sha256(left).ct_eq(&sha256(right)).into()
}

pub fn encrypt_secret(
    key_material: &[u8],
    aad: &[u8],
    plaintext: &str,
) -> Result<Vec<u8>, ApiError> {
    let key = sha256(key_material);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| {
        ApiError::new(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "encryption_failed",
            "credential could not be secured",
        )
    })?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let mut output = nonce.to_vec();
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            aes_gcm::aead::Payload {
                msg: plaintext.as_bytes(),
                aad,
            },
        )
        .map_err(|_| {
            ApiError::new(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "encryption_failed",
                "credential could not be secured",
            )
        })?;
    output.extend(encrypted);
    Ok(output)
}

pub fn decrypt_secret(
    key_material: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<String, ApiError> {
    if ciphertext.len() < 13 {
        return Err(ApiError::new(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "credential_unavailable",
            "provider credential is unavailable",
        ));
    }
    let key = sha256(key_material);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| {
        ApiError::new(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "credential_unavailable",
            "provider credential is unavailable",
        )
    })?;
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&ciphertext[..12]),
            aes_gcm::aead::Payload {
                msg: &ciphertext[12..],
                aad,
            },
        )
        .map_err(|_| {
            ApiError::new(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "credential_unavailable",
                "provider credential is unavailable",
            )
        })?;
    String::from_utf8(plain).map_err(|_| {
        ApiError::new(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "credential_unavailable",
            "provider credential is unavailable",
        )
    })
}

pub fn tool_arguments_digest(tool_name: &str, value: &serde_json::Value) -> String {
    fn canonical(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => serde_json::to_string(value).expect("JSON scalar"),
            serde_json::Value::Array(values) => format!(
                "[{}]",
                values.iter().map(canonical).collect::<Vec<_>>().join(",")
            ),
            serde_json::Value::Object(values) => {
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort();
                format!(
                    "{{{}}}",
                    keys.into_iter()
                        .map(|key| format!(
                            "{}:{}",
                            serde_json::to_string(key).expect("JSON key"),
                            canonical(&values[key])
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }
    sha256_hex(format!("{tool_name}\0{}", canonical(value)))
}

pub fn validate_provider_url(kind: &str, raw: &str) -> Result<String, ApiError> {
    let url = Url::parse(raw.trim()).map_err(|_| ApiError::invalid("invalid provider base URL"))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::invalid("invalid provider base URL"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::invalid("provider host is required"))?;
    let local_provider_port = match kind {
        "ollama" => Some(11_434),
        "llama-cpp" => Some(8_081),
        _ => None,
    };
    let local_provider = local_provider_port.is_some()
        && matches!(host, "localhost" | "127.0.0.1" | "host.docker.internal")
        && url.scheme() == "http"
        && url.port() == local_provider_port
        && url.path().trim_end_matches('/') == "/v1";
    if local_provider {
        return Ok(raw.trim().trim_end_matches('/').to_owned());
    }
    if url.scheme() != "https"
        || matches!(
            host,
            "localhost" | "127.0.0.1" | "169.254.169.254" | "metadata.google.internal"
        )
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().ok().is_some_and(|ip| {
            ip.is_loopback()
                || ip.is_unspecified()
                || match ip {
                    std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
                    std::net::IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
                }
        })
    {
        return Err(ApiError::invalid(
            "remote provider must use a public HTTPS endpoint",
        ));
    }
    Ok(raw.trim().trim_end_matches('/').to_owned())
}

pub fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            if separator && !output.is_empty() && output.len() < 63 {
                output.push('-');
            }
            separator = false;
            if output.len() < 64 {
                output.push(ch);
            }
        } else {
            separator = true;
        }
    }
    output.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2id_round_trip() {
        let hash = hash_password("a-strong-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("a-strong-password", &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn provider_urls_fail_closed() {
        assert!(validate_provider_url("openai-compatible", "http://169.254.169.254/v1").is_err());
        assert!(validate_provider_url("openai-compatible", "http://10.0.0.2/v1").is_err());
        assert!(validate_provider_url("openai", "https://api.openai.com/v1").is_ok());
        assert!(validate_provider_url("ollama", "http://host.docker.internal:11434/v1").is_ok());
        assert!(validate_provider_url("llama-cpp", "http://host.docker.internal:8081/v1").is_ok());
        assert!(validate_provider_url("llama-cpp", "http://host.docker.internal:8080/v1").is_err());
    }

    #[test]
    fn encrypted_provider_secret_round_trips_and_arguments_are_canonical() {
        let key = b"provider-key-at-least-32-bytes-long";
        let encrypted = encrypt_secret(key, b"provider-id", "private-token").unwrap();
        assert_eq!(
            decrypt_secret(key, b"provider-id", &encrypted).unwrap(),
            "private-token"
        );
        assert_eq!(
            tool_arguments_digest("example", &serde_json::json!({"b":2,"a":1})),
            tool_arguments_digest("example", &serde_json::json!({"a":1,"b":2}))
        );
    }
}
