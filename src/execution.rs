use std::{
  collections::VecDeque,
  fmt,
  os::unix::process::ExitStatusExt,
  path::Path,
  process::Stdio,
  sync::Arc,
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use thiserror::Error;
use tokio::{
  io::{AsyncBufReadExt, BufReader},
  process::Command,
  sync::Mutex,
  task::JoinHandle,
  time::{MissedTickBehavior, interval},
};
use tracing::info;

use crate::{status_update, tube_config::TubeConfig};

const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;

pub struct LogBuffer {
  entries: VecDeque<Vec<u8>>,
  total_bytes: usize,
}

impl LogBuffer {
  pub fn new() -> Self {
    Self {
      entries: VecDeque::new(),
      total_bytes: 0,
    }
  }

  pub fn push(&mut self, stream: &str, line: &str) {
    let ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_millis())
      .unwrap_or(0);
    let mut entry = format!("[{}] [{}] {}\n", ts, stream, line).into_bytes();

    if entry.len() > MAX_LOG_BYTES {
      let drop = entry.len() - MAX_LOG_BYTES;
      entry.drain(..drop);
      self.entries.clear();
      self.total_bytes = entry.len();
      self.entries.push_back(entry);
      return;
    }

    while self.total_bytes + entry.len() > MAX_LOG_BYTES {
      let Some(oldest) = self.entries.pop_front() else {
        break;
      };
      self.total_bytes -= oldest.len();
    }

    self.total_bytes += entry.len();
    self.entries.push_back(entry);
  }

  pub fn snapshot(&self) -> Vec<u8> {
    let mut out = Vec::with_capacity(self.total_bytes);
    for entry in &self.entries {
      out.extend_from_slice(entry);
    }
    out
  }

  pub fn into_bytes(mut self) -> Vec<u8> {
    if self.entries.len() == 1 {
      return self.entries.pop_front().unwrap_or_default();
    }
    self.snapshot()
  }

  pub fn is_empty(&self) -> bool {
    self.total_bytes == 0
  }

  pub fn total_bytes(&self) -> usize {
    self.total_bytes
  }
}

impl Default for LogBuffer {
  fn default() -> Self {
    Self::new()
  }
}

impl fmt::Debug for LogBuffer {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("LogBuffer")
      .field("entries", &self.entries.len())
      .field("total_bytes", &self.total_bytes)
      .finish()
  }
}

#[derive(Error, Debug)]
pub enum ExecutionError {
  #[error("Script execution failed: {0}")]
  Execution(#[from] std::io::Error),
  #[error("Script terminated by signal: {0}")]
  Signal(i32),
  #[error("Process pipe was not captured")]
  Pipe,
}

pub async fn execute_script(
  config: &TubeConfig,
  buffer: Arc<Mutex<LogBuffer>>,
) -> Result<i32, ExecutionError> {
  run_script(
    config.run_id().to_string(),
    config.node_name().to_string(),
    config.script_path().to_string(),
    config.workspace_dir().to_string(),
    buffer,
  )
  .await
}

#[tracing::instrument(skip(buffer))]
async fn run_script(
  run_id: String,
  node_name: String,
  script_path: String,
  workspace: String,
  buffer: Arc<Mutex<LogBuffer>>,
) -> Result<i32, ExecutionError> {
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
            buffer.lock().await.push("stdout", &l);
          }
          None => stdout_done = true,
        }
      }
      line = stderr_lines.next_line(), if !stderr_done => {
        match line? {
          Some(l) => {
            info!(target: "user_logs", stream = "stderr", "{}", l);
            buffer.lock().await.push("stderr", &l);
          }
          None => stderr_done = true,
        }
      }
    }
  }

  let status = child.wait().await?;

  if let Some(code) = status.code() {
    info!("User script exited with code: {}", code);
    Ok(code)
  } else {
    let signal = status.signal().unwrap_or(-1);
    Err(ExecutionError::Signal(signal))
  }
}

