use super::*;
use rmcp::{
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};

#[test]
fn generation_policy_schema_accepts_saved_versions_and_defaults_to_v3() {
    let definition = definitions()
        .into_iter()
        .find(|t| t.name == "create_generation_task")
        .unwrap();
    let schema = serde_json::to_value(definition.input_schema).unwrap();
    let version = schema
        .pointer("/properties/generation_policy/properties/version")
        .unwrap();
    assert_eq!(version["minimum"], 1);
    assert_eq!(version["maximum"], 3);
    assert_eq!(version["default"], 3);
    for version in [1, 2, 3] {
        let request: CreateRequest = serde_json::from_value(json!({"input_path":"clip.mp4","quality":"fast",
            "client_request_id":"version-test","generation_policy":{"version":version,"frameSamplingRatio":0.9}})).unwrap();
        assert_eq!(
            request
                .generation_policy
                .unwrap()
                .validate()
                .unwrap()
                .version,
            version
        );
    }
}

#[test]
fn reshoot_requests_are_explicit_and_old_generation_requests_remain_unchanged() {
    let old: CreateRequest = serde_json::from_value(
        json!({"input_path":"clip.mp4","quality":"fast","client_request_id":"old"}),
    )
    .unwrap();
    assert!(old.source_project_id.is_none() && old.reshoot_options.is_none());
    let new: CreateRequest = serde_json::from_value(json!({"input_path":"new-images","quality":"high","client_request_id":"new","source_project_id":Uuid::nil(),"reshoot_options":{"condition":"good","allowFallback":false,"sourceIdentity":"source","inputIdentity":"input","projection":"perspective"}})).unwrap();
    assert!(!new.reshoot_options.unwrap().allow_fallback);
    assert!(serde_json::from_value::<CreateRequest>(json!({"input_path":"new-images","quality":"high","client_request_id":"invalid","reshoot_options":{"condition":"good"}})).is_err());
    let definition = definitions()
        .into_iter()
        .find(|t| t.name == "inspect_reshoot_input")
        .unwrap();
    assert!(definition.output_schema.is_some());
    assert!(definition
        .input_schema
        .get("required")
        .unwrap()
        .as_array()
        .unwrap()
        .contains(&json!("source_project_id")));
}

