use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use shennong_os_server::{AppConfig, build_state, router};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

const OS_ORIGIN: &str = "https://os.test";

#[derive(Clone, Debug)]
struct Principal {
    label: &'static str,
    id: Uuid,
    role: String,
    cookie: String,
    csrf: String,
}

#[derive(Debug)]
struct Invite {
    id: Uuid,
    code: String,
}

async fn disposable_pool(database_url: &str) -> PgPool {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect to TEST_DATABASE_URL");
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("read database name");
    assert!(
        database.ends_with("_test") || database.starts_with("test_"),
        "refusing to modify non-test database {database:?}; use a name starting with test_ or ending with _test"
    );
    pool
}

fn authenticated(request: RequestBuilder, principal: &Principal) -> RequestBuilder {
    request.header("Cookie", &principal.cookie)
}

fn mutation(request: RequestBuilder, principal: &Principal) -> RequestBuilder {
    authenticated(request, principal)
        .header("Origin", OS_ORIGIN)
        .header("x-csrf-token", &principal.csrf)
}

async fn expect_status(response: Response, expected: StatusCode, context: &str) -> Response {
    let actual = response.status();
    if actual != expected {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("<unreadable response: {error}>"));
        panic!("{context}: expected {expected}, got {actual}: {body}");
    }
    response
}

async fn json_body(response: Response, context: &str) -> Value {
    response
        .json()
        .await
        .unwrap_or_else(|error| panic!("{context}: response is not JSON: {error}"))
}

async fn error_code(response: Response, expected_status: StatusCode, context: &str) -> String {
    let body = json_body(
        expect_status(response, expected_status, context).await,
        context,
    )
    .await;
    body.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error code missing from {body}"))
        .to_owned()
}

async fn principal_from_response(
    response: Response,
    expected_status: StatusCode,
    label: &'static str,
) -> Principal {
    let response = expect_status(response, expected_status, label).await;
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("shennong_os_session="))
        .and_then(|value| value.split(';').next())
        .unwrap_or_else(|| panic!("{label}: session cookie missing"))
        .to_owned();
    let body = json_body(response, label).await;
    Principal {
        label,
        id: body
            .pointer("/data/id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{label}: user id missing from {body}")),
        role: body
            .pointer("/data/role")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{label}: role missing from {body}"))
            .to_owned(),
        csrf: body
            .pointer("/data/csrf_token")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{label}: CSRF token missing from {body}"))
            .to_owned(),
        cookie,
    }
}

async fn create_invite(
    client: &reqwest::Client,
    base: &str,
    admin: &Principal,
    email_constraint: Option<&str>,
    max_uses: i32,
    note: &str,
) -> Invite {
    let response = mutation(client.post(format!("{base}/api/v1/admin/invites")), admin)
        .json(&json!({
            "email_constraint": email_constraint,
            "max_uses": max_uses,
            "expires_in_seconds": 3600,
            "note": note
        }))
        .send()
        .await
        .expect("create invite response");
    let body = json_body(
        expect_status(response, StatusCode::CREATED, "create invite").await,
        "create invite",
    )
    .await;
    Invite {
        id: body["data"]["id"]
            .as_str()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("invite id missing from {body}")),
        code: body["data"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("one-time invite code missing from {body}"))
            .to_owned(),
    }
}

async fn register_response(
    client: &reqwest::Client,
    base: &str,
    email: &str,
    display_name: &str,
    invite_code: Option<&str>,
) -> Response {
    client
        .post(format!("{base}/api/v1/auth/register"))
        .header("Origin", OS_ORIGIN)
        .json(&json!({
            "email": email,
            "display_name": display_name,
            "password": "a sufficiently long registration password",
            "invite_code": invite_code,
            // Unknown client authority must never affect the persisted role.
            "role": "admin"
        }))
        .send()
        .await
        .expect("registration response")
}

async fn register(
    client: &reqwest::Client,
    base: &str,
    label: &'static str,
    email: &str,
    invite_code: Option<&str>,
) -> Principal {
    principal_from_response(
        register_response(client, base, email, label, invite_code).await,
        StatusCode::CREATED,
        label,
    )
    .await
}

