use super::*;
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "schemaVersion": 1,
        "releases": {
            "windows-x86_64": release("0.6.10"),
            "macos-aarch64": release("0.7.0"),
            "linux-x86_64": release("0.6.9")
        }
    }))
    .unwrap()
}

fn release(version: &str) -> serde_json::Value {
    json!({ "version": version, "publishedAt": "2026-10-11T08:00:00Z",
        "notes": { "zh-CN": ["更新"], "en": ["Update"] },
        "downloads": { "github": format!("https://github.com/ooolabdev/ooosplat/releases/tag/{version}"),
            "quark": "https://pan.quark.cn/s/abc123", "quarkCode": "1234" }
    })
}

fn cache() -> CachedManifest {
    CachedManifest {
        checked_at: Utc::now(),
        manifest: manifest(),
    }
}

#[test]
fn numeric_versions_stable_releases_platforms_and_build_metadata() {
    let mut cache = cache();
    for (os, arch, expected) in [
        ("windows", "x86_64", UpdateStatus::UpdateAvailable),
        ("macos", "aarch64", UpdateStatus::UpdateAvailable),
        ("linux", "x86_64", UpdateStatus::UpToDate),
    ] {
        assert_eq!(
            evaluate(&cache, &platform_key(os, arch), "0.6.9", false)
                .unwrap()
                .status,
            expected
        );
    }
    assert_eq!(
        evaluate(&cache, "linux-aarch64", "0.6.9", false)
            .unwrap()
            .status,
        UpdateStatus::PlatformUnavailable
    );
    for version in ["0.6.9", "0.6.8", "0.7.0-rc.1", "0.6.9+new-build"] {
        cache
            .manifest
            .releases
            .get_mut("windows-x86_64")
            .unwrap()
            .version = version.into();
        assert_eq!(
            evaluate(&cache, "windows-x86_64", "0.6.9", false)
                .unwrap()
                .status,
            UpdateStatus::UpToDate
        );
    }
    cache
        .manifest
        .releases
        .get_mut("windows-x86_64")
        .unwrap()
        .version = "v0.6.10".into();
    assert_eq!(
        evaluate(&cache, "windows-x86_64", "0.6.9", false)
            .unwrap()
            .status,
        UpdateStatus::UpdateAvailable
    );
    cache.manifest.schema_version = 2;
    assert!(evaluate(&cache, "windows-x86_64", "0.6.9", false).is_err());
}

#[test]
fn trusted_links_https_and_three_redirect_limit() {
    for url in [
        "https://ooolab.cn/updates/latest.json",
        "https://www.ooolab.cn/updates/latest.json",
    ] {
        assert!(allowed_manifest_redirect(&Url::parse(url).unwrap(), 3));
        assert!(!allowed_manifest_redirect(&Url::parse(url).unwrap(), 4));
    }
    for url in [
        "http://ooolab.cn/updates/latest.json",
        "https://ooolab.cn.evil.test/x",
        "https://name:password@ooolab.cn/x",
        "https://ooolab.cn:8443/x",
    ] {
        assert!(!allowed_manifest_url(&Url::parse(url).unwrap()));
    }
    for url in [
        "https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1",
        "https://github.com/ooolabdev/ooosplat/releases/download/0.6.1/OOOSplat.exe",
    ] {
        assert!(allowed_download_url(url, false));
    }
    for url in [
        "https://github.com/another/ooosplat/releases/tag/0.6.1",
        "https://github.com/ooolabdev/ooosplat/issues/1",
        "http://github.com/ooolabdev/ooosplat/releases/tag/0.6.1",
        "https://user:pass@github.com/ooolabdev/ooosplat/releases/tag/0.6.1",
    ] {
        assert!(!allowed_download_url(url, false));
    }
    assert!(allowed_download_url("https://pan.quark.cn/s/abc123", true));
    assert!(!allowed_download_url(
        "https://pan.quark.cn.evil.test/s/abc123",
        true
    ));
    assert!(!allowed_download_url(
        "https://pan.quark.cn/s/abc123/other",
        true
    ));
    assert!(!allowed_download_url("https://pan.quark.cn/s/", true));
}

