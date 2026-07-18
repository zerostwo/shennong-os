mod auth;
mod clients;
mod config;
mod crypto;
mod error;
mod handlers;
mod rate_limit;

pub use clients::{ServiceClient, ServiceHealth};
pub use config::AppConfig;
pub use error::ApiError;

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, Uri},
    middleware,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{any, delete, get, patch, post, put},
};
use rate_limit::RateLimiter;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};
use tower_http::{
    catch_panic::CatchPanicLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<AppConfig>,
    pub auth_rate: RateLimiter,
    pub mutation_rate: RateLimiter,
}

pub async fn build_state(config: AppConfig) -> Result<AppState, Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .min_connections(1)
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(600))
        .connect(&config.database_url)
        .await?;
    if config.run_migrations {
        sqlx::migrate!("../../migrations").run(&pool).await?;
    }
    Ok(AppState {
        pool,
        config: Arc::new(config),
        auth_rate: RateLimiter::new(20, Duration::from_secs(60)),
        mutation_rate: RateLimiter::new(240, Duration::from_secs(60)),
    })
}

pub fn router(state: AppState) -> Router {
    let control = Router::new()
        .route("/healthz", get(handlers::system::healthz))
        .route("/version", get(handlers::system::version))
        .route("/openapi.yaml", get(handlers::system::openapi))
        .route(
            "/api/v1/public-config",
            get(handlers::integration::public_config),
        )
        .route(
            "/api/v1/capabilities",
            get(handlers::integration::capabilities),
        )
        .route(
            "/api/v1/setup/status",
            get(handlers::authentication::setup_status),
        )
        .route(
            "/api/v1/setup/admin",
            post(handlers::authentication::setup_admin),
        )
        .route(
            "/api/v1/auth/registration-policy",
            get(handlers::authentication::registration_policy),
        )
        .route(
            "/api/v1/auth/register",
            post(handlers::authentication::register),
        )
        .route(
            "/api/v1/auth/sign-in",
            post(handlers::authentication::sign_in),
        )
        .route(
            "/api/v1/auth/sign-out",
            post(handlers::authentication::sign_out),
        )
        .route(
            "/api/v1/auth/session",
            get(handlers::authentication::session),
        )
        .route(
            "/api/v1/auth/sessions",
            get(handlers::authentication::list_sessions),
        )
        .route(
            "/api/v1/auth/sessions/{id}",
            delete(handlers::authentication::revoke_session),
        )
        .route(
            "/api/v1/admin/invites",
            get(handlers::authentication::list_invites)
                .post(handlers::authentication::create_invite),
        )
        .route(
            "/api/v1/admin/invites/{id}",
            delete(handlers::authentication::revoke_invite),
        )
        .route(
            "/api/v1/admin/registration-policy",
            patch(handlers::authentication::update_registration_policy),
        )
        .route(
            "/api/v1/projects",
            get(handlers::projects::list_projects).post(handlers::projects::create_project),
        )
        .route(
            "/api/v1/projects/{id}",
            get(handlers::projects::get_project).patch(handlers::projects::update_project),
        )
        .route(
            "/api/v1/projects/{id}/members",
            get(handlers::projects::list_members),
        )
        .route(
            "/api/v1/projects/{id}/members/{user_id}",
            put(handlers::projects::put_member).delete(handlers::projects::delete_member),
        )
        .route(
            "/api/v1/threads",
            get(handlers::agent::list_threads).post(handlers::agent::create_thread),
        )
        .route(
            "/api/v1/threads/{id}",
            get(handlers::agent::get_thread)
                .patch(handlers::agent::update_thread)
                .delete(handlers::agent::delete_thread),
        )
        .route(
            "/api/v1/threads/{id}/messages",
            get(handlers::agent::list_messages).post(handlers::agent::create_message),
        )
        .route(
            "/api/v1/threads/{id}/runs/active",
            get(handlers::agent::get_active_thread_run),
        )
        .route("/api/v1/agent", post(handlers::integration::agent_gateway))
        .route(
            "/api/v1/agent/runs",
            post(handlers::integration::bootstrap_run),
        )
        .route(
            "/api/v1/agent/runs/{id}/metadata",
            post(handlers::integration::record_run_metadata),
        )
        .route(
            "/api/v1/agent/runs/{id}/events",
            post(handlers::integration::append_run_event),
        )
        .route(
            "/api/v1/agent/runs/{id}/finish",
            post(handlers::integration::finish_run),
        )
        .route(
            "/api/v1/agent/runs/{id}/approvals/verify",
            post(handlers::integration::verify_capability),
        )
        .route(
            "/api/v1/agent/runs/{id}/tools",
            post(handlers::integration::execute_tool),
        )
        .route(
            "/api/v1/threads/{id}/runs",
            post(handlers::agent::create_run),
        )
        .route("/api/v1/runs", get(handlers::agent::list_runs))
        .route(
            "/api/v1/runs/{id}",
            get(handlers::agent::get_run).patch(handlers::agent::update_run),
        )
        .route(
            "/api/v1/runs/{id}/events/stream",
            get(handlers::agent::stream_run_events),
        )
        .route(
            "/api/v1/runs/{id}/events",
            get(handlers::agent::list_run_events).post(handlers::agent::create_run_event),
        )
        .route(
            "/api/v1/runs/{id}/plan",
            get(handlers::agent::get_task_plan).put(handlers::agent::put_task_plan),
        )
        .route(
            "/api/v1/projects/{id}/jobs",
            get(handlers::agent::list_jobs).post(handlers::agent::create_job),
        )
        .route(
            "/api/v1/jobs/{id}",
            get(handlers::agent::get_job).patch(handlers::agent::update_job),
        )
        .route(
            "/api/v1/jobs",
            get(handlers::runtime_control::list_all_jobs),
        )
        .route(
            "/api/v1/jobs/{id}/cancel",
            post(handlers::runtime_control::cancel_job),
        )
        .route(
            "/api/v1/projects/{id}/sessions",
            get(handlers::runtime_control::list_sessions)
                .post(handlers::runtime_control::create_session),
        )
        .route(
            "/api/v1/sessions/{id}",
            get(handlers::runtime_control::get_session),
        )
        .route(
            "/api/v1/sessions/{id}/stop",
            post(handlers::runtime_control::stop_session),
        )
        .route(
            "/api/v1/sessions/{id}/launch",
            post(handlers::runtime_control::launch_session),
        )
        .route(
            "/api/v1/projects/{id}/artifacts",
            get(handlers::agent::list_artifacts).post(handlers::agent::create_artifact),
        )
        .route(
            "/api/v1/memories",
            get(handlers::context::list_memories).post(handlers::context::create_memory),
        )
        .route(
            "/api/v1/memories/{id}",
            get(handlers::context::get_memory)
                .patch(handlers::context::update_memory)
                .delete(handlers::context::archive_memory),
        )
        .route(
            "/api/v1/skills",
            get(handlers::context::list_skills).post(handlers::context::create_skill),
        )
        .route(
            "/api/v1/skills/{id}",
            get(handlers::context::get_skill).patch(handlers::context::update_skill),
        )
        .route(
            "/api/v1/skills/{id}/versions",
            get(handlers::context::list_skill_versions)
                .post(handlers::context::create_skill_version),
        )
        .route(
            "/api/v1/threads/{id}/skills/{skill_id}",
            put(handlers::context::enable_thread_skill)
                .delete(handlers::context::disable_thread_skill),
        )
        .route(
            "/api/v1/threads/{id}/skills",
            get(handlers::context::list_thread_skills),
        )
        .route(
            "/api/v1/providers",
            get(handlers::context::list_providers).post(handlers::context::create_provider),
        )
        .route(
            "/api/v1/providers/{id}",
            patch(handlers::context::update_provider).delete(handlers::context::delete_provider),
        )
        .route(
            "/api/v1/system/dependencies",
            get(handlers::system::dependencies),
        )
        .route("/api/v1/audit-events", get(handlers::system::audit_events))
        .route(
            "/api/v1/resources",
            get(handlers::data_plane::resources_root),
        )
        .route(
            "/api/v1/resources/{id}",
            get(handlers::data_plane::resource),
        )
        .route(
            "/api/v1/resources/{id}/{child}",
            get(handlers::data_plane::resource_child),
        )
        .route("/api/v1/query", post(handlers::data_plane::query))
        .route(
            "/api/v1/projects/{id}/graph/subgraph",
            get(handlers::data_plane::project_subgraph),
        )
        .route(
            "/api/v1/projects/{id}/{*tail}",
            any(handlers::data_plane::project_data),
        )
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));
    let project_upload_stream = Router::new()
        .route(
            "/api/v1/projects/{id}/uploads",
            get(handlers::data_plane::list_project_uploads)
                .post(handlers::data_plane::upload_project_file),
        )
        .layer(RequestBodyLimitLayer::new(state.config.max_upload_bytes));
    let project_upload_registration = Router::new()
        .route(
            "/api/v1/projects/{id}/uploads/register",
            post(handlers::data_plane::register_project_uploads),
        )
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));
    let ide = Router::new()
        .route(
            "/__shennong/launch",
            get(handlers::runtime_control::redeem_ide_ticket),
        )
        .route(
            "/v1/sessions/{id}/proxy",
            any(handlers::runtime_control::ide_proxy_root),
        )
        .route(
            "/v1/sessions/{id}/proxy/",
            any(handlers::runtime_control::ide_proxy_root),
        )
        .route(
            "/v1/sessions/{id}/proxy/{*path}",
            any(handlers::runtime_control::ide_proxy_path),
        )
        .layer(RequestBodyLimitLayer::new(64 * 1024 * 1024));
    control
        .merge(project_upload_stream)
        .merge(project_upload_registration)
        .merge(ide)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(CatchPanicLayer::new())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                tracing::debug_span!(
                    "request",
                    method = %request.method(),
                    path = %request_log_path(request.uri()),
                    version = ?request.version(),
                )
            }),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_host_boundary,
        ))
        .with_state(state)
}