pub fn spawn_periodic_uploader(
  client: Client,
  url: String,
  buffer: Arc<Mutex<LogBuffer>>,
  period: Duration,
) -> JoinHandle<()> {
  tokio::spawn(async move {
    let mut ticker = interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_uploaded_bytes: usize = 0;
    loop {
      ticker.tick().await;
      let snapshot = {
        let guard = buffer.lock().await;
        if guard.is_empty() || guard.total_bytes() == last_uploaded_bytes {
          continue;
        }
        last_uploaded_bytes = guard.total_bytes();
        guard.snapshot()
      };
      status_update::upload_logs(&client, &url, snapshot).await;
    }
  })
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

  fn new_buffer() -> Arc<Mutex<LogBuffer>> {
    Arc::new(Mutex::new(LogBuffer::new()))
  }

  async fn drain(buffer: Arc<Mutex<LogBuffer>>) -> Vec<u8> {
    buffer.lock().await.snapshot()
  }

  #[tokio::test]
  #[serial]
  async fn test_exit_code_zero() {
    let script = make_script("exit 0");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config, new_buffer()).await.unwrap(), 0);
  }

  #[tokio::test]
  #[serial]
  async fn test_exit_code_nonzero() {
    let script = make_script("exit 42");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config, new_buffer()).await.unwrap(), 42);
  }

  #[tokio::test]
  #[serial]
  async fn test_signal_termination() {
    let workspace = TempDir::new().unwrap();
    let config = load_config("kill -9 $$", workspace.path().to_str().unwrap());
    assert!(matches!(
      execute_script(&config, new_buffer()).await,
      Err(ExecutionError::Signal(9))
    ));
  }

  #[tokio::test]
  #[serial]
  async fn test_nonexistent_workspace_dir_returns_io_error() {
    let script = make_script("exit 0");
    let config = load_config(script.to_str().unwrap(), "/nonexistent/workspace/dir");
    assert!(matches!(
      execute_script(&config, new_buffer()).await,
      Err(ExecutionError::Execution(_))
    ));
  }

  #[tokio::test]
  #[serial]
  async fn test_stdout_captured_in_buffer() {
    let script = make_script("echo hello");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let buf = new_buffer();
    let code = execute_script(&config, Arc::clone(&buf)).await.unwrap();
    assert_eq!(code, 0);
    let output = String::from_utf8(drain(buf).await).unwrap();
    assert!(output.contains("hello"));
    assert!(output.contains("[stdout]"));
  }

  #[tokio::test]
  #[serial]
  async fn test_stderr_captured_in_buffer() {
    let script = make_script("echo err_line >&2");
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let buf = new_buffer();
    let _ = execute_script(&config, Arc::clone(&buf)).await.unwrap();
    let output = String::from_utf8(drain(buf).await).unwrap();
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
    let buf = new_buffer();
    let _ = execute_script(&config, Arc::clone(&buf)).await.unwrap();
    let total = buf.lock().await.total_bytes();
    assert!(total <= MAX_LOG_BYTES);
  }

  #[tokio::test]
  #[serial]
  async fn test_buffer_keeps_late_sentinel_when_overflowing() {
    let script = make_script(
      "yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa | head -n 130000\necho LATE_SENTINEL_zzzzzzzzzzzzzzzzzzzzzz",
    );
    let workspace = TempDir::new().unwrap();
    let config = load_config(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    let buf = new_buffer();
    let _ = execute_script(&config, Arc::clone(&buf)).await.unwrap();
    let total = buf.lock().await.total_bytes();
    assert!(total <= MAX_LOG_BYTES);
    let output = String::from_utf8(drain(buf).await).unwrap();
    assert!(
      output.contains("LATE_SENTINEL_zzzzzzzzzzzzzzzzzzzzzz"),
      "expected the most recent line to survive eviction"
    );
  }

  #[test]
  fn test_log_buffer_fifo_eviction_unit() {
    let mut buf = LogBuffer::new();
    let big_line = "x".repeat(1024 * 1024);
    for i in 0..15 {
      buf.push("stdout", &format!("ENTRY_{i:02}_{big_line}"));
    }
    assert!(buf.total_bytes() <= MAX_LOG_BYTES);
    let bytes = buf.into_bytes();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
      !text.contains("ENTRY_00_"),
      "earliest entry should be evicted"
    );
    assert!(
      text.contains("ENTRY_14_"),
      "most recent entry should be retained"
    );
  }

  #[test]
  fn test_log_buffer_oversized_single_entry_keeps_tail() {
    let mut buf = LogBuffer::new();
    buf.push("stdout", "earlier line");
    let huge = "y".repeat(MAX_LOG_BYTES + 1024);
    buf.push("stdout", &huge);
    assert!(buf.total_bytes() <= MAX_LOG_BYTES);
    let bytes = buf.into_bytes();
    assert!(bytes.len() <= MAX_LOG_BYTES);
    assert!(
      !bytes.starts_with(b"["),
      "tail-truncated entry should not begin with the original timestamp"
    );
    assert!(
      bytes.ends_with(b"y\n"),
      "the surviving content should be the tail of the oversized line"
    );
  }
}
