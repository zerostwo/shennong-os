use super::projects::{require_project_read, require_project_write};
use super::{Envelope, audit, audit_tx};
use crate::{
    AppState,
    auth::{AuthUser, authenticate},
    crypto::{encrypt_secret, sha256_hex, slugify, validate_provider_url},
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ContextQuery {
    project_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct MemoryCreate {
    project_id: Option<Uuid>,
    title: String,
    content: String,
    source_kind: Option<String>,
    source_id: Option<String>,
}

#[derive(Deserialize)]
pub struct MemoryUpdate {
    title: Option<String>,
    content: Option<String>,
    lifecycle: Option<String>,
    change_note: Option<String>,
}

#[derive(Deserialize)]
pub struct SkillCreate {
    name: String,
    description: Option<String>,
    slug: Option<String>,
    trust_level: Option<String>,
    lifecycle: Option<String>,
    #[serde(default)]
    manifest: Value,
    content: String,
    package_version: Option<String>,
    change_note: Option<String>,
}

#[derive(Deserialize)]
pub struct SkillUpdate {
    name: Option<String>,
    description: Option<String>,
    lifecycle: Option<String>,
    manifest: Option<Value>,
}

#[derive(Deserialize)]
pub struct SkillVersionCreate {
    content: String,
    package_version: Option<String>,
    change_note: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SkillEnable {
    version: Option<i32>,
}

#[derive(Deserialize)]
pub struct ProviderCreate {
    name: String,
    provider_kind: String,
    base_url: String,
    model: String,
    data_policy: Option<String>,
    api_key: Option<String>,
    enabled: Option<bool>,
    is_default: Option<bool>,
}

#[derive(Deserialize)]
pub struct ProviderUpdate {
    name: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    data_policy: Option<String>,
    api_key: Option<String>,
    enabled: Option<bool>,
    is_default: Option<bool>,
}

pub async fn list_memories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ContextQuery>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    if let Some(project_id) = query.project_id {
        require_project_read(&state, &actor, project_id).await?;
    }
    let rows = sqlx::query(
        "SELECT m.*,v.content,v.content_sha256 FROM memories m JOIN memory_versions v ON v.memory_id=m.id AND v.version=m.current_version \
         WHERE m.owner_user_id=$1 AND m.project_id IS NOT DISTINCT FROM $2::uuid ORDER BY m.updated_at DESC LIMIT 500"
    ).bind(actor.id).bind(query.project_id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(memory_json).collect(),
    }))
}

pub async fn create_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<MemoryCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    if let Some(project_id) = value.project_id {
        require_project_write(&state, &actor, project_id).await?;
    }
    let title = validate_short(&value.title, 128, "memory title")?;
    validate_content(&value.content)?;
    let source_kind = value.source_kind.as_deref().unwrap_or("manual");
    if !matches!(source_kind, "manual" | "conversation" | "imported")
        || value.source_id.as_ref().is_some_and(|id| id.len() > 512)
    {
        return Err(ApiError::invalid("invalid memory source"));
    }
    let id = Uuid::new_v4();
    let digest = sha256_hex(value.content.as_bytes());
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query("INSERT INTO memories(id,owner_user_id,project_id,title,source_kind,source_id,current_version) VALUES($1,$2,$3,$4,$5,$6,1)")
        .bind(id).bind(actor.id).bind(value.project_id).bind(&title).bind(source_kind).bind(value.source_id).execute(&mut *tx).await.map_err(ApiError::database)?;
    sqlx::query("INSERT INTO memory_versions(memory_id,version,content,content_sha256,created_by_user_id) VALUES($1,1,$2,$3,$4)")
        .bind(id).bind(&value.content).bind(&digest).bind(actor.id).execute(&mut *tx).await.map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        value.project_id,
        "memory.create",
        "memory",
        Some(id.to_string()),
        json!({"version":1}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"id":id,"owner_user_id":actor.id,"project_id":value.project_id,"title":title,"source_kind":source_kind,"lifecycle":"active","version":1,"content":value.content,"content_sha256":digest}),
        }),
    ))
}

pub async fn get_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let row = find_memory(&state, id, &actor).await?;
    if let Some(project_id) = row.get::<Option<Uuid>, _>("project_id") {
        require_project_read(&state, &actor, project_id).await?;
    }
    Ok(Json(Envelope {
        data: memory_json(row),
    }))
}

