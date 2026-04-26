use std::time::{SystemTime, SystemTimeError};

use reqwest::Client;
use serde::Serialize;
use thiserror::Error;

use crate::tube_config::TubeConfig;

#[derive(Error, Debug)]
pub enum StatusError {
  #[error("Failed to get current time: {0}")]
  Time(#[from] SystemTimeError),
  #[error("Network request failed: {0}")]
  Network(#[from] reqwest::Error),
  #[error("Serialization failed: {0}")]
  Json(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct StatusUpdate {
  run_id: String,
  node_name: String,
  state: State,
  started_at: u128,
  #[serde(skip_serializing_if = "Option::is_none")]
  finished_at: Option<u128>,
  #[serde(skip_serializing_if = "Option::is_none")]
  success: Option<bool>,
}

#[derive(Serialize)]
enum State {
  Started,
  Finished,
}

fn time_millis() -> Result<u128, StatusError> {
  let t = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)?
    .as_millis();
  Ok(t)
}

pub async fn write_started_update(
  client: &Client,
  config: &TubeConfig,
) -> Result<u128, StatusError> {
  let now = time_millis()?;
  let update = StatusUpdate {
    run_id: config.run_id().to_string(),
    node_name: config.node_name().to_string(),
    state: State::Started,
    started_at: now,
    finished_at: None,
    success: None,
  };

  client
    .put(config.put_url())
    .json(&update)
    .send()
    .await?
    .error_for_status()?;

  Ok(now)
}

pub async fn write_finished_update(
  client: &Client,
  config: &TubeConfig,
  started_at: u128,
  success: bool,
) -> Result<(), StatusError> {
  let now = time_millis()?;
  let update = StatusUpdate {
    run_id: config.run_id().to_string(),
    node_name: config.node_name().to_string(),
    state: State::Finished,
    started_at: started_at,
    finished_at: Some(now),
    success: Some(success),
  };

  client
    .put(config.put_url())
    .json(&update)
    .send()
    .await?
    .error_for_status()?;

  Ok(())
}

pub async fn poke_backend(config: &TubeConfig, client: &Client) -> Result<(), StatusError> {
  client
    .post(config.poke_url())
    .send()
    .await?
    .error_for_status()?;

  Ok(())
}
