use futures_util::StreamExt;
use tar::Archive;
use thiserror::Error;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::info;
use zstd::Decoder;

use crate::tube_config::TubeConfig;

#[derive(Error, Debug)]
pub enum WorksapceError {
  #[error("Network request failed: {0}")]
  Network(#[from] reqwest::Error),

  #[error("Internal threading error: {0}")]
  ThreadJoin(#[from] tokio::task::JoinError),

  #[error("Archive or Decompression IO failed: {0}")]
  Io(#[from] std::io::Error),
}

pub async fn create_workspace(config: &TubeConfig) -> Result<(), WorksapceError> {
  if config.get_url().is_empty() {
    info!("No get_url provided, skipping repo cloning");
    return Ok(());
  }

  info!("Streaming repo archive from S3...");
  let stream = reqwest::get(config.get_url())
    .await?
    .bytes_stream()
    .map(|res| res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
  let async_reader = StreamReader::new(stream);
  let sync_reader = SyncIoBridge::new(async_reader);

  info!("Decompressing and unpacking on the fly...");
  let workspace_dir = config.workspace_dir().to_string();
  tokio::task::spawn_blocking(move || {
    let zstd_decoder = Decoder::new(sync_reader)?;
    let mut archive = Archive::new(zstd_decoder);
    archive.unpack(workspace_dir)
  })
  .await??;

  info!("Workspace created successfully");
  Ok(())
}