pub async fn update_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<MemoryUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_memory(&state, id, &actor).await?;
    let project_id: Option<Uuid> = current.get("project_id");
    if let Some(project_id) = project_id {
        require_project_write(&state, &actor, project_id).await?;
    }
    let title = value
        .title
        .as_deref()
        .map(|value| validate_short(value, 128, "memory title"))
        .transpose()?;
    if value
        .lifecycle
        .as_deref()
        .is_some_and(|value| !matches!(value, "active" | "archived"))
    {
        return Err(ApiError::invalid("invalid memory lifecycle"));
    }
    if let Some(content) = value.content.as_deref() {
        validate_content(content)?;
    }
    let note = value.change_note.unwrap_or_default();
    if note.len() > 1024 {
        return Err(ApiError::invalid("change note is too long"));
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let version = sqlx::query_scalar::<_, i32>(
        "SELECT current_version FROM memories WHERE id=$1 AND owner_user_id=$2 FOR UPDATE",
    )
    .bind(id)
    .bind(actor.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    let next = if let Some(content) = value.content {
        let next = version + 1;
        let digest = sha256_hex(content.as_bytes());
        sqlx::query("INSERT INTO memory_versions(memory_id,version,content,content_sha256,change_note,created_by_user_id) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(next).bind(content).bind(digest).bind(&note).bind(actor.id).execute(&mut *tx).await.map_err(ApiError::database)?;
        next
    } else {
        version
    };
    sqlx::query("UPDATE memories SET title=COALESCE($2,title),lifecycle=COALESCE($3,lifecycle),current_version=$4,updated_at=NOW() WHERE id=$1")
        .bind(id).bind(title).bind(value.lifecycle).bind(next).execute(&mut *tx).await.map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        project_id,
        "memory.update",
        "memory",
        Some(id.to_string()),
        json!({"version":next}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    let row = find_memory(&state, id, &actor).await?;
    Ok(Json(Envelope {
        data: memory_json(row),
    }))
}

pub async fn archive_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let row = find_memory(&state, id, &actor).await?;
    let project_id: Option<Uuid> = row.get("project_id");
    if let Some(project_id) = project_id {
        require_project_write(&state, &actor, project_id).await?;
    }
    sqlx::query("UPDATE memories SET lifecycle='archived',updated_at=NOW() WHERE id=$1 AND owner_user_id=$2").bind(id).bind(actor.id).execute(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        project_id,
        "memory.archive",
        "memory",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let rows=sqlx::query("SELECT s.*,v.content,v.content_sha256,v.package_version FROM skills s JOIN skill_versions v ON v.skill_id=s.id AND v.version=s.current_version WHERE s.owner_user_id IS NULL OR s.owner_user_id=$1 ORDER BY s.owner_user_id NULLS FIRST,s.name").bind(actor.id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(skill_json).collect(),
    }))
}

pub async fn create_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<SkillCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let name = validate_short(&value.name, 128, "skill name")?;
    let description = value.description.unwrap_or_default();
    if description.len() > 1024 {
        return Err(ApiError::invalid("skill description is too long"));
    }
    let trust = value.trust_level.as_deref().unwrap_or("user");
    if !matches!(
        trust,
        "builtin_signed" | "admin_curated" | "user" | "generated"
    ) {
        return Err(ApiError::invalid("invalid skill trust level"));
    }
    if trust == "builtin_signed" {
        return Err(ApiError::invalid(
            "builtin_signed skills can only be installed by trusted platform migrations",
        ));
    }
    let global = trust == "admin_curated";
    if global && actor.role != "admin" {
        return Err(ApiError::forbidden());
    }
    let lifecycle = value.lifecycle.as_deref().unwrap_or("draft");
    if !matches!(lifecycle, "draft" | "active" | "disabled" | "archived") {
        return Err(ApiError::invalid("invalid skill lifecycle"));
    }
    if matches!(trust, "user" | "generated") && lifecycle != "draft" {
        return Err(ApiError::invalid(
            "user and generated skills must start as draft",
        ));
    }
    if !value.manifest.is_object() {
        return Err(ApiError::invalid("skill manifest must be an object"));
    }
    validate_content(&value.content)?;
    let slug = value
        .slug
        .as_deref()
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(&name));
    if slug.is_empty() {
        return Err(ApiError::invalid(
            "skill slug must contain ASCII letters or numbers",
        ));
    }
    let package = value.package_version.unwrap_or_else(|| "1".into());
    if package.len() > 128 {
        return Err(ApiError::invalid("package version is too long"));
    }
    let note = value.change_note.unwrap_or_default();
    if note.len() > 1024 {
        return Err(ApiError::invalid("change note is too long"));
    }
    let id = Uuid::new_v4();
    let owner = if global { None } else { Some(actor.id) };
    let digest = sha256_hex(value.content.as_bytes());
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query("INSERT INTO skills(id,owner_user_id,slug,name,description,trust_level,lifecycle,manifest,current_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,1)").bind(id).bind(owner).bind(&slug).bind(&name).bind(&description).bind(trust).bind(lifecycle).bind(&value.manifest).execute(&mut*tx).await.map_err(skill_write_error)?;
    sqlx::query("INSERT INTO skill_versions(skill_id,version,content,content_sha256,package_version,change_note,created_by_user_id) VALUES($1,1,$2,$3,$4,$5,$6)").bind(id).bind(&value.content).bind(&digest).bind(&package).bind(&note).bind(actor.id).execute(&mut*tx).await.map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        None,
        "skill.create",
        "skill",
        Some(id.to_string()),
        json!({"trust_level":trust,"version":1}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"id":id,"owner_user_id":owner,"slug":slug,"name":name,"description":description,"trust_level":trust,"lifecycle":lifecycle,"manifest":value.manifest,"version":1,"content":value.content,"content_sha256":digest,"package_version":package}),
        }),
    ))
}

