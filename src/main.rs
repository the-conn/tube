use thiserror::Error;
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;

mod execution;
mod status_update;
mod tube_config;
mod workspace;

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

fn setup_tracing(log_level: Level) {
  let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();

  tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");
}

#[tokio::main]
async fn main() -> Result<(), TubeError> {
  let config = tube_config::TubeConfig::load()?;
  info!("Configuration loaded");

  setup_tracing(config.log_level());
  let client = reqwest::Client::new();

  info!("Starting node execution");
  let started_at = status_update::write_started_update(&client, &config).await?;
  let run_result: Result<i32, TubeError> = async {
    workspace::create_workspace(&config).await?;
    execution::execute_script(&config)
      .await
      .map_err(|e| TubeError::Execution(e))
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
