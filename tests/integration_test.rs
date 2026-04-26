use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use serde_json::Value;
use tempfile::{NamedTempFile, TempDir};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use tube::TubeError;
use tube::tube_config::TubeConfig;

fn make_script(content: &str) -> tempfile::TempPath {
  let mut f = NamedTempFile::new().unwrap();
  writeln!(f, "#!/bin/sh").unwrap();
  writeln!(f, "{}", content).unwrap();
  std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
  f.into_temp_path()
}

#[tokio::test]
async fn test_full_pipeline_success_no_workspace() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .expect(2)
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    "",
    workspace.path().to_str().unwrap(),
    "run-1",
    "node-1",
    script.to_str().unwrap(),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests.iter().filter(|r| r.method == wiremock::http::Method::PUT).collect();
  assert_eq!(put_requests.len(), 2);

  let second_put_body: Value = serde_json::from_slice(&put_requests[1].body).unwrap();
  assert_eq!(second_put_body["state"], "Finished");
  assert_eq!(second_put_body["success"], true);
  assert!(second_put_body["finished_at"].is_number());
}

#[tokio::test]
async fn test_full_pipeline_with_workspace() {
  let server = MockServer::start().await;

  // Build tar.zst fixture in memory using existing production deps
  let mut archive_data: Vec<u8> = Vec::new();
  {
    let encoder = zstd::Encoder::new(&mut archive_data, 0).unwrap();
    let mut builder = tar::Builder::new(encoder);
    let content = b"hello world";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, "hello.txt", content.as_ref()).unwrap();
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
    .expect(2)
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    &format!("{}/archive", server.uri()),
    workspace.path().to_str().unwrap(),
    "run-2",
    "node-1",
    script.to_str().unwrap(),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() failed: {:?}", result);

  let extracted = std::fs::read_to_string(workspace.path().join("hello.txt")).unwrap();
  assert_eq!(extracted, "hello world");
}

#[tokio::test]
async fn test_script_failure_writes_success_false() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .expect(2)
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 1");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    "",
    workspace.path().to_str().unwrap(),
    "run-3",
    "node-1",
    script.to_str().unwrap(),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() should swallow script exit code, got: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests.iter().filter(|r| r.method == wiremock::http::Method::PUT).collect();
  let second_put_body: Value = serde_json::from_slice(&put_requests[1].body).unwrap();
  assert_eq!(second_put_body["success"], false);
}

#[tokio::test]
async fn test_corrupt_archive_reports_failure() {
  // run() catches workspace errors, reports them via HTTP, and returns Ok(()).
  let server = MockServer::start().await;

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not valid zstd".to_vec()))
    .expect(1)
    .mount(&server)
    .await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .expect(2)
    .mount(&server)
    .await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    &format!("{}/archive", server.uri()),
    workspace.path().to_str().unwrap(),
    "run-4",
    "node-1",
    script.to_str().unwrap(),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(result.is_ok(), "run() should return Ok after reporting workspace error, got: {:?}", result);

  let requests = server.received_requests().await.unwrap();
  let put_requests: Vec<_> = requests.iter().filter(|r| r.method == wiremock::http::Method::PUT).collect();
  assert_eq!(put_requests.len(), 2, "expected 2 PUT requests (started + finished)");

  let second_put_body: Value = serde_json::from_slice(&put_requests[1].body).unwrap();
  assert_eq!(second_put_body["success"], false, "expected success=false for corrupt archive");
}

#[tokio::test]
async fn test_status_endpoint_5xx_returns_status_error() {
  let server = MockServer::start().await;

  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(500))
    .expect(1)
    .mount(&server)
    .await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    "",
    workspace.path().to_str().unwrap(),
    "run-5",
    "node-1",
    script.to_str().unwrap(),
  );

  let result = tube::run(config, reqwest::Client::new()).await;
  assert!(
    matches!(result, Err(TubeError::Status(_))),
    "expected Status error, got: {:?}",
    result
  );
}

#[tokio::test]
async fn test_workspace_preserves_file_permissions() {
  let server = MockServer::start().await;

  let mut archive_data: Vec<u8> = Vec::new();
  {
    let encoder = zstd::Encoder::new(&mut archive_data, 0).unwrap();
    let mut builder = tar::Builder::new(encoder);
    let content = b"#!/bin/sh\nexit 0";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, "script.sh", content.as_ref()).unwrap();
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();
  }

  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(archive_data))
    .mount(&server).await;
  Mock::given(method("PUT"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server).await;
  Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&server).await;

  let script = make_script("exit 0");
  let workspace = TempDir::new().unwrap();
  let config = TubeConfig::new_for_test(
    &format!("{}/status", server.uri()),
    &format!("{}/poke", server.uri()),
    &format!("{}/archive", server.uri()),
    workspace.path().to_str().unwrap(),
    "run-perms",
    "node-1",
    script.to_str().unwrap(),
  );

  tube::run(config, reqwest::Client::new()).await.unwrap();

  let mode = std::fs::metadata(workspace.path().join("script.sh"))
    .unwrap().permissions().mode();
  assert_eq!(mode & 0o777, 0o755, "extracted file should preserve 0o755 permissions");
}
