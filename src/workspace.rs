use std::{
  io::{BufReader, Read},
  path::{Path, PathBuf},
};

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

// Archives from the backend always wrap contents in a top-level directory named
// after the repo + commit (e.g. "the-conn-jefferies-e646b86/"). Strip that prefix
// so files land directly in the workspace rather than a subdirectory.
fn unpack_strip_root<R: Read>(archive: &mut Archive<R>, dest: &str) -> std::io::Result<()> {
  let dest = Path::new(dest);
  for entry in archive.entries()? {
    let mut entry = entry?;
    let path = entry.path()?.into_owned();
    let stripped: PathBuf = path.components().skip(1).collect();
    if stripped.as_os_str().is_empty() {
      continue;
    }
    let target = dest.join(stripped);
    if let Some(parent) = target.parent() {
      std::fs::create_dir_all(parent)?;
    }
    entry.unpack(target)?;
  }
  Ok(())
}

pub async fn create_workspace(
  config: &TubeConfig,
  client: &reqwest::Client,
) -> Result<(), WorksapceError> {
  if config.get_url().is_empty() {
    info!("No get_url provided, skipping repo cloning");
    return Ok(());
  }

  info!("Streaming repo archive from S3...");
  let format = detect_format(config.get_url());
  let response = client.get(config.get_url()).send().await?;
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
      unpack_strip_root(&mut Archive::new(BufReader::new(decoder)), &workspace_dir)
    }
    ArchiveFormat::Zstd => {
      let decoder = zstd::Decoder::new(sync_reader)?;
      unpack_strip_root(&mut Archive::new(BufReader::new(decoder)), &workspace_dir)
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
    let client = reqwest::Client::new();
    assert!(create_workspace(&config, &client).await.is_ok());
  }

  #[test]
  fn test_unpack_strip_root_flattens_top_level_dir() {
    use std::io::Cursor;

    let mut builder = tar::Builder::new(Vec::new());
    let content = b"hello";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
      .append_data(&mut header, "repo-abc123/src/main.rs", content.as_ref())
      .unwrap();
    let tar_bytes = builder.into_inner().unwrap();

    let dest = TempDir::new().unwrap();
    let mut archive = Archive::new(Cursor::new(tar_bytes));
    unpack_strip_root(&mut archive, dest.path().to_str().unwrap()).unwrap();

    assert!(dest.path().join("src/main.rs").exists());
    assert!(!dest.path().join("repo-abc123").exists());
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
