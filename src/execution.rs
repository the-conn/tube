use std::{
  os::unix::process::ExitStatusExt,
  path::Path,
  process::{Command, Stdio},
};

use thiserror::Error;
use tracing::info;

use crate::tube_config::TubeConfig;

#[derive(Error, Debug)]
pub enum ExecutionError {
  #[error("Script execution failed: {0}")]
  Execution(#[from] std::io::Error),
  #[error("Script terminated by signal: {0}")]
  Signal(i32),
}

pub async fn execute_script(config: &TubeConfig) -> Result<i32, ExecutionError> {
  let script_path = config.script_path();
  let workspace = config.workspace_dir();

  info!("Starting user script execution in {}", workspace);
  let mut child = Command::new("sh")
    .arg("-c")
    .arg(script_path)
    .current_dir(Path::new(workspace))
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .spawn()?;
  let status = child.wait()?;

  if let Some(code) = status.code() {
    info!("User script exited with code: {}", code);
    Ok(code)
  } else {
    let signal = status.signal().unwrap_or(-1);
    Err(ExecutionError::Signal(signal))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;
  use std::os::unix::fs::PermissionsExt;
  use tempfile::{NamedTempFile, TempDir};

  // Returns a TempPath (fd closed) so the file is executable without ETXTBSY.
  fn make_script(content: &str) -> tempfile::TempPath {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "{}", content).unwrap();
    std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    f.into_temp_path()
  }

  fn config_with_script(script_path: &str, workspace_dir: &str) -> TubeConfig {
    TubeConfig::new_for_test("u", "u", "", workspace_dir, "r", "n", script_path)
  }

  #[tokio::test]
  async fn test_exit_code_zero() {
    let script = make_script("exit 0");
    let workspace = TempDir::new().unwrap();
    let config = config_with_script(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config).await.unwrap(), 0);
  }

  #[tokio::test]
  async fn test_exit_code_nonzero() {
    let script = make_script("exit 42");
    let workspace = TempDir::new().unwrap();
    let config = config_with_script(script.to_str().unwrap(), workspace.path().to_str().unwrap());
    assert_eq!(execute_script(&config).await.unwrap(), 42);
  }

  #[tokio::test]
  async fn test_signal_termination() {
    // Pass the kill command directly as script_path so sh -c "kill -9 $$" kills itself.
    // Using a script file adds an extra sh layer, producing exit 137 instead of a signal.
    let workspace = TempDir::new().unwrap();
    let config = config_with_script("kill -9 $$", workspace.path().to_str().unwrap());
    assert!(matches!(execute_script(&config).await, Err(ExecutionError::Signal(9))));
  }

  #[tokio::test]
  async fn test_nonexistent_workspace_dir_returns_io_error() {
    // spawn() fails with ENOENT when current_dir doesn't exist, triggering ExecutionError::Execution
    let script = make_script("exit 0");
    let config = config_with_script(script.to_str().unwrap(), "/nonexistent/workspace/dir");
    assert!(matches!(execute_script(&config).await, Err(ExecutionError::Execution(_))));
  }
}
