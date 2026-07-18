use reqwest::StatusCode;
use serde_json::json;
use sha2::{Digest, Sha256};
use shennong_os_server::{AppConfig, build_state, router};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

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

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).expect("serialize canonical string")
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize canonical key"),
                        canonical_json(&values[*key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn tool_arguments_digest(tool_name: &str, arguments: &serde_json::Value) -> String {
    hex::encode(Sha256::digest(
        format!("{tool_name}\0{}", canonical_json(arguments)).as_bytes(),
    ))
}

async fn insert_running_approval_run(
    pool: &PgPool,
    project_id: Uuid,
    thread_id: Uuid,
    user_id: Uuid,
    capability: &str,
) -> Uuid {
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs(id,project_id,thread_id,requested_by_user_id,status,input, \
         capability_token_hash,capability_expires_at,started_at) \
         VALUES($1,$2,$3,$4,'running',$5,$6,NOW()+INTERVAL '15 minutes',NOW())",
    )
    .bind(run_id)
    .bind(project_id)
    .bind(thread_id)
    .bind(user_id)
    .bind(json!({
        "allowed_tools":["project.write_file"],
        "tool_profile":"project-write",
        "allowed_project_read":["project://current/"],
        "allowed_project_write":["project://current/README.md"],
        "allowed_compute_profiles":[],
        "required_approvals":["project.write"]
    }))
    .bind(Sha256::digest(capability).to_vec())
    .execute(pool)
    .await
    .expect("insert approval test run");
    run_id
}

/// Run with:
/// `TEST_DATABASE_URL=postgres://.../shennong_os_test cargo test --test postgres_integration -- --ignored`
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL TEST_DATABASE_URL"]
async fn migrations_bootstrap_and_agent_gateway_security_contracts() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let pool = disposable_pool(&database_url).await;

    // This test owns the explicitly disposable database for its full run. A
    // schema reset makes it repeatable without TRUNCATE ... CASCADE erasing
    // globally seeded built-in Skills through nullable user foreign keys.
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .expect("reset disposable test schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("create disposable test schema");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    let builtin_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM skills s \
         JOIN skill_versions v ON v.skill_id=s.id AND v.version=s.current_version \
         WHERE s.owner_user_id IS NULL AND s.trust_level='builtin_signed' \
         AND s.lifecycle='active' \
         AND v.package_version='1.0.0' \
         AND jsonb_typeof(s.manifest->'spec'->'permissions')='object'",
    )
    .fetch_one(&pool)
    .await
    .expect("count seeded built-in skills");
    assert_eq!(builtin_count, 7);

    let builtin_sources = [
        (
            "discover-shennong-data",
            1,
            include_str!("../../../skills/discover-shennong-data/SKILL.md"),
        ),
        (
            "initialize-biomedical-project",
            1,
            include_str!("../../../skills/initialize-biomedical-project/SKILL.md"),
        ),
        (
            "inspect-biomedical-input",
            1,
            include_str!("../../../skills/inspect-biomedical-input/SKILL.md"),
        ),
        (
            "interpret-biomedical-results",
            1,
            include_str!("../../../skills/interpret-biomedical-results/SKILL.md"),
        ),
        (
            "manage-analysis-results",
            1,
            include_str!("../../../skills/manage-analysis-results/SKILL.md"),
        ),
        (
            "run-shennong-single-cell-workflow",
            2,
            include_str!("../../../skills/run-shennong-single-cell-workflow/SKILL.md"),
        ),
        (
            "validate-biomedical-analysis",
            1,
            include_str!("../../../skills/validate-biomedical-analysis/SKILL.md"),
        ),
    ];
    for (slug, version, source) in builtin_sources {
        let (stored_content, stored_digest): (String, String) = sqlx::query_as(
            "SELECT v.content,v.content_sha256 FROM skills s \
             JOIN skill_versions v ON v.skill_id=s.id AND v.version=s.current_version \
             WHERE s.slug=$1 AND s.owner_user_id IS NULL",
        )
        .bind(slug)
        .fetch_one(&pool)
        .await
        .expect("read seeded built-in skill");
        assert_eq!(stored_content, source, "seeded content differs for {slug}");
        assert_eq!(
            stored_digest,
            hex::encode(Sha256::digest(source.as_bytes())),
            "seeded digest differs for {slug}"
        );
        let current_version: i32 = sqlx::query_scalar(
            "SELECT current_version FROM skills WHERE slug=$1 AND owner_user_id IS NULL",
        )
        .bind(slug)
        .fetch_one(&pool)
        .await
        .expect("read current built-in version");
        assert_eq!(current_version, version, "unexpected version for {slug}");
    }

    let workflow_contract: serde_json::Value = sqlx::query_scalar(
        "SELECT manifest FROM skills WHERE slug='run-shennong-single-cell-workflow' \
         AND owner_user_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("read workflow contract");
    assert_eq!(
        workflow_contract.pointer("/metadata/revision"),
        Some(&json!(2))
    );
    assert_eq!(
        workflow_contract.pointer("/spec/permissions/computeProfiles"),
        Some(&json!(["cpu-small"]))
    );
    assert!(
        !workflow_contract["spec"]["permissions"]["tools"]
            .as_array()
            .expect("tool list")
            .contains(&json!("environment.ensure"))
    );

    let project_files_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.project_files')::text")
            .fetch_one(&pool)
            .await
            .expect("resolve project_files table");
    assert_eq!(project_files_table.as_deref(), Some("project_files"));

    sqlx::query(
        "UPDATE os_settings SET registration_mode='invite_only', updated_at=NOW() WHERE singleton=TRUE",
    )
    .execute(&pool)
    .await
    .expect("reset settings");

    let mode: String =
        sqlx::query_scalar("SELECT registration_mode FROM os_settings WHERE singleton=TRUE")
            .fetch_one(&pool)
            .await
            .expect("read registration mode");
    assert_eq!(mode, "invite_only");

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

    let endpoint = format!("http://{address}/api/v1/setup/admin");
    let client = reqwest::Client::new();
    let first = client
        .post(&endpoint)
        .header("Origin", "https://os.test")
        .header("x-shennong-bootstrap-token", "b".repeat(32))
        .json(&json!({
            "email": "first-admin@example.test",
            "display_name": "First administrator",
            "password": "a sufficiently long bootstrap password"
        }))
        .send();
    let second = client
        .post(&endpoint)
        .header("Origin", "https://os.test")
        .header("x-shennong-bootstrap-token", "b".repeat(32))
        .json(&json!({
            "email": "second-admin@example.test",
            "display_name": "Second administrator",
            "password": "another sufficiently long bootstrap password"
        }))
        .send();
    let (first, second) = tokio::join!(first, second);
    let statuses = [
        first.expect("first bootstrap response").status(),
        second.expect("second bootstrap response").status(),
    ];
    assert!(statuses.contains(&StatusCode::CREATED), "{statuses:?}");
    assert!(statuses.contains(&StatusCode::CONFLICT), "{statuses:?}");

    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(&pool)
        .await
        .expect("count admins");
    assert_eq!(admins, 1);

    let user_id = Uuid::new_v4();
    let project_a = Uuid::new_v4();
    let project_b = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users(id,email,email_normalized,display_name,username,password_hash,role) \
         VALUES($1,'agent-user@example.test','agent-user@example.test','Agent user','agent-user','test-only','user')",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert gateway test user");
    for (project_id, name) in [(project_a, "Project A"), (project_b, "Project B")] {
        sqlx::query("INSERT INTO projects(id,owner_user_id,name) VALUES($1,$2,$3)")
            .bind(project_id)
            .bind(user_id)
            .bind(name)
            .execute(&pool)
            .await
            .expect("insert gateway test project");
        sqlx::query("INSERT INTO project_members(project_id,user_id,role) VALUES($1,$2,'owner')")
            .bind(project_id)
            .bind(user_id)
            .execute(&pool)
            .await
            .expect("insert gateway test membership");
    }
    let session_token = "gateway-session-token-with-enough-entropy-for-test";
    let csrf_token = "gateway-csrf-token-with-enough-entropy";
    sqlx::query(
        "INSERT INTO sessions(id,user_id,token_hash,csrf_hash,expires_at) \
         VALUES($1,$2,$3,$4,NOW()+INTERVAL '1 hour')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(Sha256::digest(session_token).to_vec())
    .bind(Sha256::digest(csrf_token).to_vec())
    .execute(&pool)
    .await
    .expect("insert gateway test session");
    let gateway = format!("http://{address}/api/v1/agent");
    let request = |thread_id: Uuid, run_id: Uuid| {
        client
            .post(&gateway)
            .header("Origin", "https://os.test")
            .header("Cookie", format!("shennong_os_session={session_token}"))
            .header("x-csrf-token", csrf_token)
            .json(&json!({
                "threadId":thread_id,
                "runId":run_id,
                "messages":[{"role":"user","content":"test"}]
            }))
    };

    let missing_project = request(Uuid::new_v4(), Uuid::new_v4())
        .send()
        .await
        .expect("missing project response");
    assert_eq!(missing_project.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let missing_error: serde_json::Value = missing_project
        .json()
        .await
        .expect("missing project error body");
    assert_eq!(missing_error["error"]["code"], "project_required");

    let existing_thread = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO threads(id,project_id,owner_user_id,title) VALUES($1,$2,$3,'Existing')",
    )
    .bind(existing_thread)
    .bind(project_a)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert existing thread");
    let cross_project_run = Uuid::new_v4();
    let cross_project = request(existing_thread, cross_project_run)
        .header("x-shennong-project-id", project_b.to_string())
        .send()
        .await
        .expect("cross-project response");
    assert_eq!(cross_project.status(), StatusCode::NOT_FOUND);
    let cross_project_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE id=$1")
        .bind(cross_project_run)
        .fetch_one(&pool)
        .await
        .expect("count cross-project runs");
    assert_eq!(cross_project_runs, 0);

    let concurrent_thread = Uuid::new_v4();
    let first_run = Uuid::new_v4();
    let second_run = Uuid::new_v4();
    let first = request(concurrent_thread, first_run)
        .header("x-shennong-project-id", project_a.to_string())
        .send();
    let second = request(concurrent_thread, second_run)
        .header("x-shennong-project-id", project_a.to_string())
        .send();
    let (first, second) = tokio::join!(first, second);
    assert_eq!(
        first.expect("first concurrent gateway response").status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        second.expect("second concurrent gateway response").status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let concurrent_threads: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM threads WHERE id=$1 AND project_id=$2")
            .bind(concurrent_thread)
            .bind(project_a)
            .fetch_one(&pool)
            .await
            .expect("count concurrent threads");
    assert_eq!(concurrent_threads, 1);
    let concurrent_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE thread_id=$1")
        .bind(concurrent_thread)
        .fetch_one(&pool)
        .await
        .expect("count concurrent runs");
    assert_eq!(concurrent_runs, 2);

    let replay_run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs(id,project_id,thread_id,requested_by_user_id,status,finished_at) \
         VALUES($1,$2,$3,$4,'succeeded',NOW())",
    )
    .bind(replay_run)
    .bind(project_a)
    .bind(existing_thread)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert replay test run");
    let replay_payloads = [
        json!({"type":"RUN_STARTED","threadId":existing_thread,"runId":replay_run}),
        json!({"type":"TEXT_MESSAGE_START","messageId":"assistant-replay"}),
        json!({"type":"TEXT_MESSAGE_CONTENT","messageId":"assistant-replay","delta":"durable"}),
        json!({"type":"RUN_FINISHED","threadId":existing_thread,"runId":replay_run}),
    ];
    let mut replay_cursors = Vec::new();
    for payload in replay_payloads {
        let event_type = payload["type"].as_str().expect("event type").to_owned();
        let cursor: i64 = sqlx::query_scalar(
            "INSERT INTO run_events(run_id,event_type,payload) VALUES($1,$2,$3) RETURNING id",
        )
        .bind(replay_run)
        .bind(event_type)
        .bind(payload)
        .fetch_one(&pool)
        .await
        .expect("insert replay event");
        replay_cursors.push(cursor);
    }
    let events_url = format!("http://{address}/api/v1/runs/{replay_run}/events");
    let first_page: serde_json::Value = client
        .get(format!("{events_url}?after=0&limit=2"))
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .send()
        .await
        .expect("first replay page")
        .error_for_status()
        .expect("first replay status")
        .json()
        .await
        .expect("first replay payload");
    assert_eq!(first_page["data"][0]["id"], json!(replay_cursors[0]));
    assert_eq!(first_page["data"][1]["id"], json!(replay_cursors[1]));

    let mismatch = client
        .get(format!("{events_url}?after={}", replay_cursors[1]))
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("Last-Event-ID", replay_cursors[0].to_string())
        .send()
        .await
        .expect("mismatched cursor response");
    assert_eq!(mismatch.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let replay_stream = client
        .get(format!(
            "{events_url}/stream?after={}&limit=2",
            replay_cursors[1]
        ))
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("Last-Event-ID", replay_cursors[1].to_string())
        .send()
        .await
        .expect("replay stream response")
        .error_for_status()
        .expect("replay stream status")
        .text()
        .await
        .expect("replay stream body");
    assert!(!replay_stream.contains(&format!("id: {}\n", replay_cursors[1])));
    assert_eq!(
        replay_stream
            .matches(&format!("id: {}\n", replay_cursors[2]))
            .count(),
        1
    );
    assert_eq!(
        replay_stream
            .matches(&format!("id: {}\n", replay_cursors[3]))
            .count(),
        1
    );
    assert!(replay_stream.contains("\"type\":\"RUN_FINISHED\""));

    let provider_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO model_providers(id,owner_user_id,name,provider_kind,base_url,model, \
         data_policy,enabled,is_default) \
         VALUES($1,$2,'Approval test provider','ollama','http://127.0.0.1:11434/v1', \
         'approval-test','allow_private',TRUE,TRUE)",
    )
    .bind(provider_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert approval test provider");
    sqlx::query("UPDATE threads SET provider_id=$2 WHERE id=$1")
        .bind(existing_thread)
        .bind(provider_id)
        .execute(&pool)
        .await
        .expect("select approval test provider");
    sqlx::query(
        "INSERT INTO thread_skills(thread_id,skill_id,skill_version) \
         SELECT $1,id,current_version FROM skills \
         WHERE slug='initialize-biomedical-project' AND owner_user_id IS NULL",
    )
    .bind(existing_thread)
    .execute(&pool)
    .await
    .expect("select approval test skill");

    let service_token = "o".repeat(32);
    let capability = "approval-run-capability-with-enough-entropy";
    let approved_arguments = json!({
        "uri":"project://current/README.md",
        "content":"# Approved durable continuation\n"
    });
    let approved_digest = tool_arguments_digest("project.write_file", &approved_arguments);
    let original_run =
        insert_running_approval_run(&pool, project_a, existing_thread, user_id, capability).await;
    let approval_response = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{original_run}/approvals/verify"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&json!({
            "runId":original_run,
            "userId":user_id,
            "projectId":project_a,
            "toolCallId":"tool-write-approved",
            "toolName":"project.write_file",
            "argumentsDigest":approved_digest,
            "risk":"write",
            "runCapabilityToken":capability,
            "arguments":approved_arguments
        }))
        .send()
        .await
        .expect("request durable approval")
        .error_for_status()
        .expect("durable approval status")
        .json::<serde_json::Value>()
        .await
        .expect("durable approval response");
    assert_eq!(approval_response["data"]["allowed"], false);
    assert_eq!(approval_response["data"]["reason"], "approval_required");
    assert_eq!(approval_response["data"]["approvalScope"], "project.write");
    let approval_id = approval_response["data"]["approvalId"]
        .as_str()
        .expect("approval id")
        .parse::<Uuid>()
        .expect("approval UUID");
    let approval_contract = sqlx::query(
        "SELECT a.status,a.tool_call_id,a.tool_name,a.arguments_digest,a.arguments,a.approval_scope, \
                r.status AS run_status,r.capability_token_hash \
         FROM run_approvals a JOIN runs r ON r.id=a.run_id WHERE a.id=$1",
    )
    .bind(approval_id)
    .fetch_one(&pool)
    .await
    .expect("read durable approval contract");
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "status"),
        "pending"
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "run_status"),
        "waiting_approval"
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "tool_call_id"),
        "tool-write-approved"
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "tool_name"),
        "project.write_file"
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "arguments_digest"),
        approved_digest
    );
    assert_eq!(
        sqlx::Row::get::<serde_json::Value, _>(&approval_contract, "arguments"),
        approved_arguments
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approval_contract, "approval_scope"),
        "project.write"
    );
    assert!(
        sqlx::Row::get::<Option<Vec<u8>>, _>(&approval_contract, "capability_token_hash").is_none()
    );

    let malformed_child = Uuid::new_v4();
    let malformed_resume = client
        .post(&gateway)
        .header("Origin", "https://os.test")
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("x-csrf-token", csrf_token)
        .header("x-shennong-project-id", project_a.to_string())
        .json(&json!({
            "threadId":existing_thread,
            "runId":malformed_child,
            "messages":[{"role":"assistant","content":"approval pending"}],
            "resume":[{
                "interruptId":approval_id,
                "status":"resolved",
                "payload":{"approved":true,"arguments":approved_arguments}
            }]
        }))
        .send()
        .await
        .expect("tampered approval payload response");
    assert_eq!(malformed_resume.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let malformed_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE id=$1")
        .bind(malformed_child)
        .fetch_one(&pool)
        .await
        .expect("count malformed child run");
    assert_eq!(malformed_count, 0);

    // assistant-ui 0.0.45 sends only `resume[]`; OS derives and forwards the
    // immutable parent Run from the persisted interrupt instead of trusting a
    // browser-supplied parentRunId.
    let approved_child = Uuid::new_v4();
    let approved_resume = client
        .post(&gateway)
        .header("Origin", "https://os.test")
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("x-csrf-token", csrf_token)
        .header("x-shennong-project-id", project_a.to_string())
        .json(&json!({
            "threadId":existing_thread,
            "runId":approved_child,
            "messages":[{"role":"assistant","content":"approval pending"}],
            "resume":[{
                "interruptId":approval_id,
                "status":"resolved",
                "payload":{"approved":true}
            }]
        }))
        .send()
        .await
        .expect("approved continuation response");
    assert_eq!(approved_resume.status(), StatusCode::SERVICE_UNAVAILABLE);
    let approved_lineage = sqlx::query(
        "SELECT r.parent_run_id,r.status,r.input,a.status AS approval_status,a.resumed_run_id, \
                original.status AS original_status \
         FROM runs r JOIN run_approvals a ON a.id=$2 \
         JOIN runs original ON original.id=a.run_id WHERE r.id=$1",
    )
    .bind(approved_child)
    .bind(approval_id)
    .fetch_one(&pool)
    .await
    .expect("read approved child lineage");
    assert_eq!(
        sqlx::Row::get::<Option<Uuid>, _>(&approved_lineage, "parent_run_id"),
        Some(original_run)
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approved_lineage, "status"),
        "failed"
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approved_lineage, "approval_status"),
        "approved"
    );
    assert_eq!(
        sqlx::Row::get::<Option<Uuid>, _>(&approved_lineage, "resumed_run_id"),
        Some(approved_child)
    );
    assert_eq!(
        sqlx::Row::get::<String, _>(&approved_lineage, "original_status"),
        "succeeded"
    );
    let approved_child_input = sqlx::Row::get::<serde_json::Value, _>(&approved_lineage, "input");
    assert_eq!(
        approved_child_input["resume_approval_id"],
        approval_id.to_string()
    );
    assert_eq!(approved_child_input["resume_status"], "resolved");

    let replay_child = Uuid::new_v4();
    let replay_approval = client
        .post(&gateway)
        .header("Origin", "https://os.test")
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("x-csrf-token", csrf_token)
        .header("x-shennong-project-id", project_a.to_string())
        .json(&json!({
            "threadId":existing_thread,
            "runId":replay_child,
            "messages":[{"role":"assistant","content":"approval pending"}],
            "resume":[{
                "interruptId":approval_id,
                "status":"resolved",
                "payload":{"approved":true}
            }]
        }))
        .send()
        .await
        .expect("approval replay response");
    assert_eq!(replay_approval.status(), StatusCode::CONFLICT);
    let replay_child_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE id=$1")
        .bind(replay_child)
        .fetch_one(&pool)
        .await
        .expect("count replay child run");
    assert_eq!(replay_child_count, 0);

    // The test server intentionally has no Agent Runtime upstream. Reset the
    // already-authorized child to the state in which production Agent Runtime
    // bootstraps it, then exercise the immutable one-use execution grant.
    sqlx::query(
        "UPDATE runs SET status='queued',error='{}'::jsonb,finished_at=NULL,updated_at=NOW() \
         WHERE id=$1",
    )
    .bind(approved_child)
    .execute(&pool)
    .await
    .expect("restore approved child for internal bootstrap");
    let bootstrap = client
        .post(format!("http://{address}/api/v1/agent/runs"))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&json!({
            "thread_id":existing_thread,
            "run_id":approved_child,
            "parent_run_id":original_run
        }))
        .send()
        .await
        .expect("bootstrap approved continuation")
        .error_for_status()
        .expect("approved continuation bootstrap status")
        .json::<serde_json::Value>()
        .await
        .expect("approved continuation bootstrap body");
    let bootstrap_data = &bootstrap["data"];
    assert_eq!(bootstrap_data["parentRunId"], original_run.to_string());
    assert_eq!(
        bootstrap_data["resumeApproval"]["interruptId"],
        approval_id.to_string()
    );
    assert_eq!(
        bootstrap_data["resumeApproval"]["toolCallId"],
        "tool-write-approved"
    );
    assert_eq!(
        bootstrap_data["resumeApproval"]["argumentsDigest"],
        approved_digest
    );
    assert_eq!(
        bootstrap_data["resumeApproval"]["arguments"],
        approved_arguments
    );
    let run_capability = bootstrap_data["runCapabilityToken"]
        .as_str()
        .expect("resumed run capability");
    let execution_token = bootstrap_data["resumeApproval"]["executionToken"]
        .as_str()
        .expect("approval execution token");
    let execution_body = json!({
        "runId":approved_child,
        "userId":user_id,
        "projectId":project_a,
        "toolCallId":"tool-write-approved",
        "toolName":"project.write_file",
        "argumentsDigest":approved_digest,
        "risk":"write",
        "runCapabilityToken":run_capability,
        "arguments":approved_arguments,
        "executionToken":execution_token
    });
    let tampered_execution = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{approved_child}/tools"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&json!({
            "runId":approved_child,
            "userId":user_id,
            "projectId":project_a,
            "toolCallId":"tool-write-approved",
            "toolName":"project.write_file",
            "argumentsDigest":approved_digest,
            "risk":"write",
            "runCapabilityToken":run_capability,
            "arguments":{
                "uri":"project://current/README.md",
                "content":"tampered"
            },
            "executionToken":execution_token
        }))
        .send()
        .await
        .expect("tampered execution response");
    assert_eq!(tampered_execution.status(), StatusCode::FORBIDDEN);
    let unused_before_exact: bool = sqlx::query_scalar(
        "SELECT used_at IS NULL FROM run_tool_grants WHERE run_id=$1 AND tool_call_id=$2",
    )
    .bind(approved_child)
    .bind("tool-write-approved")
    .fetch_one(&pool)
    .await
    .expect("read unused grant after tamper");
    assert!(unused_before_exact);

    let exact_execution = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{approved_child}/tools"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&execution_body)
        .send()
        .await
        .expect("exact approved execution response");
    assert_eq!(exact_execution.status(), StatusCode::OK);
    let stored_file: (String, i32) =
        sqlx::query_as("SELECT content,version FROM project_files WHERE project_id=$1 AND path=$2")
            .bind(project_a)
            .bind("project://current/README.md")
            .fetch_one(&pool)
            .await
            .expect("read exactly approved project file");
    assert_eq!(stored_file.0, "# Approved durable continuation\n");
    assert_eq!(stored_file.1, 1);
    let used_once: bool = sqlx::query_scalar(
        "SELECT used_at IS NOT NULL FROM run_tool_grants WHERE run_id=$1 AND tool_call_id=$2",
    )
    .bind(approved_child)
    .bind("tool-write-approved")
    .fetch_one(&pool)
    .await
    .expect("read consumed approval grant");
    assert!(used_once);
    let execution_replay = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{approved_child}/tools"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&execution_body)
        .send()
        .await
        .expect("execution token replay response");
    assert_eq!(execution_replay.status(), StatusCode::FORBIDDEN);
    let stored_file_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM project_files WHERE project_id=$1 AND path=$2 AND version=1",
    )
    .bind(project_a)
    .bind("project://current/README.md")
    .fetch_one(&pool)
    .await
    .expect("count exactly-once project file");
    assert_eq!(stored_file_count, 1);

    let rejected_capability = "rejected-run-capability-with-enough-entropy";
    let rejected_original = insert_running_approval_run(
        &pool,
        project_a,
        existing_thread,
        user_id,
        rejected_capability,
    )
    .await;
    let rejected_request = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{rejected_original}/approvals/verify"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&json!({
            "runId":rejected_original,
            "userId":user_id,
            "projectId":project_a,
            "toolCallId":"tool-write-rejected",
            "toolName":"project.write_file",
            "argumentsDigest":approved_digest,
            "risk":"write",
            "runCapabilityToken":rejected_capability,
            "arguments":approved_arguments
        }))
        .send()
        .await
        .expect("request rejected approval")
        .error_for_status()
        .expect("rejected approval request status")
        .json::<serde_json::Value>()
        .await
        .expect("rejected approval request body");
    let rejected_approval = rejected_request["data"]["approvalId"]
        .as_str()
        .expect("rejected approval id")
        .parse::<Uuid>()
        .expect("rejected approval UUID");
    let rejected_child = Uuid::new_v4();
    let rejected_resume = client
        .post(&gateway)
        .header("Origin", "https://os.test")
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("x-csrf-token", csrf_token)
        .header("x-shennong-project-id", project_a.to_string())
        .json(&json!({
            "threadId":existing_thread,
            "runId":rejected_child,
            "messages":[{"role":"assistant","content":"approval pending"}],
            "resume":[{"interruptId":rejected_approval,"status":"cancelled"}]
        }))
        .send()
        .await
        .expect("rejected continuation response");
    assert_eq!(rejected_resume.status(), StatusCode::SERVICE_UNAVAILABLE);
    let rejected_state: (String, String, Uuid) = sqlx::query_as(
        "SELECT a.status,r.status,a.resumed_run_id FROM run_approvals a \
         JOIN runs r ON r.id=a.run_id WHERE a.id=$1",
    )
    .bind(rejected_approval)
    .fetch_one(&pool)
    .await
    .expect("read rejected continuation state");
    assert_eq!(rejected_state.0, "rejected");
    assert_eq!(rejected_state.1, "cancelled");
    assert_eq!(rejected_state.2, rejected_child);

    let expired_capability = "expired-run-capability-with-enough-entropy";
    let expired_original = insert_running_approval_run(
        &pool,
        project_a,
        existing_thread,
        user_id,
        expired_capability,
    )
    .await;
    let expired_request = client
        .post(format!(
            "http://{address}/api/v1/agent/runs/{expired_original}/approvals/verify"
        ))
        .header("authorization", format!("Bearer {service_token}"))
        .header("x-shennong-service", "agent-runtime")
        .json(&json!({
            "runId":expired_original,
            "userId":user_id,
            "projectId":project_a,
            "toolCallId":"tool-write-expired",
            "toolName":"project.write_file",
            "argumentsDigest":approved_digest,
            "risk":"write",
            "runCapabilityToken":expired_capability,
            "arguments":approved_arguments
        }))
        .send()
        .await
        .expect("request expiring approval")
        .error_for_status()
        .expect("expiring approval request status")
        .json::<serde_json::Value>()
        .await
        .expect("expiring approval request body");
    let expired_approval = expired_request["data"]["approvalId"]
        .as_str()
        .expect("expired approval id")
        .parse::<Uuid>()
        .expect("expired approval UUID");
    sqlx::query("UPDATE run_approvals SET expires_at=NOW()-INTERVAL '1 second' WHERE id=$1")
        .bind(expired_approval)
        .execute(&pool)
        .await
        .expect("expire pending approval");
    let expired_child = Uuid::new_v4();
    let expired_resume = client
        .post(&gateway)
        .header("Origin", "https://os.test")
        .header("Cookie", format!("shennong_os_session={session_token}"))
        .header("x-csrf-token", csrf_token)
        .header("x-shennong-project-id", project_a.to_string())
        .json(&json!({
            "threadId":existing_thread,
            "runId":expired_child,
            "messages":[{"role":"assistant","content":"approval pending"}],
            "resume":[{
                "interruptId":expired_approval,
                "status":"resolved",
                "payload":{"approved":true}
            }]
        }))
        .send()
        .await
        .expect("expired continuation response");
    assert_eq!(expired_resume.status(), StatusCode::CONFLICT);
    let expired_state: (String, String) = sqlx::query_as(
        "SELECT a.status,r.status FROM run_approvals a JOIN runs r ON r.id=a.run_id \
         WHERE a.id=$1",
    )
    .bind(expired_approval)
    .fetch_one(&pool)
    .await
    .expect("read expired continuation state");
    assert_eq!(expired_state.0, "expired");
    assert_eq!(expired_state.1, "cancelled");
    let expired_child_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE id=$1")
        .bind(expired_child)
        .fetch_one(&pool)
        .await
        .expect("count expired child run");
    assert_eq!(expired_child_count, 0);
    server.abort();
}
