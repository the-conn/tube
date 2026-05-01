use std::{io::Write, os::unix::fs::PermissionsExt};

use flate2::{Compression, write::GzEncoder};
use serde_json::Value;
use serial_test::serial;
use tempfile::{NamedTempFile, TempDir};
use tube::{TubeError, tube_config::TubeConfig};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn make_script(content: &str) -> tempfile::TempPath {
  let mut f = NamedTempFile::new().unwrap();
  writeln!(f, "#!/bin/sh").unwrap();
  writeln!(f, "{}", content).unwrap();
  std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
  f.into_temp_path()
}

struct EnvVarGuard {
  key: String,
  original: Option<String>,
}

impl EnvVarGuard {
  fn set(key: &str, val: &str) -> Self {
    let original = std::env::var(key).ok();
    unsafe { std::env::set_var(key, val) };
    EnvVarGuard {
      key: key.to_string(),
      original,
    }
  }
}

impl Drop for EnvVarGuard {
  fn drop(&mut self) {
    match &self.original {
      Some(v) => unsafe { std::env::set_var(&self.key, v) },
      None => unsafe { std::env::remove_var(&self.key) },
    }
  }
}

fn load_config(
  server: &MockServer,
  get_url: &str,
  workspace_dir: &str,
  script_path: &str,
  logs_put_url: &str,
) -> (TubeConfig, Vec<EnvVarGuard>) {
  let guards = vec![
    EnvVarGuard::set(
      "TUBE__EXECUTION__STATUS_PUT_URL",
      &format!("{}/status", server.uri()),
    ),
    EnvVarGuard::set(
      "TUBE__EXECUTION__POKE_URL",
      &format!("{}/poke", server.uri()),
    ),
    EnvVarGuard::set("TUBE__EXECUTION__LOGS_PUT_URL", logs_put_url),
    EnvVarGuard::set("TUBE__WORKSPACE__GET_URL", get_url),
    EnvVarGuard::set("TUBE__WORKSPACE__DIR", workspace_dir),
    EnvVarGuard::set("TUBE__EXECUTION__USER_SCRIPT_PATH", script_path),
  ];
  let config = TubeConfig::load().unwrap();
  (config, guards)
}

#[tokio::test]
#[serial]
async fn test_full_pipeline_success_no_workspace() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .expect(3)
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests
    .iter()
    .filter(|r| r.method == wiremock::http::Method::PUT)
    .collect();
  assert_eq!(put_requests.len(), 3);

  let finished_put = put_requests
    .iter()
    .find(|r| {
      r.headers
        .get("content-type")
        .map(|v| v.to_str().is_ok_and(|s| s.contains("json")))
        .unwrap_or(false)
        && serde_json::from_slice::<Value>(&r.body)
          .ok()
          .and_then(|v| v.get("state").cloned())
          == Some(Value::String("Finished".into()))
    })
    .expect("expected a finished status PUT");

  let body: Value = serde_json::from_slice(&finished_put.body).unwrap();
  assert_eq!(body["state"], "Finished");
  assert_eq!(body["success"], true);
  assert!(body["finished_at"].is_number());
}

#[tokio::test]
#[serial]
async fn test_full_pipeline_with_workspace() {
  let server = MockServer::start().await;

  let mut archive_data: Vec<u8> = Vec::new();
  {
    let encoder = GzEncoder::new(&mut archive_data, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let content = b"hello world";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
      .append_data(&mut header, "repo-abc123/hello.txt", content.as_ref())
      .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
  }

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(archive_data))
    .expect(1)
    .mount(&server)
    .await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    &format!("{}/archive.tar.gz", server.uri()),
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let extracted = std::fs::read_to_string(workspace.path().join("hello.txt")).unwrap();
  assert_eq!(extracted, "hello world");
}

#[tokio::test]
#[serial]
async fn test_full_pipeline_with_zstd_workspace() {
  let server = MockServer::start().await;

  let mut archive_data: Vec<u8> = Vec::new();
  {
    let encoder = zstd::Encoder::new(&mut archive_data, 0).unwrap();
    let mut builder = tar::Builder::new(encoder);
    let content = b"hello from zstd";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
      .append_data(&mut header, "repo-abc123/hello.txt", content.as_ref())
      .unwrap();
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();
  }

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(archive_data))
    .expect(1)
    .mount(&server)
    .await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    &format!("{}/archive.tar.zst", server.uri()),
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let extracted = std::fs::read_to_string(workspace.path().join("hello.txt")).unwrap();
  assert_eq!(extracted, "hello from zstd");
}

#[tokio::test]
#[serial]
async fn test_script_failure_writes_success_false() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 1");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(
    result.is_ok(),
    "run() should swallow script exit code, got: {:?}",
    result
  );

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests
    .iter()
    .filter(|r| r.method == wiremock::http::Method::PUT)
    .collect();
  let finished_put = put_requests
    .iter()
    .find(|r| {
      serde_json::from_slice::<Value>(&r.body)
        .ok()
        .and_then(|v| v.get("state").cloned())
        == Some(Value::String("Finished".into()))
    })
    .expect("expected finished PUT");
  let body: Value = serde_json::from_slice(&finished_put.body).unwrap();
  assert_eq!(body["success"], false);
}

