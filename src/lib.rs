pub mod execution;
pub mod status_update;
pub mod tube_config;
pub mod workspace;

use thiserror::Error;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Error, Debug)]
pub enum TubeError {
  #[error("Configuration error: {0}")]
  Config(#[from] crate::tube_config::TubeConfigError),
  #[error("Workspace error: {0}")]
  Workspace(#[from] crate::workspace::WorksapceError),
  #[error("Execution error: {0}")]
  Execution(#[from] crate::execution::ExecutionError),
  #[error("Status reporting error: {0}")]
  Status(#[from] crate::status_update::StatusError),
}

pub async fn run(
  config: tube_config::TubeConfig,
  client: reqwest::Client,
) -> Result<(), TubeError> {
  let system_level = config.log_level();
  let user_level = config.execution_log_level();
  let filter = EnvFilter::new(format!("off,tube={system_level},user_logs={user_level}"));
  let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

  info!("Starting node execution");
  let started_at = status_update::write_started_update(&client, &config).await?;

  let (success, log_buffer) = match run_workspace_and_script(&config).await {
    Ok((0, buf)) => {
      info!("Node execution successful");
      (true, buf)
    }
    Ok((code, buf)) => {
      info!("Node execution failed with exit code: {}", code);
      (false, buf)
    }
    Err(e) => {
      error!("System error during execution: {}", e);
      (false, Vec::new())
    }
  };

  status_update::upload_logs(&client, config.logs_put_url(), log_buffer).await;

  status_update::write_finished_update(&client, &config, started_at, success).await?;
  status_update::poke_backend(&config, &client).await?;

  Ok(())
}

async fn run_workspace_and_script(
  config: &tube_config::TubeConfig,
) -> Result<(i32, Vec<u8>), TubeError> {
  workspace::create_workspace(config).await?;
  execution::execute_script(config)
    .await
    .map_err(TubeError::Execution)
}