async fn assert_cross_id_404(
    client: &reqwest::Client,
    principal: &Principal,
    known_url: String,
    guessed_url: String,
    label: &str,
) {
    for (kind, url) in [("known foreign", known_url), ("random", guessed_url)] {
        let response = authenticated(client.get(url), principal)
            .send()
            .await
            .unwrap_or_else(|error| panic!("{label} {kind} request failed: {error}"));
        expect_status(
            response,
            StatusCode::NOT_FOUND,
            &format!("{label} {kind} id"),
        )
        .await;
    }
}

/// Full disposable-PostgreSQL identity and authorization gate.
///
/// Run this test by itself so its deliberate `TRUNCATE ... CASCADE` cannot race
/// another integration binary:
///
/// `TEST_DATABASE_URL=postgres://.../shennong_os_test cargo test --test identity_rbac_integration -- --ignored --test-threads=1`
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL TEST_DATABASE_URL"]
async fn bootstrap_invites_and_project_rbac_are_fail_closed() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let pool = disposable_pool(&database_url).await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(&pool)
        .await
        .expect("clear disposable identity data");
    sqlx::query(
        "UPDATE os_settings SET registration_mode='invite_only',updated_at=NOW() WHERE singleton=TRUE",
    )
    .execute(&pool)
    .await
    .expect("reset registration policy");

    let state = build_state(AppConfig::for_test(database_url))
        .await
        .expect("build app state");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test app");
    });
    let base = format!("http://{address}");
    let client = reqwest::Client::new();

    let setup_before = json_body(
        client
            .get(format!("{base}/api/v1/setup/status"))
            .send()
            .await
            .expect("setup status before bootstrap"),
        "setup status before bootstrap",
    )
    .await;
    assert_eq!(setup_before["data"]["needs_setup"], true);

    let invalid_bootstrap = client
        .post(format!("{base}/api/v1/setup/admin"))
        .header("Origin", OS_ORIGIN)
        .header("x-shennong-bootstrap-token", "x".repeat(32))
        .json(&json!({
            "email": "system-admin@example.test",
            "display_name": "System administrator",
            "password": "a sufficiently long bootstrap password"
        }))
        .send()
        .await
        .expect("invalid bootstrap response");
    assert_eq!(
        error_code(
            invalid_bootstrap,
            StatusCode::UNAUTHORIZED,
            "invalid bootstrap token"
        )
        .await,
        "bootstrap_token_invalid"
    );
    let users_after_invalid: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("count users after invalid bootstrap");
    assert_eq!(users_after_invalid, 0);

    let bootstrap = client
        .post(format!("{base}/api/v1/setup/admin"))
        .header("Origin", OS_ORIGIN)
        .header("x-shennong-bootstrap-token", "b".repeat(32))
        .json(&json!({
            "email": "system-admin@example.test",
            "display_name": "System administrator",
            "password": "a sufficiently long bootstrap password"
        }))
        .send()
        .await
        .expect("bootstrap response");
    let system_admin =
        principal_from_response(bootstrap, StatusCode::CREATED, "system-admin").await;
    assert_eq!(system_admin.role, "admin");

    let replay = client
        .post(format!("{base}/api/v1/setup/admin"))
        .header("Origin", OS_ORIGIN)
        .header("x-shennong-bootstrap-token", "b".repeat(32))
        .json(&json!({
            "email": "replayed-admin@example.test",
            "display_name": "Replayed administrator",
            "password": "a sufficiently long replay password"
        }))
        .send()
        .await
        .expect("bootstrap replay response");
    assert_eq!(
        error_code(replay, StatusCode::CONFLICT, "bootstrap replay").await,
        "conflict"
    );
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(&pool)
        .await
        .expect("count administrators");
    assert_eq!(admins, 1);
    let setup_after = json_body(
        client
            .get(format!("{base}/api/v1/setup/status"))
            .send()
            .await
            .expect("setup status after bootstrap"),
        "setup status after bootstrap",
    )
    .await;
    assert_eq!(setup_after["data"]["needs_setup"], false);

    let csrf_rejected = authenticated(
        client
            .post(format!("{base}/api/v1/admin/invites"))
            .header("Origin", OS_ORIGIN)
            .json(&json!({"max_uses":1,"expires_in_seconds":3600})),
        &system_admin,
    )
    .send()
    .await
    .expect("missing CSRF response");
    assert_eq!(
        error_code(csrf_rejected, StatusCode::FORBIDDEN, "missing CSRF").await,
        "csrf_required"
    );

    let constrained = create_invite(
        &client,
        &base,
        &system_admin,
        Some("Project.Owner@Example.Test"),
        1,
        "one-time email-bound invite",
    )
    .await;
    let invite_list = json_body(
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/admin/invites")),
                &system_admin,
            )
            .send()
            .await
            .expect("list invites response"),
            StatusCode::OK,
            "list invites",
        )
        .await,
        "list invites",
    )
    .await;
    let listed_invite = invite_list["data"]
        .as_array()
        .expect("invite list array")
        .iter()
        .find(|value| value["id"] == constrained.id.to_string())
        .expect("created invite in list");
    assert!(
        listed_invite.get("code").is_none(),
        "full invite code must only be returned by create"
    );
    assert!(
        !invite_list.to_string().contains(&constrained.code),
        "full invite code leaked from list"
    );

    let wrong_email = register_response(
        &client,
        &base,
        "not-owner@example.test",
        "wrong-email",
        Some(&constrained.code),
    )
    .await;
    assert_eq!(
        error_code(
            wrong_email,
            StatusCode::FORBIDDEN,
            "email-constrained invite"
        )
        .await,
        "invite_unavailable"
    );
    let constrained_uses: i32 =
        sqlx::query_scalar("SELECT use_count FROM registration_invites WHERE id=$1")
            .bind(constrained.id)
            .fetch_one(&pool)
            .await
            .expect("read constrained invite uses");
    assert_eq!(constrained_uses, 0);
    let owner = register(
        &client,
        &base,
        "owner",
        "project.owner@example.test",
        Some(&constrained.code),
    )
    .await;
    assert_eq!(owner.role, "user");
    let replayed_invite = register_response(
        &client,
        &base,
        "project.owner@example.test",
        "replayed-invite",
        Some(&constrained.code),
    )
    .await;
    assert_eq!(
        error_code(
            replayed_invite,
            StatusCode::FORBIDDEN,
            "consumed invite replay"
        )
        .await,
        "invite_unavailable"
    );

    let expired = create_invite(&client, &base, &system_admin, None, 1, "expired invite").await;
    sqlx::query("UPDATE registration_invites SET expires_at=NOW()-INTERVAL '1 second' WHERE id=$1")
        .bind(expired.id)
        .execute(&pool)
        .await
        .expect("expire invite");
    let expired_response = register_response(
        &client,
        &base,
        "expired@example.test",
        "expired",
        Some(&expired.code),
    )
    .await;
    assert_eq!(
        error_code(expired_response, StatusCode::FORBIDDEN, "expired invite").await,
        "invite_unavailable"
    );

    let revoked = create_invite(&client, &base, &system_admin, None, 1, "revoked invite").await;
    expect_status(
        mutation(
            client.delete(format!("{base}/api/v1/admin/invites/{}", revoked.id)),
            &system_admin,
        )
        .send()
        .await
        .expect("revoke invite response"),
        StatusCode::NO_CONTENT,
        "revoke invite",
    )
    .await;
    let revoked_response = register_response(
        &client,
        &base,
        "revoked@example.test",
        "revoked",
        Some(&revoked.code),
    )
    .await;
    assert_eq!(
        error_code(revoked_response, StatusCode::FORBIDDEN, "revoked invite").await,
        "invite_unavailable"
    );

    let concurrent = create_invite(
        &client,
        &base,
        &system_admin,
        None,
        1,
        "concurrent single-use invite",
    )
    .await;
    let first_concurrent = register_response(
        &client,
        &base,
        "concurrent-a@example.test",
        "concurrent-a",
        Some(&concurrent.code),
    );
    let second_concurrent = register_response(
        &client,
        &base,
        "concurrent-b@example.test",
        "concurrent-b",
        Some(&concurrent.code),
    );
    let (first_concurrent, second_concurrent) = tokio::join!(first_concurrent, second_concurrent);
    let concurrent_responses = [first_concurrent, second_concurrent];
    let concurrent_statuses = concurrent_responses
        .iter()
        .map(Response::status)
        .collect::<Vec<_>>();
    assert_eq!(
        concurrent_statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "one concurrent redemption must win: {concurrent_statuses:?}"
    );
    assert_eq!(
        concurrent_statuses
            .iter()
            .filter(|status| **status == StatusCode::FORBIDDEN)
            .count(),
        1,
        "one concurrent redemption must fail closed: {concurrent_statuses:?}"
    );
    for response in concurrent_responses {
        let status = response.status();
        let body = json_body(response, "concurrent invite response").await;
        if status == StatusCode::CREATED {
            assert_eq!(body["data"]["role"], "user");
        } else {
            assert_eq!(body["error"]["code"], "invite_unavailable");
        }
    }
    let concurrent_contract: (i32, i64) = sqlx::query_as(
        "SELECT i.use_count,COUNT(r.id) FROM registration_invites i \
         LEFT JOIN registration_invite_redemptions r ON r.invite_id=i.id \
         WHERE i.id=$1 GROUP BY i.id",
    )
    .bind(concurrent.id)
    .fetch_one(&pool)
    .await
    .expect("read concurrent invite contract");
    assert_eq!(concurrent_contract, (1, 1));

    expect_status(
        mutation(
            client.patch(format!("{base}/api/v1/admin/registration-policy")),
            &system_admin,
        )
        .json(&json!({"registration_mode":"open"}))
        .send()
        .await
        .expect("enable open registration response"),
        StatusCode::OK,
        "enable open registration",
    )
    .await;
    let open_user = register(&client, &base, "open-user", "open-user@example.test", None).await;
    assert_eq!(open_user.role, "user");
    expect_status(
        mutation(
            client.patch(format!("{base}/api/v1/admin/registration-policy")),
            &system_admin,
        )
        .json(&json!({"registration_mode":"invite_only"}))
        .send()
        .await
        .expect("restore invite registration response"),
        StatusCode::OK,
        "restore invite registration",
    )
    .await;

    let role_invite = create_invite(
        &client,
        &base,
        &system_admin,
        None,
        4,
        "project RBAC principals",
    )
    .await;
    let project_admin = register(
        &client,
        &base,
        "project-admin",
        "project-admin@example.test",
        Some(&role_invite.code),
    )
    .await;
    let editor = register(
        &client,
        &base,
        "editor",
        "editor@example.test",
        Some(&role_invite.code),
    )
    .await;
    let viewer = register(
        &client,
        &base,
        "viewer",
        "viewer@example.test",
        Some(&role_invite.code),
    )
    .await;
    let outsider = register(
        &client,
        &base,
        "outsider",
        "outsider@example.test",
        Some(&role_invite.code),
    )
    .await;
    for principal in [
        &owner,
        &project_admin,
        &editor,
        &viewer,
        &outsider,
        &open_user,
    ] {
        assert_eq!(
            principal.role, "user",
            "{} gained a system role through registration",
            principal.label
        );
    }
    let non_bootstrap_admins: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin' AND id<>$1")
            .bind(system_admin.id)
            .fetch_one(&pool)
            .await
            .expect("count unexpected administrators");
    assert_eq!(non_bootstrap_admins, 0);

    let project_response = mutation(client.post(format!("{base}/api/v1/projects")), &owner)
        .json(&json!({
            "name":"Identity RBAC project",
            "description":"Disposable authorization fixture",
            "visibility":"private"
        }))
        .send()
        .await
        .expect("create project response");
    let project_body = json_body(
        expect_status(project_response, StatusCode::CREATED, "create project").await,
        "create project",
    )
    .await;
    let project_id: Uuid = project_body["data"]["id"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .expect("project id");
    assert_eq!(project_body["data"]["member_role"], "owner");

    for (principal, role) in [
        (&project_admin, "admin"),
        (&editor, "editor"),
        (&viewer, "viewer"),
    ] {
        expect_status(
            mutation(
                client.put(format!(
                    "{base}/api/v1/projects/{project_id}/members/{}",
                    principal.id
                )),
                &owner,
            )
            .json(&json!({"role":role}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("assign {role}: {error}")),
            StatusCode::OK,
            &format!("assign {role}"),
        )
        .await;
    }

    let project_readers = [&system_admin, &owner, &project_admin, &editor, &viewer];
    let project_writers = [&system_admin, &owner, &project_admin, &editor];
    let project_managers = [&system_admin, &owner, &project_admin];

    for principal in project_readers {
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/projects/{project_id}")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} project read: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} project read", principal.label),
        )
        .await;
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/projects/{project_id}/members")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} member read: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} member read", principal.label),
        )
        .await;
    }
    for principal in project_managers {
        expect_status(
            mutation(
                client.patch(format!("{base}/api/v1/projects/{project_id}")),
                principal,
            )
            .json(&json!({"description":format!("managed by {}", principal.label)}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} project manage: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} project manage", principal.label),
        )
        .await;
    }
    for principal in [&editor, &viewer, &outsider] {
        expect_status(
            mutation(
                client.patch(format!("{base}/api/v1/projects/{project_id}")),
                principal,
            )
            .json(&json!({"description":"must not be applied"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied project manage: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied project manage", principal.label),
        )
        .await;
    }

    let canonical_thread = {
        let response = mutation(client.post(format!("{base}/api/v1/threads")), &owner)
            .json(&json!({"project_id":project_id,"title":"Canonical RBAC thread"}))
            .send()
            .await
            .expect("owner create thread response");
        let body = json_body(
            expect_status(response, StatusCode::CREATED, "owner create thread").await,
            "owner create thread",
        )
        .await;
        body["data"]["id"]
            .as_str()
            .and_then(|value| value.parse::<Uuid>().ok())
            .expect("canonical thread id")
    };
    for principal in [&system_admin, &project_admin, &editor] {
        expect_status(
            mutation(client.post(format!("{base}/api/v1/threads")), principal)
                .json(&json!({
                    "project_id":project_id,
                    "title":format!("{} writable thread",principal.label)
                }))
                .send()
                .await
                .unwrap_or_else(|error| panic!("{} create thread: {error}", principal.label)),
            StatusCode::CREATED,
            &format!("{} create thread", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(client.post(format!("{base}/api/v1/threads")), principal)
                .json(&json!({"project_id":project_id,"title":"forbidden thread"}))
                .send()
                .await
                .unwrap_or_else(|error| {
                    panic!("{} denied thread create: {error}", principal.label)
                }),
            StatusCode::NOT_FOUND,
            &format!("{} denied thread create", principal.label),
        )
        .await;
    }
    for principal in project_readers {
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/threads/{canonical_thread}")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} thread read: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} thread read", principal.label),
        )
        .await;
    }
    for principal in project_writers {
        expect_status(
            mutation(
                client.patch(format!("{base}/api/v1/threads/{canonical_thread}")),
                principal,
            )
            .json(&json!({"title":format!("updated by {}",principal.label)}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} thread update: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} thread update", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(
                client.patch(format!("{base}/api/v1/threads/{canonical_thread}")),
                principal,
            )
            .json(&json!({"title":"must not be applied"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied thread update: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied thread update", principal.label),
        )
        .await;
    }
    for (index, principal) in project_writers.into_iter().enumerate() {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/threads/{canonical_thread}/messages")),
                principal,
            )
            .header("Idempotency-Key", format!("rbac-message-{index:02}"))
            .json(&json!({"role":"user","content_json":{"text":principal.label}}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} create message: {error}", principal.label)),
            StatusCode::CREATED,
            &format!("{} create message", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/threads/{canonical_thread}/messages")),
                principal,
            )
            .header(
                "Idempotency-Key",
                format!("denied-{}-message", principal.label),
            )
            .json(&json!({"role":"user","content_json":{"text":"denied"}}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied message create: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied message create", principal.label),
        )
        .await;
    }

    let canonical_job = {
        let response = mutation(
            client.post(format!("{base}/api/v1/projects/{project_id}/jobs")),
            &owner,
        )
        .json(&json!({"kind":"analysis","spec":{"fixture":true}}))
        .send()
        .await
        .expect("owner create job response");
        let body = json_body(
            expect_status(response, StatusCode::CREATED, "owner create job").await,
            "owner create job",
        )
        .await;
        body["data"]["id"]
            .as_str()
            .and_then(|value| value.parse::<Uuid>().ok())
            .expect("canonical job id")
    };
    for principal in [&system_admin, &project_admin, &editor] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/jobs")),
                principal,
            )
            .json(&json!({"kind":"analysis","spec":{"actor":principal.label}}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} create job: {error}", principal.label)),
            StatusCode::CREATED,
            &format!("{} create job", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/jobs")),
                principal,
            )
            .json(&json!({"kind":"analysis","spec":{}}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied job create: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied job create", principal.label),
        )
        .await;
    }
    for principal in project_readers {
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/jobs/{canonical_job}")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} job read: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} job read", principal.label),
        )
        .await;
    }
    for principal in project_writers {
        expect_status(
            mutation(
                client.patch(format!("{base}/api/v1/jobs/{canonical_job}")),
                principal,
            )
            .json(&json!({"status":"queued","result":{}}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} job update: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} job update", principal.label),
        )
        .await;
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/jobs/{canonical_job}/cancel")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} job cancel boundary: {error}", principal.label)),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} job cancel reached runtime boundary", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        for (method, label) in [("update", "PATCH"), ("cancel", "POST")] {
            let request = if label == "PATCH" {
                mutation(
                    client
                        .patch(format!("{base}/api/v1/jobs/{canonical_job}"))
                        .json(&json!({"status":"queued","result":{}})),
                    principal,
                )
            } else {
                mutation(
                    client.post(format!("{base}/api/v1/jobs/{canonical_job}/cancel")),
                    principal,
                )
            };
            expect_status(
                request.send().await.unwrap_or_else(|error| {
                    panic!("{} denied job {method}: {error}", principal.label)
                }),
                StatusCode::NOT_FOUND,
                &format!("{} denied job {method}", principal.label),
            )
            .await;
        }
    }

    let runtime_session = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runtime_sessions(\
           id,project_id,created_by_user_id,kind,worker_profile,status,runtime_view,expires_at\
         ) VALUES($1,$2,$3,'jupyterlab','ide-small','starting','{}'::jsonb,NOW()+INTERVAL '1 hour')",
    )
    .bind(runtime_session)
    .bind(project_id)
    .bind(owner.id)
    .execute(&pool)
    .await
    .expect("insert IDE authorization fixture");
    for principal in project_readers {
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/projects/{project_id}/sessions")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} IDE list: {error}", principal.label)),
            StatusCode::OK,
            &format!("{} IDE list", principal.label),
        )
        .await;
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/sessions/{runtime_session}")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} IDE read boundary: {error}", principal.label)),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} IDE read reached runtime boundary", principal.label),
        )
        .await;
    }
    for principal in project_writers {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/sessions")),
                principal,
            )
            .json(&json!({"kind":"jupyterlab","worker_profile":"ide-small"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} IDE create boundary: {error}", principal.label)),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} IDE create reached runtime boundary", principal.label),
        )
        .await;
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/sessions/{runtime_session}/stop")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} IDE stop boundary: {error}", principal.label)),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} IDE stop reached runtime boundary", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/sessions")),
                principal,
            )
            .json(&json!({"kind":"jupyterlab"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied IDE create: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied IDE create", principal.label),
        )
        .await;
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/sessions/{runtime_session}/stop")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied IDE stop: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied IDE stop", principal.label),
        )
        .await;
    }
    expect_status(
        mutation(
            client.post(format!("{base}/api/v1/sessions/{runtime_session}/launch")),
            &owner,
        )
        .send()
        .await
        .expect("IDE owner launch boundary response"),
        StatusCode::SERVICE_UNAVAILABLE,
        "IDE owner launch reached runtime boundary",
    )
    .await;
    for principal in [&system_admin, &project_admin, &editor, &viewer, &outsider] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/sessions/{runtime_session}/launch")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied IDE launch: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied another user's IDE launch", principal.label),
        )
        .await;
    }

    for principal in project_readers {
        expect_status(
            authenticated(
                client.get(format!("{base}/api/v1/projects/{project_id}/resources")),
                principal,
            )
            .send()
            .await
            .unwrap_or_else(|error| {
                panic!("{} data-plane read boundary: {error}", principal.label)
            }),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} data-plane read reached DB boundary", principal.label),
        )
        .await;
    }
    for principal in project_writers {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/entities")),
                principal,
            )
            .json(&json!({"kind":"test"}))
            .send()
            .await
            .unwrap_or_else(|error| {
                panic!("{} data-plane write boundary: {error}", principal.label)
            }),
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{} data-plane write reached DB boundary", principal.label),
        )
        .await;
    }
    for principal in [&viewer, &outsider] {
        expect_status(
            mutation(
                client.post(format!("{base}/api/v1/projects/{project_id}/entities")),
                principal,
            )
            .json(&json!({"kind":"denied"}))
            .send()
            .await
            .unwrap_or_else(|error| panic!("{} denied data-plane write: {error}", principal.label)),
            StatusCode::NOT_FOUND,
            &format!("{} denied data-plane write", principal.label),
        )
        .await;
    }

    let random_project = Uuid::new_v4();
    let random_thread = Uuid::new_v4();
    let random_job = Uuid::new_v4();
    let random_session = Uuid::new_v4();
    assert_cross_id_404(
        &client,
        &outsider,
        format!("{base}/api/v1/projects/{project_id}"),
        format!("{base}/api/v1/projects/{random_project}"),
        "project",
    )
    .await;
    assert_cross_id_404(
        &client,
        &outsider,
        format!("{base}/api/v1/threads/{canonical_thread}"),
        format!("{base}/api/v1/threads/{random_thread}"),
        "thread",
    )
    .await;
    assert_cross_id_404(
        &client,
        &outsider,
        format!("{base}/api/v1/jobs/{canonical_job}"),
        format!("{base}/api/v1/jobs/{random_job}"),
        "job",
    )
    .await;
    assert_cross_id_404(
        &client,
        &outsider,
        format!("{base}/api/v1/sessions/{runtime_session}"),
        format!("{base}/api/v1/sessions/{random_session}"),
        "IDE session",
    )
    .await;
    assert_cross_id_404(
        &client,
        &outsider,
        format!("{base}/api/v1/projects/{project_id}/resources"),
        format!("{base}/api/v1/projects/{random_project}/resources"),
        "DB data-plane project",
    )
    .await;

    let outsider_projects = json_body(
        authenticated(client.get(format!("{base}/api/v1/projects")), &outsider)
            .send()
            .await
            .expect("outsider project list"),
        "outsider project list",
    )
    .await;
    assert_eq!(outsider_projects["data"], json!([]));
    let outsider_threads = json_body(
        authenticated(client.get(format!("{base}/api/v1/threads")), &outsider)
            .send()
            .await
            .expect("outsider thread list"),
        "outsider thread list",
    )
    .await;
    assert_eq!(outsider_threads["data"], json!([]));
    let outsider_jobs = json_body(
        authenticated(client.get(format!("{base}/api/v1/jobs")), &outsider)
            .send()
            .await
            .expect("outsider job list"),
        "outsider job list",
    )
    .await;
    assert_eq!(outsider_jobs["data"], json!([]));

    server.abort();
}