pub async fn get_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let row = find_skill(&state, id, &actor).await?;
    Ok(Json(Envelope {
        data: skill_json(row),
    }))
}

pub async fn update_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<SkillUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_skill(&state, id, &actor).await?;
    authorize_skill_write(&actor, &current)?;
    let name = value
        .name
        .as_deref()
        .map(|v| validate_short(v, 128, "skill name"))
        .transpose()?;
    if value.description.as_ref().is_some_and(|v| v.len() > 1024) {
        return Err(ApiError::invalid("skill description is too long"));
    }
    if value
        .lifecycle
        .as_deref()
        .is_some_and(|v| !matches!(v, "draft" | "active" | "disabled" | "archived"))
    {
        return Err(ApiError::invalid("invalid skill lifecycle"));
    }
    if value.manifest.as_ref().is_some_and(|v| !v.is_object()) {
        return Err(ApiError::invalid("skill manifest must be an object"));
    }
    sqlx::query("UPDATE skills SET name=COALESCE($2,name),description=COALESCE($3,description),lifecycle=COALESCE($4,lifecycle),manifest=COALESCE($5,manifest),updated_at=NOW() WHERE id=$1").bind(id).bind(name).bind(value.description).bind(value.lifecycle).bind(value.manifest).execute(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        None,
        "skill.update",
        "skill",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    let row = find_skill(&state, id, &actor).await?;
    Ok(Json(Envelope {
        data: skill_json(row),
    }))
}

pub async fn list_skill_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    find_skill(&state, id, &actor).await?;
    let rows=sqlx::query("SELECT version,content,content_sha256,package_version,change_note,created_by_user_id,created_at FROM skill_versions WHERE skill_id=$1 ORDER BY version DESC").bind(id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope{data:rows.into_iter().map(|r|json!({"version":r.get::<i32,_>("version"),"content":r.get::<String,_>("content"),"content_sha256":r.get::<String,_>("content_sha256"),"package_version":r.get::<String,_>("package_version"),"change_note":r.get::<String,_>("change_note"),"created_by_user_id":r.get::<Option<Uuid>,_>("created_by_user_id"),"created_at":r.get::<chrono::DateTime<chrono::Utc>,_>("created_at")})).collect()}))
}

pub async fn create_skill_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<SkillVersionCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = find_skill(&state, id, &actor).await?;
    authorize_skill_write(&actor, &current)?;
    validate_content(&value.content)?;
    let package = value.package_version.unwrap_or_else(|| "1".into());
    if package.len() > 128 {
        return Err(ApiError::invalid("package version is too long"));
    }
    let note = value.change_note.unwrap_or_default();
    if note.len() > 1024 {
        return Err(ApiError::invalid("change note is too long"));
    }
    let digest = sha256_hex(value.content.as_bytes());
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    let current_version =
        sqlx::query_scalar::<_, i32>("SELECT current_version FROM skills WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::database)?;
    let next = current_version + 1;
    sqlx::query("INSERT INTO skill_versions(skill_id,version,content,content_sha256,package_version,change_note,created_by_user_id) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(next).bind(&value.content).bind(&digest).bind(&package).bind(&note).bind(actor.id).execute(&mut*tx).await.map_err(ApiError::database)?;
    sqlx::query("UPDATE skills SET current_version=$2,updated_at=NOW() WHERE id=$1")
        .bind(id)
        .bind(next)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        None,
        "skill.version_create",
        "skill",
        Some(id.to_string()),
        json!({"version":next,"content_sha256":digest}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"skill_id":id,"version":next,"content":value.content,"content_sha256":digest,"package_version":package,"change_note":note}),
        }),
    ))
}