#[tokio::test]
async fn official_client_token_free_http_smoke_and_local_request_rejections() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("ordinary-images");
    std::fs::create_dir(&input).unwrap();
    // Real, unambiguous image headers; only execution is controlled, not admission.
    for index in 0..31 {
        image::RgbImage::new(3, 3)
            .save(input.join(format!("{index:03}.png")))
            .unwrap();
    }
    let tasks = TaskService::at(temp.path().join("registry"));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    let release = Arc::new(tokio::sync::Notify::new());
    let worker_release = release.clone();
    *tasks.executor.lock().unwrap() = Some(Arc::new(move |_, _| {
        let release = worker_release.clone();
        Box::pin(async move {
            release.notified().await;
            Err(crate::error::SplatError::Process(
                "controlled protocol fixture failure; no GPU executed".into(),
            ))
        })
    }));
    let stop = CancellationToken::new();
    let tools = McpTools {
        app: None,
        tasks: tasks.clone(),
        engines: crate::pipeline::runner::default_engine_paths(Some(
            temp.path().join("missing-engines"),
        )),
        settings: McpSettings {
            enabled: true,
            port,
            input_roots: vec![std::fs::canonicalize(temp.path()).unwrap()],
        },
        telemetry: None,
    };
    let app = router(tools, LocalRequestGuard { port }, stop.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();
    assert_eq!(
        http.post(&url)
            .header("Origin", "https://untrusted.example")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        http.post(&url)
            .header("Host", "untrusted.example")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(url.clone()),
    );
    let client = ().serve(transport).await.unwrap();
    let definitions = client.list_tools(None).await.unwrap();
    assert_eq!(definitions.tools.len(), 14);
    assert!(definitions.tools.iter().all(|t| t.output_schema.is_some()));
    let app_status = client
        .call_tool(CallToolRequestParams::new("get_app_status"))
        .await
        .unwrap();
    assert_eq!(app_status.is_error, Some(false));
    assert_eq!(
        app_status.structured_content.as_ref().unwrap()["authorized_input_roots"][0],
        serde_json::to_value(std::fs::canonicalize(temp.path()).unwrap()).unwrap()
    );
    let arguments =
        json!({"input_path":input,"quality":"fast","client_request_id":"protocol-request-1"});
    let inspected = client
        .call_tool(
            CallToolRequestParams::new("inspect_generation_input").with_arguments(
                json!({"input_path":input,"quality":"fast"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(inspected.is_error, Some(false), "{inspected:#?}");
    let inspected = inspected.structured_content.unwrap();
    assert!(inspected["questions"].as_array().unwrap().is_empty());
    assert_eq!(inspected["plan_is_estimate"], true);
    assert!(inspected["max_splats"].is_null());
    assert!(!inspected.to_string().contains("preview_jpeg"));
    let created = client
        .call_tool(
            CallToolRequestParams::new("create_generation_task")
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(created.is_error, Some(false), "{created:#?}");
    let task_id = created.structured_content.as_ref().unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let duplicate = client
        .call_tool(
            CallToolRequestParams::new("create_generation_task")
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        duplicate.structured_content.as_ref().unwrap()["task_id"],
        task_id
    );
    let args = json!({"task_id":task_id});
    let started = client
        .call_tool(
            CallToolRequestParams::new("start_task")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(started.is_error, Some(false));
    let run_id = started.structured_content.as_ref().unwrap()["run_id"].clone();
    assert_eq!(
        started.structured_content.as_ref().unwrap()["accepted"],
        true
    );
    client.cancel().await.unwrap(); // Explicitly disconnect while execution is still blocked.
    assert!(tasks
        .get(Uuid::parse_str(&task_id).unwrap())
        .await
        .unwrap()
        .status
        .active());
    release.notify_one();
    let terminal = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tasks.wait(Uuid::parse_str(&task_id).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(terminal.status, crate::tasks::TaskStatus::Failed);
    assert!(terminal.error.is_some());
    let reconnect = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(url.clone()),
    );
    let client = ().serve(reconnect).await.unwrap();
    for name in ["get_task_status", "read_task_logs"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false), "{name}");
    }
    assert_eq!(
        client
            .call_tool(CallToolRequestParams::new("list_tasks"))
            .await
            .unwrap()
            .is_error,
        Some(false)
    );
    let cancel = json!({"task_id":task_id,"run_id":run_id});
    assert_eq!(
        client
            .call_tool(
                CallToolRequestParams::new("cancel_task")
                    .with_arguments(cancel.as_object().unwrap().clone())
            )
            .await
            .unwrap()
            .is_error,
        Some(false)
    );
    let unauthorized = json!({"input_path":temp.path().parent().unwrap(),"quality":"high","client_request_id":"outside"});
    let rejection = client
        .call_tool(
            CallToolRequestParams::new("create_generation_task")
                .with_arguments(unauthorized.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(rejection.is_error, Some(true));
    assert_eq!(
        rejection.structured_content.unwrap()["error"]["code"],
        "PATH_NOT_AUTHORIZED"
    );
    let invalid_cursor = client
        .call_tool(
            CallToolRequestParams::new("read_task_logs").with_arguments(
                json!({"task_id":task_id,"run_id":run_id,"cursor":"invalid-cursor"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(invalid_cursor.is_error, Some(true));
    assert_eq!(
        invalid_cursor.structured_content.unwrap()["error"]["code"],
        "INVALID_CURSOR"
    );
    // A token left in an older client configuration is ignored, not required.
    let legacy_transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(url).auth_header("obsolete-client-token"),
    );
    let legacy = ().serve(legacy_transport).await.unwrap();
    assert_eq!(
        legacy
            .call_tool(CallToolRequestParams::new("get_app_status"))
            .await
            .unwrap()
            .is_error,
        Some(false)
    );
    legacy.cancel().await.unwrap();
    // Turning off the HTTP entry point also leaves a second accepted execution alive.
    let created=client.call_tool(CallToolRequestParams::new("create_generation_task").with_arguments(json!({"input_path":input,"quality":"fast","client_request_id":"protocol-request-2"}).as_object().unwrap().clone())).await.unwrap();
    let second = created.structured_content.unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .call_tool(
            CallToolRequestParams::new("start_task")
                .with_arguments(json!({"task_id":second}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    client.cancel().await.unwrap();
    stop.cancel();
    assert!(tasks
        .get(Uuid::parse_str(&second).unwrap())
        .await
        .unwrap()
        .status
        .active());
    release.notify_one();
    tasks.wait(Uuid::parse_str(&second).unwrap()).await.unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn default_input_directory_is_created_authorized_and_kept_separate_from_saved_scopes() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("Projects");
    let custom = temp.path().join("UserMedia");
    std::fs::create_dir(&custom).unwrap();
    let settings = McpSettings {
        input_roots: vec![std::fs::canonicalize(&custom).unwrap()],
        ..Default::default()
    };
    let (default, effective) = with_default_input_root(&settings, &projects).await.unwrap();
    assert_eq!(
        default,
        std::fs::canonicalize(projects.join("Inputs")).unwrap()
    );
    assert!(default.is_dir());
    assert_eq!(settings.input_roots.len(), 1);
    assert_eq!(effective.input_roots.len(), 2);
    let video = default.join("clip.mov");
    std::fs::write(&video, b"fixture").unwrap();
    assert!(authorize_input(&video, &effective.input_roots).is_ok());
    let outside = projects.join("private.mov");
    std::fs::write(&outside, b"fixture").unwrap();
    assert_eq!(
        authorize_input(&outside, &effective.input_roots)
            .unwrap_err()
            .code,
        "PATH_NOT_AUTHORIZED"
    );
    let (_, again) = with_default_input_root(&settings, &projects).await.unwrap();
    assert_eq!(effective.input_roots, again.input_roots);
    let (moved, updated) = with_default_input_root(&settings, &temp.path().join("OtherProjects"))
        .await
        .unwrap();
    assert!(updated.input_roots.contains(&moved));
    assert!(!updated.input_roots.contains(&default));
}

#[tokio::test]
async fn default_input_directory_rejects_relative_roots() {
    assert_eq!(
        with_default_input_root(
            &McpSettings::default(),
            std::path::Path::new("relative-projects")
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_PATH"
    );
}

#[tokio::test]
async fn default_input_directory_rejects_redirected_folders() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("Projects");
    let outside = temp.path().join("Outside");
    std::fs::create_dir(&projects).unwrap();
    std::fs::create_dir(&outside).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&outside, projects.join("Inputs"))
        .expect("This test needs permission for its temporary symlink fixture");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, projects.join("Inputs")).unwrap();
    assert_eq!(
        with_default_input_root(&McpSettings::default(), &projects)
            .await
            .unwrap_err()
            .code,
        "INVALID_INPUT_ROOT"
    );
}

#[test]
fn local_request_guard_keeps_host_and_origin_checks_without_credentials() {
    let guard = LocalRequestGuard { port: 39877 };
    for host in ["127.0.0.1:39877", "localhost:39877"] {
        for origin in [
            None,
            Some("http://127.0.0.1:39877"),
            Some("http://localhost:39877"),
        ] {
            let mut request = Request::builder()
                .header("Host", host)
                .header("Authorization", "Bearer old-unused-token");
            if let Some(origin) = origin {
                request = request.header("Origin", origin);
            }
            assert!(local_request_allowed(
                &request.body(axum::body::Body::empty()).unwrap(),
                &guard
            ));
        }
    }
    for (host, origin) in [
        ("untrusted.example", "http://127.0.0.1:39877"),
        ("127.0.0.1:39878", "http://127.0.0.1:39877"),
        ("127.0.0.1:39877", "https://untrusted.example"),
        ("127.0.0.1:39877", "null"),
    ] {
        let request = Request::builder()
            .header("Host", host)
            .header("Origin", origin)
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(!local_request_allowed(&request, &guard));
    }
    assert!(!local_request_allowed(
        &Request::new(axum::body::Body::empty()),
        &guard
    ));
}

#[test]
fn schemas_and_annotations_match_the_fourteen_tool_contracts() {
    for tool in definitions() {
        let annotation = tool.annotations.unwrap();
        assert_eq!(annotation.open_world_hint, Some(false));
        assert_eq!(
            annotation.idempotent_hint,
            Some(tool.name != "confirm_model_export")
        );
        assert_eq!(
            annotation.destructive_hint,
            Some(matches!(
                tool.name.as_ref(),
                "cancel_task" | "cancel_model_export"
            ))
        );
        assert_eq!(tool.input_schema["additionalProperties"], false);
        let schema = tool.output_schema.unwrap();
        assert_eq!(
            schema["anyOf"][1]["properties"]["error"]["required"],
            json!(["code", "message"])
        );
    }
}

#[tokio::test]
async fn task_status_bounds_execution_evidence_and_excludes_local_gps_and_tokens() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("video.mp4");
    std::fs::write(&input, b"fixture").unwrap();
    let service = TaskService::at(root.path().join("registry"));
    let mut task = service
        .create(
            None,
            input,
            Quality::High,
            root.path().join("projects"),
            true,
            "mcp",
            None,
            None,
        )
        .await
        .unwrap();
    task.actual_configuration = Some(
        json!({"reshoot_recipe":{"version":2,"init_georeference":"PRIVATE_GEO","init_ply_path":"PRIVATE_INIT","source_names":["PRIVATE_SOURCE"],"actual":"retrain","options":{"condition":"appearance","camera_relation":"unsure","allow_fallback":true,"source_identity":"PRIVATE_ID"}}}),
    );
    let mut runtime = crate::pipeline::runtime::RuntimeTracker::new().snapshot;
    runtime.device = Some("Fixture GPU".into());
    runtime.process_id = 12345;
    runtime
        .config
        .insert("private".into(), "PRIVATE_DEVICE_ID".into());
    let execution = crate::pipeline::brush_execution::BrushExecutionSnapshot::new(&runtime, None);
    task.runs.push(crate::tasks::TaskRun {
        run_id: Uuid::new_v4(),
        kind: "generation".into(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        logs: Default::default(),
        last_engine: None,
        last_exit_code: None,
        error: None,
        brush_executions: vec![execution; 10],
        current_brush_execution: Some(9),
    });
    let public = public_task(task).unwrap();
    let text = public.to_string();
    for private in [
        "PRIVATE_GEO",
        "PRIVATE_INIT",
        "PRIVATE_SOURCE",
        "PRIVATE_ID",
        "PRIVATE_DEVICE_ID",
        "request_parameters",
        "process_id",
    ] {
        assert!(!text.contains(private), "{private}");
    }
    assert_eq!(
        public["runs"][0]["brush_executions"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(public["runs"][0]["brush_executions_truncated"], true);
    assert_eq!(
        public["runs"][0]["brush_executions"][7]["device"]["name"],
        "Fixture GPU"
    );
    assert!(public["runs"][0]["brush_executions"][7]["device"]["total_memory_mb"].is_null());
}
