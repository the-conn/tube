pub mod execution;
pub mod status_update;
pub mod tube_config;
pub mod workspace;

use thiserror::Error;
use tracing::{error, info};

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

pub async fn run(config: tube_config::TubeConfig, client: reqwest::Client) -> Result<(), TubeError> {
  let _ = tracing_subscriber::fmt()
    .with_max_level(config.log_level())
    .try_init();

  info!("Starting node execution");
  let started_at = status_update::write_started_update(&client, &config).await?;
  let run_result: Result<i32, TubeError> = async {
    workspace::create_workspace(&config).await?;
    execution::execute_script(&config)
      .await
      .map_err(TubeError::Execution)
  }
  .await;

  match run_result {
    Ok(exit_code) if exit_code == 0 => {
      info!("Node execution successful");
      status_update::write_finished_update(&client, &config, started_at, true).await?;
    }
    Ok(exit_code) => {
      info!("Node execution failed with exit code: {}", exit_code);
      status_update::write_finished_update(&client, &config, started_at, false).await?;
    }
    Err(e) => {
      error!("System error during execution: {}", e);
      status_update::write_finished_update(&client, &config, started_at, false).await?;
    }
  }
  status_update::poke_backend(&config, &client).await?;

  Ok(())
}
