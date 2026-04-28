use std::{
  os::unix::process::ExitStatusExt,
  path::Path,
  process::Stdio,
  time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;
use tokio::{
  io::{AsyncBufReadExt, BufReader},
  process::Command,
};
use tracing::info;

use crate::tube_config::TubeConfig;

const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;

#[derive(Error, Debug)]
pub enum ExecutionError {
  #[error("Script execution failed: {0}")]
  Execution(#[from] std::io::Error),
  #[error("Script terminated by signal: {0}")]
  Signal(i32),
  #[error("Process pipe was not captured")]
  Pipe,
}

pub async fn execute_script(config: &TubeConfig) -> Result<(i32, Vec<u8>), ExecutionError> {
  run_script(
    config.run_id().to_string(),
    config.node_name().to_string(),
    config.script_path().to_string(),
    config.workspace_dir().to_string(),
  )
  .await
}

#[tracing::instrument]
async fn run_script(
  run_id: String,
  node_name: String,
  script_path: String,
  workspace: String,
) -> Result<(i32, Vec<u8>), ExecutionError> {
  info!("Starting user script execution");

  let mut child = Command::new("sh")
    .arg("-c")
    .arg(&script_path)
    .current_dir(Path::new(&workspace))
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

  let stdout = child.stdout.take().ok_or(ExecutionError::Pipe)?;
  let stderr = child.stderr.take().ok_or(ExecutionError::Pipe)?;

  let mut stdout_lines = BufReader::new(stdout).lines();
  let mut stderr_lines = BufReader::new(stderr).lines();

  let mut buffer: Vec<u8> = Vec::new();
  let mut stdout_done = false;
  let mut stderr_done = false;

  loop {
    if stdout_done && stderr_done {
      break;
    }
    tokio::select! {
      line = stdout_lines.next_line(), if !stdout_done => {
        match line? {
          Some(l) => {
            info!(target: "user_logs", stream = "stdout", "{}", l);
            append_to_buffer(&mut buffer, "stdout", &l);
          }
          None => stdout_done = true,
        }
      }
      line = stderr_lines.next_line(), if !stderr_done => {
        match line? {
          Some(l) => {
            info!(target: "user_logs", stream = "stderr", "{}", l);
            append_to_buffer(&mut buffer, "stderr", &l);
          }
          None => stderr_done = true,
        }
      }
    }
  }

  let status = child.wait().await?;

  if let Some(code) = status.code() {
    info!("User script exited with code: {}", code);
    Ok((code, buffer))
  } else {
    let signal = status.signal().unwrap_or(-1);
    Err(ExecutionError::Signal(signal))
  }
}

fn append_to_buffer(buffer: &mut Vec<u8>, stream: &str, line: &str) {
  if buffer.len() >= MAX_LOG_BYTES {
    return;
  }
  let ts = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0);
  let entry = format!("[{}] [{}] {}\n", ts, stream, line);
  let entry_bytes = entry.as_bytes();
  let space_left = MAX_LOG_BYTES - buffer.len();
  buffer.extend_from_slice(&entry_bytes[..space_left.min(entry_bytes.len())]);
}

#[cfg(test)]
mod tests {
  use std::{env, io::Write, os::unix::fs::PermissionsExt};

  use serial_test::serial;
  use tempfile::{NamedTempFile, TempDir};

  use super::*;

  fn make_script(content: &str) -> tempfile::TempPath {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "{}", content).unwrap();
    std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    f.into_temp_path()
  }

  fn load_config(script_path: &str, workspace_dir: &str) -> TubeConfig {
    unsafe {
      env::set_var("TUBE__EXECUTION__USER_SCRIPT_PATH", script_path);
      env::set_var("TUBE__WORKSPACE__DIR", workspace_dir);
    }
    TubeConfig::load().unwrap()
  }

  #[tokio::test]
  #[serial]
  async fn test_exit_code_zero() {
    let script = make_script("exit 0");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config).await.unwrap().0, 0);
  }

  #[tokio::test]
  #[serial]
  async fn test_exit_code_nonzero() {
    let script = make_script("exit 42");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config).await.unwrap().0, 42);
  }

  #[tokio::test]
  #[serial]
  async fn test_signal_termination() {
    let workspace = TempDir::new().unwrap();
    let config = load_config("kill -9 $$", workspace.path().to_str().unwrap());
    assert!(matches!(
      execute_script(&config).await,
      Err(ExecutionError::Signal(9))
    ));
  }

  #[tokio::test]
  #[serial]
  async fn test_nonexistent_workspace_dir_returns_io_error() {
    let script = make_script("exit 0");
    let config = load_config(script.to_str().unwrap(), "/nonexistent/workspace/dir");
    assert!(matches!(
      execute_script(&config).await,
      Err(ExecutionError::Execution(_))
    ));
  }

  #[tokio::test]
  #[serial]
  async fn test_stdout_captured_in_buffer() {
    let script = make_script("echo hello");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let (code, buffer) = execute_script(&config).await.unwrap();
    assert_eq!(code, 0);
    let output = String::from_utf8(buffer).unwrap();
    assert!(output.contains("hello"));
    assert!(output.contains("[stdout]"));
  }

  #[tokio::test]
  #[serial]
  async fn test_stderr_captured_in_buffer() {
    let script = make_script("echo err_line >&2");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let (_, buffer) = execute_script(&config).await.unwrap();
    let output = String::from_utf8(buffer).unwrap();
    assert!(output.contains("err_line"));
    assert!(output.contains("[stderr]"));
  }

  #[tokio::test]
  #[serial]
  async fn test_buffer_capped_at_10mb() {
    let script = make_script(
      "yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa | head -n 120000",
    );
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let (_, buffer) = execute_script(&config).await.unwrap();
    assert!(buffer.len() <= MAX_LOG_BYTES);
  }
}
