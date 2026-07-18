use super::{Envelope, audit, audit_tx};
use crate::{
    AppState,
    auth::{AuthUser, authenticate},
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ProjectCreate {
    name: String,
    #[serde(default)]
    description: String,
    visibility: Option<String>,
}

#[derive(Deserialize)]
pub struct ProjectUpdate {
    name: Option<String>,
    description: Option<String>,
    visibility: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
pub struct MemberWrite {
    role: String,
}

pub async fn project_role(
    state: &AppState,
    user: &AuthUser,
    project_id: Uuid,
) -> Result<String, ApiError> {
    if user.role == "admin" {
        return Ok("system_admin".into());
    }
    sqlx::query_scalar::<_, String>(
        "SELECT pm.role FROM project_members pm JOIN projects p ON p.id=pm.project_id \
         WHERE pm.project_id=$1 AND pm.user_id=$2 AND p.status='active'",
    )
    .bind(project_id)
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(ApiError::not_found)
}

pub async fn require_project_read(
    state: &AppState,
    user: &AuthUser,
    project_id: Uuid,
) -> Result<String, ApiError> {
    project_role(state, user, project_id).await
}

pub async fn require_project_write(
    state: &AppState,
    user: &AuthUser,
    project_id: Uuid,
) -> Result<String, ApiError> {
    let role = project_role(state, user, project_id).await?;
    if !matches!(role.as_str(), "system_admin" | "owner" | "admin" | "editor") {
        return Err(ApiError::not_found());
    }
    Ok(role)
}

pub async fn require_project_manage(
    state: &AppState,
    user: &AuthUser,
    project_id: Uuid,
) -> Result<String, ApiError> {
    let role = project_role(state, user, project_id).await?;
    if !matches!(role.as_str(), "system_admin" | "owner" | "admin") {
        return Err(ApiError::not_found());
    }
    Ok(role)
}

pub async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<ProjectCreate>,
) -> Result<(StatusCode, Json<Envelope<Value>>), ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let name = validate_name(&value.name)?;
    if value.description.len() > 4096 {
        return Err(ApiError::invalid("project description is too long"));
    }
    let visibility = value.visibility.as_deref().unwrap_or("private");
    if !matches!(visibility, "private" | "public") {
        return Err(ApiError::invalid("invalid project visibility"));
    }
    let id = Uuid::new_v4();
    let mut tx = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query(
        "INSERT INTO projects(id,owner_user_id,name,description,visibility) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(actor.id)
    .bind(&name)
    .bind(&value.description)
    .bind(visibility)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    sqlx::query("INSERT INTO project_members(project_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(id)
        .bind(actor.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    audit_tx(
        &mut tx,
        Some(actor.id),
        Some(id),
        "project.create",
        "project",
        Some(id.to_string()),
        json!({"visibility":visibility}),
        None,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    best_effort_project_shadow_sync(&state, id).await;
    Ok((
        StatusCode::CREATED,
        Json(Envelope {
            data: json!({"id":id,"owner_user_id":actor.id,"name":name,"description":value.description,"visibility":visibility,"status":"active","member_role":"owner"}),
        }),
    ))
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let rows = if actor.role == "admin" {
        sqlx::query("SELECT p.*,COALESCE(pm.role,'system_admin') AS member_role FROM projects p LEFT JOIN project_members pm ON pm.project_id=p.id AND pm.user_id=$1 ORDER BY p.updated_at DESC LIMIT 500")
            .bind(actor.id).fetch_all(&state.pool).await
    } else {
        sqlx::query("SELECT p.*,pm.role AS member_role FROM projects p JOIN project_members pm ON pm.project_id=p.id WHERE pm.user_id=$1 ORDER BY p.updated_at DESC LIMIT 500")
            .bind(actor.id).fetch_all(&state.pool).await
    }.map_err(ApiError::database)?;
    Ok(Json(Envelope {
        data: rows.into_iter().map(project_json).collect(),
    }))
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    let role = require_project_read(&state, &actor, id).await?;
    let row = sqlx::query("SELECT p.*,$2::text AS member_role FROM projects p WHERE p.id=$1")
        .bind(id)
        .bind(role)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(Envelope {
        data: project_json(row),
    }))
}

pub async fn update_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(value): Json<ProjectUpdate>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    let role = require_project_manage(&state, &actor, id).await?;
    let name = value.name.as_deref().map(validate_name).transpose()?;
    if value
        .description
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err(ApiError::invalid("project description is too long"));
    }
    if value
        .visibility
        .as_deref()
        .is_some_and(|value| !matches!(value, "private" | "public"))
    {
        return Err(ApiError::invalid("invalid project visibility"));
    }
    if value
        .status
        .as_deref()
        .is_some_and(|value| !matches!(value, "active" | "archived"))
    {
        return Err(ApiError::invalid("invalid project status"));
    }
    let row = sqlx::query("UPDATE projects SET name=COALESCE($2,name),description=COALESCE($3,description),visibility=COALESCE($4,visibility),status=COALESCE($5,status),updated_at=NOW() WHERE id=$1 RETURNING *,$6::text AS member_role")
        .bind(id).bind(name).bind(value.description).bind(value.visibility).bind(value.status).bind(&role)
        .fetch_optional(&state.pool).await.map_err(ApiError::database)?.ok_or_else(ApiError::not_found)?;
    audit(
        &state,
        Some(&actor),
        Some(id),
        "project.update",
        "project",
        Some(id.to_string()),
        json!({}),
    )
    .await?;
    best_effort_project_shadow_sync(&state, id).await;
    Ok(Json(Envelope {
        data: project_json(row),
    }))
}

async fn best_effort_project_shadow_sync(state: &AppState, project_id: Uuid) {
    if let Err(error) = super::data_plane::sync_project_shadow(state, project_id).await {
        tracing::warn!(
            %project_id,
            error_code = error.code,
            upstream_status = %error.status,
            "project committed in Shennong OS but its DB shadow could not be synchronized"
        );
    }
}

pub async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Envelope<Vec<Value>>>, ApiError> {
    let actor = authenticate(&state, &headers, false).await?;
    require_project_read(&state, &actor, id).await?;
    let rows = sqlx::query("SELECT u.id,u.email,u.display_name,u.status,pm.role,pm.created_at FROM project_members pm JOIN users u ON u.id=pm.user_id WHERE pm.project_id=$1 ORDER BY CASE pm.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'editor' THEN 2 ELSE 3 END,u.display_name")
        .bind(id).fetch_all(&state.pool).await.map_err(ApiError::database)?;
    Ok(Json(Envelope { data: rows.into_iter().map(|row| json!({"user_id":row.get::<Uuid,_>("id"),"email":row.get::<String,_>("email"),"display_name":row.get::<String,_>("display_name"),"status":row.get::<String,_>("status"),"role":row.get::<String,_>("role"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at")})).collect() }))
}

pub async fn put_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
    Json(value): Json<MemberWrite>,
) -> Result<Json<Envelope<Value>>, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_manage(&state, &actor, id).await?;
    if !matches!(value.role.as_str(), "admin" | "editor" | "viewer") {
        return Err(ApiError::invalid(
            "member role must be admin, editor, or viewer",
        ));
    }
    let owner = sqlx::query_scalar::<_, Uuid>("SELECT owner_user_id FROM projects WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    if user_id == owner {
        return Err(ApiError::conflict("project owner role cannot be changed"));
    }
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;
    if !exists {
        return Err(ApiError::not_found());
    }
    sqlx::query("INSERT INTO project_members(project_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT(project_id,user_id) DO UPDATE SET role=EXCLUDED.role,updated_at=NOW()")
        .bind(id).bind(user_id).bind(&value.role).execute(&state.pool).await.map_err(ApiError::database)?;
    audit(
        &state,
        Some(&actor),
        Some(id),
        "project.member_upsert",
        "user",
        Some(user_id.to_string()),
        json!({"role":value.role}),
    )
    .await?;
    Ok(Json(Envelope {
        data: json!({"project_id":id,"user_id":user_id,"role":value.role}),
    }))
}

pub async fn delete_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticate(&state, &headers, true).await?;
    require_project_manage(&state, &actor, id).await?;
    let owner = sqlx::query_scalar::<_, Uuid>("SELECT owner_user_id FROM projects WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(ApiError::not_found)?;
    if user_id == owner {
        return Err(ApiError::conflict("project owner cannot be removed"));
    }
    let result = sqlx::query("DELETE FROM project_members WHERE project_id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(ApiError::database)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found());
    }
    audit(
        &state,
        Some(&actor),
        Some(id),
        "project.member_remove",
        "user",
        Some(user_id.to_string()),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 {
        return Err(ApiError::invalid("project name must be 1..200 characters"));
    }
    Ok(value.to_owned())
}

fn project_json(row: sqlx::postgres::PgRow) -> Value {
    json!({"id":row.get::<Uuid,_>("id"),"owner_user_id":row.get::<Uuid,_>("owner_user_id"),"name":row.get::<String,_>("name"),"description":row.get::<String,_>("description"),"visibility":row.get::<String,_>("visibility"),"status":row.get::<String,_>("status"),"member_role":row.get::<String,_>("member_role"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"updated_at":row.get::<chrono::DateTime<chrono::Utc>,_>("updated_at")})
}
