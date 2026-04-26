use std::env;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use thiserror::Error;
use tracing::Level;

#[derive(Error, Debug)]
pub enum TubeConfigError {
  #[error("Configuration loading failed: {0}")]
  Config(#[from] ConfigError),
  #[error("Required configuration field is empty: {0}")]
  RequiredField(String),
}

#[derive(Debug, Deserialize)]
struct ExecutionConfig {
  put_url: String,
  poke_url: String,
  run_id: String,
  node_name: String,
  user_script_path: String,
}

#[derive(Debug, Deserialize)]
struct LogConfig {
  level: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceConfig {
  get_url: String,
  dir: String,
}

#[derive(Debug, Deserialize)]
pub struct TubeConfig {
  execution: ExecutionConfig,
  log: LogConfig,
  workspace: WorkspaceConfig,
}

impl TubeConfig {
  pub fn load() -> Result<Self, TubeConfigError> {
    let environment = env::var("ENV").unwrap_or_else(|_| "dev".into());

    let builder = Config::builder()
      .add_source(File::with_name("config/default").required(false))
      .add_source(File::with_name(&format!("config/{}", environment)).required(false))
      .add_source(Environment::with_prefix("TUBE").separator("__"))
      .build()?;

    let tc: Self = builder.try_deserialize().map_err(TubeConfigError::from)?;

    if tc.execution.user_script_path.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__USER_SCRIPT_PATH".into(),
      ));
    }
    if tc.execution.put_url.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__PUT_URL".into(),
      ));
    }
    if tc.execution.poke_url.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__POKE_URL".into(),
      ));
    }
    if tc.execution.run_id.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__RUN_ID".into(),
      ));
    }
    if tc.execution.node_name.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__NODE_NAME".into(),
      ));
    }
    if tc.workspace.dir.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__WORKSPACE__DIR".into(),
      ));
    }

    return Ok(tc);
  }

  pub fn get_url(&self) -> &str {
    &self.workspace.get_url
  }

  pub fn put_url(&self) -> &str {
    &self.execution.put_url
  }

  pub fn poke_url(&self) -> &str {
    &self.execution.poke_url
  }

  pub fn run_id(&self) -> &str {
    &self.execution.run_id
  }

  pub fn node_name(&self) -> &str {
    &self.execution.node_name
  }

  pub fn log_level(&self) -> Level {
    self.log.level.parse().unwrap_or(Level::INFO)
  }

  pub fn workspace_dir(&self) -> &str {
    &self.workspace.dir
  }

  pub fn script_path(&self) -> &str {
    &self.execution.user_script_path
  }
}
