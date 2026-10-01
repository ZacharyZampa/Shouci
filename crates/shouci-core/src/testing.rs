//! Throwaway libraries for tests: a temporary directory with a small
//! fixture dictionary, removed when dropped. Enabled by the `test-support`
//! feature so frontend crates can test against a real core.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use vocab_core::Result;
use vocab_dictionary::catalog::dictionary_path;
use vocab_dictionary::{CedictSource, build_dictionary_db};

use crate::{Config, Shouci};

/// A CC-CEDICT-format sample: 你好, 学校, 猫, 旅行, 米饭, and a few more.
pub const SAMPLE_DICTIONARY: &str = include_str!("../../../fixtures/dictionary/cedict-sample.u8");

/// A library in a temporary directory, deleted on drop.
pub struct Sandbox {
    pub dir: PathBuf,
    pub shouci: Shouci,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl std::ops::Deref for Sandbox {
    type Target = Shouci;

    fn deref(&self) -> &Shouci {
        &self.shouci
    }
}

/// A fresh, empty directory under the system temp directory.
///
/// # Panics
///
/// When the directory cannot be created.
#[must_use]
pub fn scratch_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "shouci-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Builds dictionary `id` from CC-CEDICT-format `text` into `dir`.
///
/// # Errors
///
/// When the text cannot be parsed or the file cannot be written.
pub fn install_dictionary(dir: &Path, id: &str, text: &str) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|err| crate::Error::io(err.to_string()))?;
    let path = dictionary_path(dir, id);
    let _ = std::fs::remove_file(&path);
    let mut conn = rusqlite::Connection::open(&path)?;
    build_dictionary_db(&mut conn, &CedictSource::new(id, "test"), text.as_bytes())?;
    Ok(())
}

/// A library with the sample dictionary installed as `cc-cedict` and
/// loaded. Nothing is downloaded.
///
/// # Panics
///
/// When the sandbox cannot be set up.
#[must_use]
pub fn sandbox() -> Sandbox {
    let sandbox = empty_sandbox();
    install_dictionary(
        &sandbox.config().dictionaries_dir,
        "cc-cedict",
        SAMPLE_DICTIONARY,
    )
    .expect("install sample dictionary");
    sandbox.load_dictionaries().expect("load sample dictionary");
    sandbox
}

/// A library with no dictionary, not loaded.
///
/// # Panics
///
/// When the sandbox cannot be set up.
#[must_use]
pub fn empty_sandbox() -> Sandbox {
    let dir = scratch_dir("sandbox");
    let mut config = Config::in_dir(&dir);
    config.fetch_dictionaries = false;
    Sandbox {
        shouci: Shouci::open(config).expect("open sandbox"),
        dir,
    }
}