pub async fn list_thread_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM threads WHERE id=$1")
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    require_project_read(&state, &actor, project_id).await?;
    let rows = sqlx::query(
        "SELECT s.*,v.content,v.content_sha256,v.package_version, \
         COALESCE(ts.enabled,FALSE) AS enabled,ts.skill_version AS selected_version \
         FROM skills s JOIN skill_versions v ON v.skill_id=s.id AND v.version=s.current_version \
         LEFT JOIN thread_skills ts ON ts.thread_id=$1 AND ts.skill_id=s.id \
         WHERE s.owner_user_id IS NULL OR s.owner_user_id=$2 \
         ORDER BY s.owner_user_id NULLS FIRST,s.name",
    )
    .bind(thread_id)
    .bind(actor.id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(thread_skill_json).collect(),
    }))
}

pub async fn enable_thread_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((thread_id, skill_id)): Path<(Uuid, Uuid)>,
    Json(value): Json<SkillEnable>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM threads WHERE id=$1")
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    require_project_write(&state, &actor, project_id).await?;
    let skill = find_skill(&state, skill_id, &actor).await?;
    if skill.get::<String, _>("lifecycle") != "active" {
        return Err(ApiError::conflict("only active skills can be enabled"));
    }
    let version = value
        .version
        .unwrap_or_else(|| skill.get("current_version"));
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM skill_versions WHERE skill_id=$1 AND version=$2)",
    )
    .bind(skill_id)
    .bind(version)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if !exists {
        return Err(ApiError::not_found());
    }
    sqlx::query("INSERT INTO thread_skills(thread_id,skill_id,skill_version,enabled) VALUES($1,$2,$3,TRUE) ON CONFLICT(thread_id,skill_id) DO UPDATE SET skill_version=EXCLUDED.skill_version,enabled=TRUE").bind(thread_id).bind(skill_id).bind(version).execute(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "thread.skill_enable",
        "skill",
        Some(skill_id.to_string()),
        json!({"thread_id":thread_id,"version":version}),
    )
    .await?;
    Ok(Json(Envelope {
        data: json!({"thread_id":thread_id,"skill_id":skill_id,"version":version,"enabled":true}),
    }))
}

pub async fn disable_thread_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((thread_id, skill_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let project_id = sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM threads WHERE id=$1")
        .bind(thread_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    require_project_write(&state, &actor, project_id).await?;
    find_skill(&state, skill_id, &actor).await?;
    let result = sqlx::query("DELETE FROM thread_skills WHERE thread_id=$1 AND skill_id=$2")
        .bind(thread_id)
        .bind(skill_id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found());
    }
    audit(
        &state,
        Some(&actor),
        Some(project_id),
        "thread.skill_disable",
        "skill",
        Some(skill_id.to_string()),
        json!({"thread_id":thread_id}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let rows=sqlx::query("SELECT id,name,provider_kind,base_url,model,data_policy,encrypted_api_key IS NOT NULL AS has_api_key,key_version,enabled,is_default,created_at,updated_at FROM model_providers WHERE owner_user_id=$1 ORDER BY is_default DESC,updated_at DESC").bind(actor.id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(provider_json).collect(),
    }))
}

pub async fn create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<ProviderCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    validate_provider_kind(&value.provider_kind)?;
    let name = validate_short(&value.name, 128, "provider name")?;
    let model = validate_short(&value.model, 256, "provider model")?;
    let base = validate_provider_url(&value.provider_kind, &value.base_url)?;
    let policy = value.data_policy.as_deref().unwrap_or("public_only");
    if !matches!(policy, "public_only" | "allow_private") {
        return Err(ApiError::invalid("invalid provider data policy"));
    }
    if value.api_key.as_ref().is_some_and(|v| v.len() > 8192) {
        return Err(ApiError::invalid("provider API key is too long"));
    }
    let id = Uuid::new_v4();
    let encrypted = value
        .api_key
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|key| {
            encrypt_secret(
                &state.config.provider_encryption_key,
                format!("{}:{}", actor.id, id).as_bytes(),
                key,
            )
        })
        .transpose()?;
    let default = value.is_default.unwrap_or(false);
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    if default {
        sqlx::query(
            "UPDATE model_providers SET is_default=FALSE,updated_at=NOW() WHERE owner_user_id=$1",
        )
        .bind(actor.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    }
    let row=sqlx::query("INSERT INTO model_providers(id,owner_user_id,name,provider_kind,base_url,model,data_policy,encrypted_api_key,enabled,is_default) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id,name,provider_kind,base_url,model,data_policy,encrypted_api_key IS NOT NULL AS has_api_key,key_version,enabled,is_default,created_at,updated_at").bind(id).bind(actor.id).bind(name).bind(&value.provider_kind).bind(base).bind(model).bind(policy).bind(encrypted).bind(value.enabled.unwrap_or(true)).bind(default).fetch_one(&mut*tx).await.map_err(skill_write_error)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        None,
        "provider.create",
        "provider",
        Some(id.to_string()),
        json!({"provider_kind":value.provider_kind}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: provider_json(row),
        }),
    ))
}

