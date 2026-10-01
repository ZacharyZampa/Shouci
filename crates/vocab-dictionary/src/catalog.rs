//! Which dictionaries exist: one read-only SQLite file per dictionary,
//! `<dir>/<id>.db`, each describing itself in its metadata table.
//!
//! One file per dictionary keeps every dictionary independent: it is built,
//! refreshed, and removed on its own, and the fetch lock and atomic replace
//! in [`crate::ensure_dictionary_db`] work unchanged.

use std::path::{Path, PathBuf};

use vocab_core::{Result, VocabError};

use crate::schema::{
    METADATA_ENTRY_COUNT, METADATA_LICENSE, METADATA_SOURCE_ID, METADATA_SOURCE_VERSION,
};
use crate::{Ensured, FetchStage, SqliteDictionary};

/// A dictionary Shouci knows how to fetch and build.
#[derive(Debug, Clone, Copy)]
pub struct DictionarySpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Builds the database at the given path when missing, and refreshes it
    /// when due, reporting each stage. Needs the network on first run.
    pub ensure: fn(&Path, &mut dyn FnMut(FetchStage)) -> Result<Ensured>,
}

/// CC-CEDICT with `OpenSubtitles` frequency and HSK 3.0 ranks.
pub const CC_CEDICT: DictionarySpec = DictionarySpec {
    id: "cc-cedict",
    name: "CC-CEDICT",
    ensure: crate::ensure_dictionary_db,
};

/// Dictionaries that ship with Shouci, fetched on first use.
#[must_use]
pub fn builtin() -> &'static [DictionarySpec] {
    &[CC_CEDICT]
}

/// Where dictionary `id` lives inside `dir`.
#[must_use]
pub fn dictionary_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.db"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryInfo {
    /// The `source_id` every entry carries: `cc-cedict`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    pub entry_count: u64,
    /// `None` for an in-memory build.
    pub path: Option<PathBuf>,
}

impl SqliteDictionary {
    /// Reads this dictionary's description from its metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata cannot be read or has no source id.
    pub fn info(&self, path: Option<&Path>) -> Result<DictionaryInfo> {
        let metadata = self.metadata()?;
        let value = |key: &str| {
            metadata
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        let id = value(METADATA_SOURCE_ID).ok_or_else(|| {
            VocabError::format("dictionary has no source id; rebuild it".to_owned())
        })?;
        let name = builtin()
            .iter()
            .find(|spec| spec.id == id)
            .map_or_else(|| id.clone(), |spec| spec.name.to_owned());
        Ok(DictionaryInfo {
            name,
            version: value(METADATA_SOURCE_VERSION).unwrap_or_default(),
            license: value(METADATA_LICENSE).unwrap_or_default(),
            entry_count: value(METADATA_ENTRY_COUNT)
                .and_then(|count| count.parse().ok())
                .unwrap_or(0),
            path: path.map(Path::to_path_buf),
            id,
        })
    }
}

/// What [`installed`] found.
#[derive(Debug, Default)]
pub struct Installed {
    pub dictionaries: Vec<DictionaryInfo>,
    /// Files that look like dictionaries but could not be opened, with why.
    pub problems: Vec<String>,
}

/// Every built dictionary in `dir`, ordered by id. A missing directory has
/// none. Half-built files (`*.building.db`) are ignored.
///
/// # Errors
///
/// Returns an error if `dir` exists but cannot be listed.
pub fn installed(dir: &Path) -> Result<Installed> {
    let mut found = Installed::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(err) => {
            return Err(VocabError::io(format!(
                "cannot list dictionaries in {}: {err}",
                dir.display()
            )));
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "db")
                && !path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().ends_with(".building"))
        })
        .collect();
    paths.sort();
    for path in paths {
        match SqliteDictionary::open(&path).and_then(|dict| dict.info(Some(&path))) {
            Ok(info) => found.dictionaries.push(info),
            Err(err) => found.problems.push(format!("{}: {err}", path.display())),
        }
    }
    found.dictionaries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{dictionary_path, installed};
    use crate::{CedictSource, build_dictionary_db};

    fn sample() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/dictionary/cedict-sample.u8"),
        )
        .expect("fixture")
    }

    #[test]
    fn lists_built_dictionaries_and_skips_partial_builds() {
        let dir = std::env::temp_dir().join(format!("shouci-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dictionary_path(&dir, "cc-cedict");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        build_dictionary_db(&mut conn, &CedictSource::default(), &sample()).unwrap();
        drop(conn);
        std::fs::write(dir.join("cc-cedict.building.db"), b"partial").unwrap();
        std::fs::write(dir.join("broken.db"), b"not sqlite").unwrap();

        let found = installed(&dir).unwrap();
        assert_eq!(found.dictionaries.len(), 1);
        let info = &found.dictionaries[0];
        assert_eq!(info.id, "cc-cedict");
        assert_eq!(info.name, "CC-CEDICT");
        assert!(info.entry_count > 10);
        assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
        assert!(found.problems[0].contains("broken.db"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_has_no_dictionaries() {
        let found = installed(std::path::Path::new("/nonexistent/shouci/dictionaries")).unwrap();
        assert!(found.dictionaries.is_empty());
        assert!(found.problems.is_empty());
    }
}