#[test]
fn cache_age_is_bounded_and_future_timestamps_do_not_delay_checks() {
    let cache = cache();
    assert!(cache_is_fresh(
        &cache,
        cache.checked_at + chrono::Duration::hours(23)
    ));
    assert!(!cache_is_fresh(
        &cache,
        cache.checked_at + chrono::Duration::hours(24)
    ));
    assert!(!cache_is_fresh(
        &cache,
        cache.checked_at - chrono::Duration::seconds(1)
    ));
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: String,
    delay: Duration,
    chunked: bool,
}
impl Reply {
    fn json() -> Self {
        Self {
            status: StatusCode::OK,
            body: serde_json::to_string(&manifest()).unwrap(),
            delay: Duration::ZERO,
            chunked: false,
        }
    }
}
struct Server {
    endpoint: String,
    calls: Arc<AtomicUsize>,
    requests: Arc<std::sync::Mutex<Vec<(String, HeaderMap)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(replies: Vec<Reply>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/updates/latest.json",
        listener.local_addr().unwrap()
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let responses = Arc::new(std::sync::Mutex::new(VecDeque::from(replies)));
    let handler_calls = calls.clone();
    let handler_requests = requests.clone();
    let router = Router::new().route(
        "/updates/latest.json",
        get(move |uri: axum::http::Uri, headers: HeaderMap| {
            let calls = handler_calls.clone();
            let requests = handler_requests.clone();
            let responses = responses.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                requests.lock().unwrap().push((uri.to_string(), headers));
                let reply = responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(Reply::json);
                tokio::time::sleep(reply.delay).await;
                let mut response = Response::builder().status(reply.status);
                if reply.chunked {
                    response = response.header("Transfer-Encoding", "chunked");
                }
                response.body(Body::from(reply.body)).unwrap()
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        endpoint,
        calls,
        requests,
        task,
    }
}

fn service(server: &Server, path: Option<PathBuf>, timeout: Duration) -> UpdateService {
    UpdateService {
        // Localhost HTTP is injectable only in this test module, never via an IPC argument.
        client: Some(
            reqwest::Client::builder()
                .timeout(timeout)
                .redirect(Policy::none())
                .build()
                .unwrap(),
        ),
        endpoint: server.endpoint.clone(),
        platform: "windows-x86_64".into(),
        current_version: "0.6.9".into(),
        cache_path: path,
        state: Mutex::new(CheckState::default()),
    }
}

#[tokio::test]
async fn shared_requests_cooldown_and_persistent_cache_do_not_upload_identifiers() {
    let mock = server(vec![Reply {
        delay: Duration::from_millis(20),
        ..Reply::json()
    }])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("update-notice.json");
    let checker = service(&mock, Some(path.clone()), Duration::from_secs(1));
    let (first, second) = tokio::join!(checker.check(true), checker.check(true));
    let first = first.unwrap();
    assert_eq!(first.status, UpdateStatus::UpdateAvailable);
    let wire = serde_json::to_value(&first).unwrap();
    assert_eq!(wire["status"], "updateAvailable");
    assert_eq!(wire["currentVersion"], "0.6.9");
    assert_eq!(wire["release"]["notes"]["zh-CN"][0], "更新");
    assert_eq!(wire["release"]["downloads"]["quarkCode"], "1234");
    assert_eq!(wire["fromCache"], false);
    assert_eq!(second.unwrap().status, UpdateStatus::UpdateAvailable);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    assert!(checker.check(false).await.unwrap().from_cache);
    let restarted = service(&mock, Some(path), Duration::from_secs(1));
    assert!(restarted.check(false).await.unwrap().from_cache);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    // Manual checks bypass persistent cache, but not the in-process 30-second cooldown.
    restarted.check(true).await.unwrap();
    restarted.check(true).await.unwrap();
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    for (uri, headers) in mock.requests.lock().unwrap().iter() {
        assert_eq!(uri, "/updates/latest.json");
        assert!(!headers.contains_key("Authorization"));
        assert!(!headers.contains_key("Cookie"));
        assert_eq!(headers.get("Accept").unwrap(), "application/json");
    }
}

#[tokio::test]
async fn corrupt_expired_future_or_unwritable_cache_does_not_block_checks() {
    let mock = server(Vec::new()).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notice.json");
    for contents in [
        "broken".into(),
        serde_json::to_string(&CachedManifest {
            checked_at: Utc::now() - chrono::Duration::hours(25),
            ..cache()
        })
        .unwrap(),
        serde_json::to_string(&CachedManifest {
            checked_at: Utc::now() + chrono::Duration::hours(1),
            ..cache()
        })
        .unwrap(),
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(
            !service(&mock, Some(path.clone()), Duration::from_secs(1))
                .check(false)
                .await
                .unwrap()
                .from_cache
        );
    }
    // A cache path below an ordinary file cannot be created or written.
    let blocked = directory.path().join("file");
    std::fs::write(&blocked, "not a directory").unwrap();
    assert_eq!(
        service(
            &mock,
            Some(blocked.join("notice.json")),
            Duration::from_secs(1)
        )
        .check(false)
        .await
        .unwrap()
        .status,
        UpdateStatus::UpdateAvailable
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn invalid_http_json_and_oversize_responses_are_rejected_and_not_cached() {
    for reply in [
        Reply {
            status: StatusCode::NOT_FOUND,
            ..Reply::json()
        },
        Reply {
            body: "<html>not json</html>".into(),
            ..Reply::json()
        },
        Reply {
            body: " ".repeat(MAX_MANIFEST_BYTES + 1),
            ..Reply::json()
        },
        Reply {
            body: " ".repeat(MAX_MANIFEST_BYTES + 1),
            chunked: true,
            ..Reply::json()
        },
    ] {
        let mock = server(vec![reply]).await;
        let checker = service(&mock, None, Duration::from_secs(1));
        assert!(checker.check(false).await.is_err());
        assert!(checker.state.lock().await.cache.is_none());
        assert!(checker.check(true).await.is_err());
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn timeout_and_failed_refresh_preserve_the_last_good_release() {
    let mock = server(vec![
        Reply::json(),
        Reply {
            delay: Duration::from_millis(150),
            ..Reply::json()
        },
    ])
    .await;
    let checker = service(&mock, None, Duration::from_millis(70));
    checker.check(false).await.unwrap();
    checker.state.lock().await.last_attempt = Some(Instant::now() - CHECK_COOLDOWN);
    assert!(checker.check(true).await.is_err());
    assert_eq!(
        checker
            .state
            .lock()
            .await
            .cache
            .as_ref()
            .unwrap()
            .manifest
            .releases["windows-x86_64"]
            .version,
        "0.6.10"
    );
    assert!(checker.check(false).await.unwrap().from_cache);
}