pub async fn update_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<ProviderUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let current = sqlx::query(
        "SELECT provider_kind,base_url FROM model_providers WHERE id=$1 AND owner_user_id=$2",
    )
    .bind(id)
    .bind(actor.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)?;
    let kind: String = current.get("provider_kind");
    let name = value
        .name
        .as_deref()
        .map(|v| validate_short(v, 128, "provider name"))
        .transpose()?;
    let model = value
        .model
        .as_deref()
        .map(|v| validate_short(v, 256, "provider model"))
        .transpose()?;
    let base = value
        .base_url
        .as_deref()
        .map(|v| validate_provider_url(&kind, v))
        .transpose()?;
    if value
        .data_policy
        .as_deref()
        .is_some_and(|v| !matches!(v, "public_only" | "allow_private"))
    {
        return Err(ApiError::invalid("invalid provider data policy"));
    }
    if value.api_key.as_ref().is_some_and(|v| v.len() > 8192) {
        return Err(ApiError::invalid("provider API key is too long"));
    }
    let encrypted = value
        .api_key
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|key| {
            encrypt_secret(
                &state.config.provider_encryption_key,
                format!("{}:{}", actor.id, id).as_bytes(),
                key,
            )
        })
        .transpose()?;
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    if value.is_default == Some(true) {
        sqlx::query("UPDATE model_providers SET is_default=FALSE,updated_at=NOW() WHERE owner_user_id=$1 AND id<>$2").bind(actor.id).bind(id).execute(&mut*tx).await.map_err(ApiError::database)?;
    }
    let row=sqlx::query("UPDATE model_providers SET name=COALESCE($3,name),base_url=COALESCE($4,base_url),model=COALESCE($5,model),data_policy=COALESCE($6,data_policy),encrypted_api_key=COALESCE($7,encrypted_api_key),enabled=COALESCE($8,enabled),is_default=COALESCE($9,is_default),updated_at=NOW() WHERE id=$1 AND owner_user_id=$2 RETURNING id,name,provider_kind,base_url,model,data_policy,encrypted_api_key IS NOT NULL AS has_api_key,key_version,enabled,is_default,created_at,updated_at").bind(id).bind(actor.id).bind(name).bind(base).bind(model).bind(value.data_policy).bind(encrypted).bind(value.enabled).bind(value.is_default).fetch_one(&mut*tx).await.map_err(skill_write_error)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        None,
        "provider.update",
        "provider",
        Some(id.to_string()),
        json!({}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: provider_json(row),
    }))
}