#[tokio::test]
#[serial]
async fn test_corrupt_archive_reports_failure() {
  let server = MockServer::start().await;

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not valid gzip".to_vec()))
    .expect(1)
    .mount(&server)
    .await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    &format!("{}/archive", server.uri()),
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(
    result.is_ok(),
    "run() should return Ok after reporting workspace error, got: {:?}",
    result
  );

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests
    .iter()
    .filter(|r| r.method == wiremock::http::Method::PUT)
    .collect();

  let finished_put = put_requests
    .iter()
    .find(|r| {
      serde_json::from_slice::<Value>(&r.body)
        .ok()
        .and_then(|v| v.get("state").cloned())
        == Some(Value::String("Finished".into()))
    })
    .expect("expected finished PUT");
  let body: Value = serde_json::from_slice(&finished_put.body).unwrap();
  assert_eq!(
    body["success"], false,
    "expected success=false for corrupt archive"
  );
}

#[tokio::test]
#[serial]
async fn test_status_endpoint_5xx_returns_status_error() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(500))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(
    matches!(result, Err(TubeError::Status(_))),
    "expected Status error, got: {:?}",
    result
  );
}

#[tokio::test]
#[serial]
async fn test_workspace_preserves_file_permissions() {
  let server = MockServer::start().await;

  let mut archive_data: Vec<u8> = Vec::new();
  {
    let encoder = GzEncoder::new(&mut archive_data, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let content = b"#!/bin/sh\nexit 0";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
      .append_data(&mut header, "repo-abc123/script.sh", content.as_ref())
      .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
  }

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(archive_data))
    .mount(&server)
    .await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    &format!("{}/archive.tar.gz", server.uri()),
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  tube::run(config, reqwest::Client::new()).await.unwrap();

  let mode = std::fs::metadata(workspace.path().join("script.sh"))
    .unwrap()
    .permissions()
    .mode();
  assert_eq!(
    mode & 0o777,
    0o755,
    "extracted file should preserve 0o755 permissions"
  );
}

#[tokio::test]
#[serial]
async fn test_log_upload_on_success() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;

  let script = make_script("echo hello && echo world >&2");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests
    .iter()
    .filter(|r| r.method == wiremock::http::Method::PUT)
    .collect();

  // started PUT + logs PUT + finished PUT
  assert_eq!(put_requests.len(), 3);

  let log_put = put_requests
    .iter()
    .find(|r| {
      r.headers
        .get("content-type")
        .map(|v| v.to_str().is_ok_and(|s| s.starts_with("text/plain")))
        .unwrap_or(false)
    })
    .expect("expected a text/plain PUT for log upload");

  let body = std::str::from_utf8(&log_put.body).unwrap();
  assert!(body.contains("hello"), "log body should contain stdout");
  assert!(body.contains("world"), "log body should contain stderr");
  assert!(body.contains("[stdout]"));
  assert!(body.contains("[stderr]"));
}

#[tokio::test]
#[serial]
async fn test_periodic_log_uploads_during_long_script() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;

  let script = make_script("for i in 1 2 3 4 5; do echo line$i; sleep 0.2; done");
  let workspace = TempDir::new().unwrap();
  let _interval_guard = EnvVarGuard::set("TUBE__EXECUTION__LOG_UPLOAD_INTERVAL_MS", "100");
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    &format!("{}/logs", server.uri()),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let log_puts: Vec<_> = requests
    .iter()
    .filter(|r| {
      r.method == wiremock::http::Method::PUT
        && r
          .headers
          .get("content-type")
          .map(|v| v.to_str().is_ok_and(|s| s.starts_with("text/plain")))
          .unwrap_or(false)
    })
    .collect();

  assert!(
    log_puts.len() >= 2,
    "expected at least one periodic log PUT plus the final upload, got {}",
    log_puts.len()
  );

  let first_body_len = log_puts.first().unwrap().body.len();
  let last_body_len = log_puts.last().unwrap().body.len();
  assert!(
    last_body_len > first_body_len,
    "final log body should be larger than the first periodic upload (first={}, last={})",
    first_body_len,
    last_body_len
  );

  let final_body = std::str::from_utf8(&log_puts.last().unwrap().body).unwrap();
  for i in 1..=5 {
    assert!(
      final_body.contains(&format!("line{i}")),
      "final body should contain line{i}"
    );
  }
}

#[tokio::test]
#[serial]
async fn test_log_upload_failure_does_not_fail_run() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let (config, _guards) = load_config(
    &server,
    "",
    workspace.path().to_str().unwrap(),
    script.to_str().unwrap(),
    "http://127.0.0.1:1/logs",
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(
    result.is_ok(),
    "run() should succeed even if log upload fails"
  );
}
