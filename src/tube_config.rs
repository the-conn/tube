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

impl TubeConfig {
  pub fn new_for_test(
    put_url: &str,
    poke_url: &str,
    get_url: &str,
    workspace_dir: &str,
    run_id: &str,
    node_name: &str,
    user_script_path: &str,
  ) -> Self {
    TubeConfig {
      execution: ExecutionConfig {
        put_url: put_url.into(),
        poke_url: poke_url.into(),
        run_id: run_id.into(),
        node_name: node_name.into(),
        user_script_path: user_script_path.into(),
      },
      log: LogConfig { level: "info".into() },
      workspace: WorkspaceConfig {
        get_url: get_url.into(),
        dir: workspace_dir.into(),
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serial_test::serial;

  struct EnvVarGuard {
    key: String,
    original: Option<String>,
  }

  impl EnvVarGuard {
    fn set(key: &str, val: &str) -> Self {
      let original = env::var(key).ok();
      unsafe { env::set_var(key, val) };
      EnvVarGuard { key: key.to_string(), original }
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
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__EXECUTION__PUT_URL", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("PUT_URL")));
  }

  #[test]
  #[serial]
  fn test_missing_poke_url() {
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__EXECUTION__POKE_URL", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("POKE_URL")));
  }

  #[test]
  #[serial]
  fn test_missing_run_id() {
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__EXECUTION__RUN_ID", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("RUN_ID")));
  }

  #[test]
  #[serial]
  fn test_missing_node_name() {
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__EXECUTION__NODE_NAME", "");
    let result = TubeConfig::load();
    assert!(matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("NODE_NAME")));
  }

  #[test]
  #[serial]
  fn test_missing_user_script_path() {
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__EXECUTION__USER_SCRIPT_PATH", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("USER_SCRIPT_PATH"))
    );
  }

  #[test]
  #[serial]
  fn test_missing_workspace_dir() {
    let _env = EnvVarGuard::set("ENV", "test");
    let _g = EnvVarGuard::set("TUBE__WORKSPACE__DIR", "");
    let result = TubeConfig::load();
    assert!(
      matches!(result, Err(TubeConfigError::RequiredField(ref s)) if s.contains("WORKSPACE__DIR"))
    );
  }

  #[test]
  #[serial]
  fn test_load_succeeds_with_all_required_vars() {
    let _env = EnvVarGuard::set("ENV", "test");
    let result = TubeConfig::load();
    assert!(result.is_ok());
  }

  #[test]
  fn test_log_level_invalid_falls_back_to_info() {
    let mut config = TubeConfig::new_for_test("u", "u", "", "/tmp", "r", "n", "/s");
    config.log.level = "bogus".into();
    assert_eq!(config.log_level(), tracing::Level::INFO);
  }
}