pub async fn delete_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let result = sqlx::query("DELETE FROM model_providers WHERE id=$1 AND owner_user_id=$2")
        .bind(id)
        .bind(actor.id)
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
        "provider.delete",
        "provider",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn find_memory(
    state: &AppState,
    id: Uuid,
    actor: &AuthUser,
) -> Result<sqlx::postgres::PgRow, ApiError> {
    let query = if actor.role == "admin" {
        "SELECT m.*,v.content,v.content_sha256 FROM memories m JOIN memory_versions v ON v.memory_id=m.id AND v.version=m.current_version WHERE m.id=$1"
    } else {
        "SELECT m.*,v.content,v.content_sha256 FROM memories m JOIN memory_versions v ON v.memory_id=m.id AND v.version=m.current_version WHERE m.id=$1 AND m.owner_user_id=$2"
    };
    let mut q = sqlx::query(query).bind(id);
    if actor.role != "admin" {
        q = q.bind(actor.id);
    }
    q.fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)
}
async fn find_skill(
    state: &AppState,
    id: Uuid,
    actor: &AuthUser,
) -> Result<sqlx::postgres::PgRow, ApiError> {
    sqlx::query("SELECT s.*,v.content,v.content_sha256,v.package_version FROM skills s JOIN skill_versions v ON v.skill_id=s.id AND v.version=s.current_version WHERE s.id=$1 AND (s.owner_user_id IS NULL OR s.owner_user_id=$2)").bind(id).bind(actor.id).fetch_optional(&state.pool).await.map_err(ApiError::database)?.ok_or_else(ApiError::not_found)
}
fn authorize_skill_write(actor: &AuthUser, row: &sqlx::postgres::PgRow) -> Result<(), ApiError> {
    if row.get::<String, _>("trust_level") == "builtin_signed" {
        return Err(ApiError::forbidden());
    }
    let owner: Option<Uuid> = row.get("owner_user_id");
    if owner == Some(actor.id) || (owner.is_none() && actor.role == "admin") {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}
fn validate_short(value: &str, max: usize, label: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        Err(ApiError::invalid(format!(
            "{label} must be 1..{max} characters"
        )))
    } else {
        Ok(value.into())
    }
}
fn validate_content(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() || value.len() > 65_536 || value.contains('\0') {
        Err(ApiError::invalid("content must be 1..65536 characters"))
    } else {
        Ok(())
    }
}
fn validate_provider_kind(value: &str) -> Result<(), ApiError> {
    if matches!(
        value,
        "openai" | "deepseek" | "ollama" | "openai-compatible"
    ) {
        Ok(())
    } else {
        Err(ApiError::invalid("unsupported provider kind"))
    }
}
fn skill_write_error(error: sqlx::Error) -> ApiError {
    if error.as_database_error().and_then(|v| v.code()).as_deref() == Some("23505") {
        ApiError::conflict("an item with this name already exists")
    } else {
        ApiError::database(error)
    }
}
fn memory_json(r: sqlx::postgres::PgRow) -> Value {
    json!({"id":r.get::<Uuid,_>("id"),"owner_user_id":r.get::<Uuid,_>("owner_user_id"),"project_id":r.get::<Option<Uuid>,_>("project_id"),"title":r.get::<String,_>("title"),"source_kind":r.get::<String,_>("source_kind"),"source_id":r.get::<Option<String>,_>("source_id"),"lifecycle":r.get::<String,_>("lifecycle"),"version":r.get::<i32,_>("current_version"),"content":r.get::<String,_>("content"),"content_sha256":r.get::<String,_>("content_sha256"),"created_at":r.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":r.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
fn skill_json(r: sqlx::postgres::PgRow) -> Value {
    skill_json_ref(&r)
}
fn skill_json_ref(r: &sqlx::postgres::PgRow) -> Value {
    json!({"id":r.get::<Uuid,_>("id"),"owner_user_id":r.get::<Option<Uuid>,_>("owner_user_id"),"slug":r.get::<String,_>("slug"),"name":r.get::<String,_>("name"),"description":r.get::<String,_>("description"),"trust_level":r.get::<String,_>("trust_level"),"lifecycle":r.get::<String,_>("lifecycle"),"manifest":r.get::<Value,_>("manifest"),"version":r.get::<i32,_>("current_version"),"content":r.get::<String,_>("content"),"content_sha256":r.get::<String,_>("content_sha256"),"package_version":r.get::<String,_>("package_version"),"created_at":r.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":r.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
fn thread_skill_json(r: sqlx::postgres::PgRow) -> Value {
    let mut value = skill_json_ref(&r);
    value["enabled"] = json!(r.get::<bool, _>("enabled"));
    value["selected_version"] = json!(r.get::<Option<i32>, _>("selected_version"));
    value
}
fn provider_json(r: sqlx::postgres::PgRow) -> Value {
    json!({"id":r.get::<Uuid,_>("id"),"name":r.get::<String,_>("name"),"provider_kind":r.get::<String,_>("provider_kind"),"base_url":r.get::<String,_>("base_url"),"model":r.get::<String,_>("model"),"data_policy":r.get::<String,_>("data_policy"),"has_api_key":r.get::<bool,_>("has_api_key"),"key_version":r.get::<i32,_>("key_version"),"enabled":r.get::<bool,_>("enabled"),"is_default":r.get::<bool,_>("is_default"),"created_at":r.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":r.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
