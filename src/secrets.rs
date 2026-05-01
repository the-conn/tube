use std::{borrow::Cow, fs, io, path::Path};

use thiserror::Error;
use tracing::{info, warn};

const MASK: &str = "***";

#[derive(Error, Debug)]
pub enum SecretsError {
  #[error("Failed to read secrets directory: {0}")]
  Io(#[from] io::Error),
}

#[derive(Debug, Default, Clone)]
pub struct Secrets {
  entries: Vec<(String, String)>,
}

impl Secrets {
  pub fn load(dir: &Path) -> Result<Self, SecretsError> {
    let read_dir = match fs::read_dir(dir) {
      Ok(rd) => rd,
      Err(e) if e.kind() == io::ErrorKind::NotFound => {
        info!(
          "No secrets directory at {}, continuing without secrets",
          dir.display()
        );
        return Ok(Self::default());
      }
      Err(e) => return Err(SecretsError::Io(e)),
    };

    let mut entries = Vec::new();
    for entry in read_dir {
      let entry = entry?;
      let name = entry.file_name();
      let Some(name_str) = name.to_str() else {
        continue;
      };
      // Skip kubernetes secret-mount internals (..data, ..2024_xx_xx_xx)
      // and any other dotfile.
      if name_str.starts_with('.') {
        continue;
      }

      let path = entry.path();
      let meta = fs::metadata(&path)?;
      if !meta.is_file() {
        continue;
      }

      let value = fs::read_to_string(&path)?;
      let trimmed = value.trim();
      if trimmed.is_empty() {
        warn!("Skipping empty secret: {}", name_str);
        continue;
      }
      entries.push((name_str.to_string(), trimmed.to_string()));
    }

    // Mask longer values first so a shorter secret that is a substring
    // of a longer one cannot leave fragments behind.
    entries.sort_by_key(|e| std::cmp::Reverse(e.1.len()));

    info!("Loaded {} secret(s) from {}", entries.len(), dir.display());
    Ok(Self { entries })
  }

  pub fn env(&self) -> impl Iterator<Item = (&str, &str)> {
    self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn mask<'a>(&self, text: &'a str) -> Cow<'a, str> {
    let mut current: Cow<'a, str> = Cow::Borrowed(text);
    for (_, value) in &self.entries {
      if current.contains(value.as_str()) {
        current = Cow::Owned(current.replace(value.as_str(), MASK));
      }
    }
    current
  }
}

#[cfg(test)]
mod tests {
  use std::{fs::File, io::Write};

  use tempfile::TempDir;

  use super::*;

  fn write_secret(dir: &Path, name: &str, value: &str) {
    let mut f = File::create(dir.join(name)).unwrap();
    f.write_all(value.as_bytes()).unwrap();
  }

  #[test]
  fn missing_directory_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does-not-exist");
    let secrets = Secrets::load(&missing).unwrap();
    assert!(secrets.is_empty());
  }

  #[test]
  fn loads_files_and_trims() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "QUAY_USERNAME", "alice\n");
    write_secret(tmp.path(), "QUAY_PASSWORD", "  s3cret  \n");
    let secrets = Secrets::load(tmp.path()).unwrap();
    assert!(!secrets.is_empty());

    let masked = secrets
      .mask("alice logging in with s3cret token")
      .into_owned();
    assert_eq!(masked, "*** logging in with *** token");
  }

  #[test]
  fn skips_dotfiles_used_by_kubernetes_mounts() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "QUAY_PASSWORD", "real_value");
    write_secret(tmp.path(), "..data", "kubernetes-internal-marker");
    let secrets = Secrets::load(tmp.path()).unwrap();
    assert_eq!(secrets.env().count(), 1);
    let masked = secrets
      .mask("password=real_value, marker=kubernetes-internal-marker")
      .into_owned();
    assert!(masked.contains("password=***"));
    assert!(masked.contains("marker=kubernetes-internal-marker"));
  }

  #[test]
  fn skips_subdirectories() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "TOKEN", "tok");
    fs::create_dir(tmp.path().join("nested")).unwrap();
    let secrets = Secrets::load(tmp.path()).unwrap();
    assert_eq!(secrets.env().count(), 1);
  }

  #[test]
  fn skips_empty_files() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "BLANK", "   \n");
    write_secret(tmp.path(), "TOKEN", "abc");
    let secrets = Secrets::load(tmp.path()).unwrap();
    let names: Vec<&str> = secrets.env().map(|(k, _)| k).collect();
    assert_eq!(names, vec!["TOKEN"]);
  }

  #[test]
  fn mask_returns_borrowed_when_no_match() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "TOKEN", "abc");
    let secrets = Secrets::load(tmp.path()).unwrap();
    let line = "no secrets here";
    match secrets.mask(line) {
      Cow::Borrowed(s) => assert_eq!(s, line),
      Cow::Owned(_) => panic!("expected borrowed return when no match"),
    }
  }

  #[test]
  fn mask_handles_overlapping_secrets() {
    let tmp = TempDir::new().unwrap();
    // The shorter secret is a substring of the longer one; sorting by
    // length-desc ensures the longer secret is replaced first.
    write_secret(tmp.path(), "SHORT", "abcd");
    write_secret(tmp.path(), "LONG", "abcdefgh");
    let secrets = Secrets::load(tmp.path()).unwrap();
    let masked = secrets.mask("value=abcdefgh, fragment=abcd").into_owned();
    assert_eq!(masked, "value=***, fragment=***");
  }

  #[test]
  fn empty_secrets_mask_is_passthrough() {
    let secrets = Secrets::default();
    let line = "anything goes";
    match secrets.mask(line) {
      Cow::Borrowed(s) => assert_eq!(s, line),
      Cow::Owned(_) => panic!("default Secrets should not allocate"),
    }
  }

  #[test]
  fn env_iterator_yields_loaded_pairs() {
    let tmp = TempDir::new().unwrap();
    write_secret(tmp.path(), "QUAY_USERNAME", "alice");
    let secrets = Secrets::load(tmp.path()).unwrap();
    let pairs: Vec<(&str, &str)> = secrets.env().collect();
    assert_eq!(pairs, vec![("QUAY_USERNAME", "alice")]);
  }
}
