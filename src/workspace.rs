use std::io::BufReader;

use flate2::read::GzDecoder;
use futures_util::StreamExt;
use tar::Archive;
use thiserror::Error;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::info;

use crate::tube_config::TubeConfig;

#[derive(Error, Debug)]
pub enum WorksapceError {
  #[error("Network request failed: {0}")]
  Network(#[from] reqwest::Error),

  #[error("Internal threading error: {0}")]
  ThreadJoin(#[from] tokio::task::JoinError),

  #[error("Archive or Decompression IO failed: {0}")]
  Io(#[from] std::io::Error),

  #[error("S3 download failed with status {0}: {1}")]
  S3(String, String),
}

enum ArchiveFormat {
  Gzip,
  Zstd,
}

fn detect_format(url: &str) -> ArchiveFormat {
  let lower = url.to_lowercase();
  if lower.contains(".tar.zst") || lower.contains(".zst") {
    ArchiveFormat::Zstd
  } else {
    ArchiveFormat::Gzip
  }
}

pub async fn create_workspace(config: &TubeConfig) -> Result<(), WorksapceError> {
  if config.get_url().is_empty() {
    info!("No get_url provided, skipping repo cloning");
    return Ok(());
  }

  info!("Streaming repo archive from S3...");
  let format = detect_format(config.get_url());
  let response = reqwest::get(config.get_url()).await?;
  if !response.status().is_success() {
    let status = response.status();
    let body = response
      .text()
      .await
      .unwrap_or_else(|_| "Could not read error body".to_string());
    return Err(WorksapceError::S3(status.as_str().to_string(), body));
  }
  let stream = response
    .bytes_stream()
    .map(|res| res.map_err(std::io::Error::other));
  let async_reader = StreamReader::new(stream);
  let sync_reader = SyncIoBridge::new(async_reader);

  info!("Decompressing and unpacking on the fly...");
  let workspace_dir = config.workspace_dir().to_string();
  tokio::task::spawn_blocking(move || match format {
    ArchiveFormat::Gzip => {
      let decoder = GzDecoder::new(sync_reader);
      Archive::new(BufReader::new(decoder)).unpack(workspace_dir)
    }
    ArchiveFormat::Zstd => {
      let decoder = zstd::Decoder::new(sync_reader)?;
      Archive::new(BufReader::new(decoder)).unpack(workspace_dir)
    }
  })
  .await??;

  info!("Workspace created successfully");
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::env;

  use serial_test::serial;
  use tempfile::TempDir;

  use super::*;

  #[tokio::test]
  #[serial]
  async fn test_empty_get_url_skips_download() {
    let workspace = TempDir::new().unwrap();
    unsafe {
      env::set_var("TUBE__WORKSPACE__DIR", workspace.path().to_str().unwrap());
    }
    let config = TubeConfig::load().unwrap();
    assert!(create_workspace(&config).await.is_ok());
  }

  #[test]
  fn test_error_variant_display() {
    let io_err = std::io::Error::new(std::io::ErrorKind::Other, "boom");
    let err = WorksapceError::Io(io_err);
    assert!(
      err
        .to_string()
        .contains("Archive or Decompression IO failed")
    );
  }

  #[test]
  fn test_detect_format_gzip() {
    assert!(matches!(
      detect_format("https://s3.example.com/repo.tar.gz"),
      ArchiveFormat::Gzip
    ));
    assert!(matches!(
      detect_format("https://s3.example.com/repo.tgz"),
      ArchiveFormat::Gzip
    ));
    assert!(matches!(
      detect_format("https://s3.example.com/archive"),
      ArchiveFormat::Gzip
    ));
  }

  #[test]
  fn test_detect_format_zstd() {
    assert!(matches!(
      detect_format("https://s3.example.com/repo.tar.zst"),
      ArchiveFormat::Zstd
    ));
    assert!(matches!(
      detect_format("https://s3.example.com/repo.zst"),
      ArchiveFormat::Zstd
    ));
  }
}
