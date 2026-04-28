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
  status_put_url: String,
  poke_url: String,
  run_id: String,
  node_name: String,
  user_script_path: String,
  log_level: String,
  logs_put_url: String,
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
    if tc.execution.status_put_url.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__STATUS_PUT_URL".into(),
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
    if tc.execution.logs_put_url.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__EXECUTION__LOGS_PUT_URL".into(),
      ));
    }
    if tc.workspace.dir.is_empty() {
      return Err(TubeConfigError::RequiredField(
        "TUBE__WORKSPACE__DIR".into(),
      ));
    }

    Ok(tc)
  }

  pub fn get_url(&self) -> &str {
    &self.workspace.get_url
  }

  pub fn status_put_url(&self) -> &str {
    &self.execution.status_put_url
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
    self.log.level.parse().unwrap_or(Level::WARN)
  }

  pub fn execution_log_level(&self) -> Level {
    self.execution.log_level.parse().unwrap_or(Level::INFO)
  }

  pub fn logs_put_url(&self) -> &str {
    &self.execution.logs_put_url
  }

  pub fn workspace_dir(&self) -> &str {
    &self.workspace.dir
  }

  pub fn script_path(&self) -> &str {
    &self.execution.user_script_path
  }
}

#[cfg(test)]
mod tests {
  use serial_test::serial;

  use super::*;

  struct EnvVarGuard {
    key: String,
    original: Option<String>,
  }

  impl EnvVarGuard {
    fn set(key: &str, val: &str) -> Self {
      let original = env::var(key).ok();
      unsafe { env::set_var(key, val) };
      EnvVarGuard {
        key: key.to_string(),
        original,
      }
    }
  }

  impl Drop for EnvVarGuard {
    fn drop(&mut self) {
      match &self.original {
        Some(v) => unsafe { env::set_var(&self.key, v) },
        None => unsafe { env::remove_var(&self.key) },
      }
    }
  }

  #[test]
  #[serial]
  fn test_missing_put_url() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__STATUS_PUT_URL", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("STATUS_PUT_URL"))
    );
  }

  #[test]
  #[serial]
  fn test_missing_poke_url() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__POKE_URL", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("POKE_URL")));
  }

  #[test]
  #[serial]
  fn test_missing_run_id() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__RUN_ID", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("RUN_ID")));
  }

  #[test]
  #[serial]
  fn test_missing_node_name() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__NODE_NAME", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("NODE_NAME"))
    );
  }

  #[test]
  #[serial]
  fn test_missing_user_script_path() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__USER_SCRIPT_PATH", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("USER_SCRIPT_PATH"))
    );
  }

  #[test]
  #[serial]
  fn test_missing_workspace_dir() {
    let _g = EnvVarGuard::set("TUBE__WORKSPACE__DIR", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("WORKSPACE__DIR"))
    );
  }

  #[test]
  #[serial]
  fn test_load_succeeds_with_all_required_vars() {
    assert!(TubeConfig::load().is_ok());
  }

  #[test]
  #[serial]
  fn test_log_level_invalid_falls_back_to_warn() {
    let _g = EnvVarGuard::set("TUBE__LOG__LEVEL", "bogus");
    let config = TubeConfig::load().unwrap();
    assert_eq!(config.log_level(), tracing::Level::WARN);
  }

  #[test]
  #[serial]
  fn test_execution_log_level_invalid_falls_back_to_info() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__LOG_LEVEL", "bogus");
    let config = TubeConfig::load().unwrap();
    assert_eq!(config.execution_log_level(), tracing::Level::INFO);
  }

  #[test]
  #[serial]
  fn test_missing_logs_put_url() {
    let _g = EnvVarGuard::set("TUBE__EXECUTION__LOGS_PUT_URL", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("LOGS_PUT_URL"))
    );
  }

  #[test]
  #[serial]
  fn test_logs_put_url_returns_configured_value() {
    let _g = EnvVarGuard::set(
      "TUBE__EXECUTION__LOGS_PUT_URL",
      "https://s3.example.com/logs",
    );
    let config = TubeConfig::load().unwrap();
    assert_eq!(config.logs_put_url(), "https://s3.example.com/logs");
  }
}
