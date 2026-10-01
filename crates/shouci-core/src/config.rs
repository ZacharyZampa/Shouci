//! Where Shouci keeps its files.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use vocab_core::{Result, VocabError};

/// Where data lives and how dictionaries are obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Holds `user.db`.
    pub data_dir: PathBuf,
    /// Holds one `<id>.db` per dictionary.
    pub dictionaries_dir: PathBuf,
    /// Download and build built-in dictionaries that are missing, and
    /// refresh them monthly. Needs the network on first run.
    pub fetch_dictionaries: bool,
    /// A proof-of-concept data directory to bring words over from when
    /// `user.db` is created.
    pub legacy_dir: Option<PathBuf>,
}

impl Config {
    /// Everything under `dir`: `dir/user.db`, `dir/dictionaries/`.
    #[must_use]
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        let data_dir = dir.into();
        Self {
            dictionaries_dir: data_dir.join("dictionaries"),
            data_dir,
            fetch_dictionaries: true,
            legacy_dir: None,
        }
    }

    /// The standard locations, overridable by environment:
    ///
    /// - `SHOUCI_HOME`: the data directory. Otherwise
    ///   `~/Library/Application Support/Shouci` on macOS, and
    ///   `$XDG_DATA_HOME/shouci` (or `~/.local/share/shouci`) elsewhere.
    /// - `SHOUCI_DICTIONARIES`: the dictionaries directory, otherwise
    ///   `dictionaries/` inside the data directory.
    ///
    /// With no `SHOUCI_HOME`, the proof of concept's directory is checked
    /// for words to bring over.
    #[must_use]
    ///
    /// `~` and relative paths in the variables are resolved (against the
    /// home and current directories).
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let home = var("HOME");
        let explicit = var("SHOUCI_HOME");
        let data_dir = explicit.clone().map_or_else(
            || {
                platform_dir(
                    home.as_deref(),
                    var("XDG_DATA_HOME").as_deref(),
                    "Shouci",
                    "shouci",
                )
            },
            |dir| resolve(&dir, home.as_deref()),
        );
        let mut config = Self::in_dir(data_dir);
        if let Some(dir) = var("SHOUCI_DICTIONARIES") {
            config.dictionaries_dir = resolve(&dir, home.as_deref());
        }
        if explicit.is_none() {
            config.legacy_dir = Some(platform_dir(
                home.as_deref(),
                var("XDG_DATA_HOME").as_deref(),
                "pleco-companion",
                "pleco-companion",
            ));
        }
        config
    }

    #[must_use]
    pub fn user_db_path(&self) -> PathBuf {
        self.data_dir.join("user.db")
    }
}

/// A directory from the environment: `~` expanded, relative made absolute.
fn resolve(dir: &str, home: Option<&str>) -> PathBuf {
    let path =
        expand_tilde_in(Path::new(dir), home.map(Path::new)).unwrap_or_else(|_| PathBuf::from(dir));
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_or(path.clone(), |cwd| cwd.join(&path))
    }
}

fn platform_dir(home: Option<&str>, xdg: Option<&str>, mac_name: &str, unix_name: &str) -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(home) = home {
            return PathBuf::from(home)
                .join("Library/Application Support")
                .join(mac_name);
        }
    }
    if let Some(xdg) = xdg {
        return PathBuf::from(xdg).join(unix_name);
    }
    match home {
        Some(home) => PathBuf::from(home).join(".local/share").join(unix_name),
        None => PathBuf::from(unix_name),
    }
}

/// Expands a leading `~` to `home`. Only `~` and `~/…`: `~user`, `$VAR`, and
/// quotes are left alone, so a path meant literally reaches the filesystem
/// unchanged.
///
/// # Errors
///
/// When the path starts with `~` and there is no home directory.
pub fn expand_tilde_in(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let mut components = path.components();
    if components.next() != Some(Component::Normal(OsStr::new("~"))) {
        return Ok(path.to_path_buf());
    }
    let home = home.ok_or_else(|| VocabError::invalid("cannot expand ~: HOME is not set"))?;
    let rest: PathBuf = components.collect();
    Ok(if rest.as_os_str().is_empty() {
        home.to_path_buf()
    } else {
        home.join(rest)
    })
}

/// [`expand_tilde_in`] with `$HOME`.
///
/// # Errors
///
/// When the path starts with `~` and `HOME` is not set.
pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    expand_tilde_in(path, home.as_deref())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Config, expand_tilde_in, platform_dir};

    fn expand(input: &str) -> std::path::PathBuf {
        expand_tilde_in(Path::new(input), Some(Path::new("/Users/tester"))).unwrap()
    }

    #[test]
    fn tilde_forms() {
        assert_eq!(expand("~"), Path::new("/Users/tester"));
        assert_eq!(
            expand("~/Desktop/a.txt"),
            Path::new("/Users/tester/Desktop/a.txt")
        );
        for literal in ["~someone/a.txt", "$HOME/a.txt", "/tmp/a.txt", "a.txt", ""] {
            assert_eq!(expand(literal), Path::new(literal));
        }
        assert!(expand_tilde_in(Path::new("~/a"), None).is_err());
        assert!(expand_tilde_in(Path::new("a"), None).is_ok());
    }

    #[test]
    fn data_lives_together() {
        let config = Config::in_dir("/data");
        assert_eq!(config.user_db_path(), Path::new("/data/user.db"));
        assert_eq!(config.dictionaries_dir, Path::new("/data/dictionaries"));
    }

    #[test]
    fn environment_paths_are_resolved() {
        assert_eq!(
            super::resolve("~/shouci", Some("/Users/me")),
            Path::new("/Users/me/shouci")
        );
        assert!(super::resolve("relative/dir", None).is_absolute());
    }

    #[test]
    fn platform_directories() {
        let dir = platform_dir(Some("/Users/me"), None, "Shouci", "shouci");
        if cfg!(target_os = "macos") {
            assert_eq!(
                dir,
                Path::new("/Users/me/Library/Application Support/Shouci")
            );
        } else {
            assert_eq!(dir, Path::new("/Users/me/.local/share/shouci"));
        }
    }
}
