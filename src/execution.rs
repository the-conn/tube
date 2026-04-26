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
