use super::*;
fn fixture(count: u64) -> Inspection {
    Inspection {
        id: Uuid::new_v4(),
        path: PathBuf::from("images"),
        identity: "fixture".into(),
        quality: Quality::High,
        policy: GenerationPolicy::default(),
        source: None,
        analysis: InputAnalysis {
            input_type: crate::project::ProjectInputType::Images,
            video: None,
            image_sequence: Some(crate::video::ImageSequenceInfo {
                image_count: count,
                width: 400,
                height: 200,
                has_alpha: false,
                requires_large_sequence_confirmation: count > 500,
            }),
            projection_detection: crate::video::panorama::ProjectionDetection {
                source_identity: "fixture".into(),
                unsupported: false,
                confirmed_erp: false,
                requires_choice: false,
                supported_erp: true,
                reason: String::new(),
                preview_jpeg: vec![],
            },
        },
    }
}
fn tools(root: &Path) -> McpTools {
    McpTools {
        app: None,
        tasks: TaskService::at(root.join("registry")),
        engines: crate::pipeline::runner::default_engine_paths(Some(root.join("engines"))),
        settings: McpSettings {
            enabled: true,
            port: 39877,
            input_roots: vec![std::fs::canonicalize(root).unwrap()],
        },
        telemetry: None,
    }
}
#[tokio::test]
async fn admission_is_answer_gated_cached_identity_bound_and_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let directory = root.path().join("images");
    std::fs::create_dir(&directory).unwrap();
    for i in 0..101 {
        image::RgbImage::new(4, 2)
            .save(directory.join(format!("{i:03}.png")))
            .unwrap();
    }
    let request = json!({"input_path":directory,"quality":"fast","client_request_id":"admission"});
    let error = create(&tools, request.clone()).await.unwrap_err();
    assert_eq!(error.code, "CONFIRMATION_REQUIRED");
    assert!(tools.tasks.all().await.unwrap().is_empty());
    let inspect_request = || InspectRequest {
        input_path: directory.to_string_lossy().into(),
        quality: Quality::Fast,
        generation_policy: None,
        source_project_id: None,
    };
    let first = inspect(&tools, inspect_request()).await.unwrap();
    let canonical = std::fs::canonicalize(&directory).unwrap();
    let reads = crate::video::image_projection::header_reads_for_test(&canonical);
    assert!(reads >= 101);
    let mut replan = inspect_request();
    replan.quality = Quality::High;
    replan.generation_policy = Some(GenerationPolicy {
        frame_sampling_ratio: Some(0.5),
        ..Default::default()
    });
    let changed_config = inspect(&tools, replan).await.unwrap();
    assert_ne!(changed_config.id, first.id);
    assert_eq!(
        crate::video::image_projection::header_reads_for_test(&canonical),
        reads
    );
    assert_eq!(
        verify_id(Some(changed_config.id), &first)
            .await
            .unwrap_err()
            .code,
        "INPUT_CHANGED"
    );
    let mut answered = request.clone();
    answered["preflight_id"] = json!(first.id);
    answered["input_source_identity"] = json!(first.identity);
    answered["answers"] = json!({"input_projection":"perspective","image_order":"sequential"});
    let created = create(&tools, answered.clone()).await.unwrap();
    let id = Uuid::parse_str(created["task_id"].as_str().unwrap()).unwrap();
    let saved = tools.tasks.get(id).await.unwrap();
    assert_eq!(
        saved.request_parameters.as_ref().unwrap()["imageMatchingStrategy"],
        "sequential"
    );
    // Losing a create response must not duplicate a task, even after source changes.
    image::RgbImage::new(4, 2)
        .save(directory.join("new.png"))
        .unwrap();
    assert_eq!(
        create(&tools, answered.clone()).await.unwrap()["task_id"],
        created["task_id"]
    );
    answered["quality"] = json!("high");
    assert_eq!(
        create(&tools, answered).await.unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        tools
            .tasks
            .start(id, tools.engines.clone(), None)
            .await
            .unwrap_err()
            .code,
        "INPUT_CHANGED"
    );
    assert_eq!(
        tools.tasks.get(id).await.unwrap().status,
        crate::tasks::TaskStatus::Created
    );
    let new = inspect(&tools, inspect_request()).await.unwrap();
    assert_ne!(new.id, first.id);
    assert_eq!(
        verify_id(Some(first.id), &new).await.unwrap_err().code,
        "INPUT_CHANGED"
    );
    assert_eq!(tools.tasks.all().await.unwrap().len(), 1);
}
#[test]
fn questions_use_existing_bilingual_copy_and_have_no_default_answer() {
    for count in [2, 29, 30, 100, 101, 500, 501] {
        let mut input = fixture(count);
        input.analysis.projection_detection.requires_choice = true;
        for question in input.questions() {
            assert!(question.get("default").is_none());
            for locale in ["zh-CN", "en"] {
                assert!(
                    !question["title"][locale].as_str().unwrap().is_empty(),
                    "{question}"
                );
                for option in question["options"].as_array().unwrap() {
                    assert!(
                        !option["label"][locale].as_str().unwrap().is_empty(),
                        "{question}"
                    );
                }
            }
        }
    }
    for key in [
        "export.spzAdjustTitle",
        "export.spzAdjustReason",
        "export.spzAdjustAction",
        "export.spzAdjustImpact",
        "export.spzAdjustAdvice",
        "export.spzAdjustConfirm",
        "common.cancel",
    ] {
        for locale in ["zh-CN", "en"] {
            assert!(
                !text(key, 0)[locale].as_str().unwrap().is_empty(),
                "{key}/{locale}"
            );
        }
    }
}
#[test]
fn image_thresholds_and_explicit_projection_cannot_be_bypassed() {
    for count in [29, 30, 100, 101, 500, 501] {
        let input = fixture(count);
        let keys: Vec<_> = input
            .questions()
            .iter()
            .map(|q| q["key"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(keys.contains(&"small_images".into()), count < 30);
        assert_eq!(keys.contains(&"image_order".into()), count > 100);
        let answers = Answers {
            small_images: Some(true),
            image_order: Some(ImageMatchingStrategy::Sequential),
            ..Default::default()
        };
        let (_, strategy, _) = input.validate_answers(&answers).unwrap();
        assert_eq!(
            strategy,
            if count > 100 {
                ImageMatchingStrategy::Sequential
            } else {
                ImageMatchingStrategy::Exhaustive
            }
        );
    }
    let mut input = fixture(101);
    input.analysis.projection_detection.requires_choice = true;
    let error = input.validate_answers(&Answers::default()).unwrap_err();
    assert_eq!(error.code, "CONFIRMATION_REQUIRED");
    assert_eq!(
        error.details.unwrap()["questions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    input.analysis.projection_detection.supported_erp = false;
    assert_eq!(input.questions()[0]["options"].as_array().unwrap().len(), 1);
    let answers = Answers {
        input_projection: Some(VideoProjection::Equirectangular),
        image_order: Some(ImageMatchingStrategy::Exhaustive),
        ..Default::default()
    };
    assert_eq!(
        input.validate_answers(&answers).unwrap_err().code,
        "INVALID_PROJECTION"
    );
    input.analysis.projection_detection.requires_choice = false;
    input.analysis.projection_detection.supported_erp = true;
    input.analysis.projection_detection.confirmed_erp = true;
    let answers = Answers {
        input_projection: Some(VideoProjection::Perspective),
        ..answers
    };
    assert_eq!(
        input.validate_answers(&answers).unwrap_err().code,
        "INVALID_PROJECTION"
    );
}
#[test]
fn reshoot_question_order_and_retry_authorization_share_existing_options() {
    let mut input = fixture(101);
    input.policy.auto_retry_enabled = false;
    input.source = Some(ReshootSourceInfo {
        source_identity: "source".into(),
        projection: VideoProjection::Perspective,
        warm_available: true,
        project_id: Uuid::nil().to_string(),
        project_name: "source".into(),
        quality: Quality::High,
        planner_enabled: true,
        camera_id: 1,
        camera_model: "SIMPLE_RADIAL".into(),
        width: 400,
        height: 200,
        source_image_count: 20,
        eligible: true,
        reason: None,
        imported: false,
    });
    let questions = input.questions();
    assert_eq!(
        questions
            .iter()
            .map(|q| q["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["model_condition", "camera_relation", "image_order"]
    );
    for option in questions[0]["options"].as_array().unwrap() {
        for locale in ["zh-CN", "en"] {
            assert!(!option["description"][locale].as_str().unwrap().is_empty());
        }
    }
    let answers = Answers {
        model_condition: Some(ModelCondition::Good),
        camera_relation: Some(CameraRelation::Same),
        image_order: Some(ImageMatchingStrategy::Sequential),
        ..Default::default()
    };
    assert!(
        !input
            .validate_answers(&answers)
            .unwrap()
            .2
            .unwrap()
            .allow_fallback
    );
    input.source.as_mut().unwrap().projection = VideoProjection::Equirectangular;
    assert!(!input
        .questions()
        .iter()
        .any(|q| q["key"] == "camera_relation"));
}
