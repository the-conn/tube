pub mod execution;
pub mod secrets;
pub mod status_update;
pub mod tube_config;
pub mod workspace;

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::{execution::LogBuffer, secrets::Secrets};

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
  #[error("Secrets error: {0}")]
  Secrets(#[from] crate::secrets::SecretsError),
  #[error("Reqwest error: {0}")]
  Reqwest(#[from] reqwest::Error),
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
  let secrets = Arc::new(Secrets::load(config.secrets_dir())?);
  let started_at = status_update::write_started_update(&client, &config).await?;

  let (success, log_buffer) = match run_workspace_and_script(&config, &client, &secrets).await {
    (Ok(0), buf) => {
      info!("Node execution successful");
      (true, buf)
    }
    (Ok(code), buf) => {
      info!("Node execution failed with exit code: {}", code);
      (false, buf)
    }
    (Err(e), buf) => {
      error!("System error during execution: {}", e);
      (false, buf)
    }
  };

  status_update::upload_logs(&client, config.logs_put_url(), log_buffer).await;

  status_update::write_finished_update(&client, &config, started_at, success).await?;
  status_update::poke_backend(&config, &client).await?;

  Ok(())
}

async fn run_workspace_and_script(
  config: &tube_config::TubeConfig,
  client: &reqwest::Client,
  secrets: &Arc<Secrets>,
) -> (Result<i32, TubeError>, Vec<u8>) {
  if let Err(e) = workspace::create_workspace(config, client).await {
    return (Err(e.into()), Vec::new());
  }

  let buffer = Arc::new(Mutex::new(LogBuffer::new()));
  let uploader = execution::spawn_periodic_uploader(
    client.clone(),
    config.logs_put_url().to_string(),
    Arc::clone(&buffer),
    config.log_upload_interval(),
  );

  let exit_code_res = execution::execute_script(config, Arc::clone(&buffer), secrets).await;

  uploader.abort();
  let _ = uploader.await;

  let final_bytes = match Arc::try_unwrap(buffer) {
    Ok(m) => m.into_inner().into_bytes(),
    Err(arc) => arc.lock().await.snapshot(),
  };

  match exit_code_res {
    Ok(code) => (Ok(code), final_bytes),
    Err(e) => (Err(e.into()), final_bytes),
  }
}