fn request_log_path(uri: &Uri) -> &str {
    uri.path()
}

async fn enforce_host_boundary(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if handlers::runtime_control::request_targets_ide_host(&state, request.headers())
        && !handlers::runtime_control::ide_host_path_allowed(request.uri().path())
    {
        return ApiError::not_found().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn request_trace_never_records_query_secrets() {
        let uri: Uri = "/__shennong/launch?ticket=one-time-secret"
            .parse()
            .expect("launch URI");
        assert_eq!(request_log_path(&uri), "/__shennong/launch");
        assert!(!request_log_path(&uri).contains("ticket"));
    }

    #[tokio::test]
    async fn all_v1_routes_can_be_composed_without_conflicts() {
        let config = AppConfig::for_test("postgres://test:test@127.0.0.1/test_router".into());
        let state = AppState {
            pool: PgPoolOptions::new()
                .connect_lazy(&config.database_url)
                .expect("lazy test pool"),
            config: Arc::new(config),
            auth_rate: RateLimiter::new(20, Duration::from_secs(60)),
            mutation_rate: RateLimiter::new(240, Duration::from_secs(60)),
        };
        let _router = router(state);
    }

    #[tokio::test]
    async fn ide_host_rejects_normal_os_api_before_handler_dispatch() {
        let config = AppConfig::for_test("postgres://test:test@127.0.0.1/test_router".into());
        let state = AppState {
            pool: PgPoolOptions::new()
                .connect_lazy(&config.database_url)
                .expect("lazy test pool"),
            config: Arc::new(config),
            auth_rate: RateLimiter::new(20, Duration::from_secs(60)),
            mutation_rate: RateLimiter::new(240, Duration::from_secs(60)),
        };
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/public-config")
                    .header("host", "ide.test")
                    .body(Body::empty())
                    .expect("IDE-host request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn project_graph_subgraph_route_is_composed() {
        let config = AppConfig::for_test("postgres://test:test@127.0.0.1/test_router".into());
        let state = AppState {
            pool: PgPoolOptions::new()
                .connect_lazy(&config.database_url)
                .expect("lazy test pool"),
            config: Arc::new(config),
            auth_rate: RateLimiter::new(20, Duration::from_secs(60)),
            mutation_rate: RateLimiter::new(240, Duration::from_secs(60)),
        };
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/projects/{}/graph/subgraph?root=sample-1&depth=2&limit=80",
                        uuid::Uuid::nil()
                    ))
                    .body(Body::empty())
                    .expect("project graph request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn public_resource_catalog_does_not_require_an_os_session() {
        let config = AppConfig::for_test("postgres://test:test@127.0.0.1/test_router".into());
        let state = AppState {
            pool: PgPoolOptions::new()
                .connect_lazy(&config.database_url)
                .expect("lazy test pool"),
            config: Arc::new(config),
            auth_rate: RateLimiter::new(20, Duration::from_secs(60)),
            mutation_rate: RateLimiter::new(240, Duration::from_secs(60)),
        };
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/resources")
                    .body(Body::empty())
                    .expect("public catalog request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
