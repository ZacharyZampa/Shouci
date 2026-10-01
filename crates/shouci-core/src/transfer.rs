//! Importing and exporting through connectors.

use std::path::{Path, PathBuf};

use vocab_core::Result;
use vocab_exchange::{
    ExportPlan, ExportRequest, ImportPlan, ImportPolicy, TransferSummary, content_hash,
};

use crate::dto::ConnectorView;
use crate::{Error, Shouci, expand_tilde};

/// `~` expanded and made absolute, so the same file is always the same
/// path: the "never overwrite an import source" check depends on it.
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = expand_tilde(path)?;
    if let Ok(real) = std::fs::canonicalize(&path) {
        return Ok(real);
    }
    // Not created yet: resolve the directory, keep the name.
    let name = path
        .file_name()
        .ok_or_else(|| Error::invalid(format!("{} is not a file path", path.display())))?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = match parent {
        Some(parent) => std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()),
        None => std::env::current_dir()
            .map_err(|err| Error::io(format!("cannot find the current directory: {err}")))?,
    };
    Ok(dir.join(name))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|err| Error::io(format!("cannot read {}: {err}", path.display())))
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Shouci {
    /// Formats this build can import and export.
    #[must_use]
    pub fn connectors(&self) -> Vec<ConnectorView> {
        self.connectors.views()
    }

    /// Reads a file and plans the import without writing anything. Words are
    /// checked against the loaded dictionaries; without them, words are taken
    /// as written and incomplete ones need review.
    ///
    /// A file with error lines gives a refused plan unless `force` is set
    /// (then those lines are skipped).
    ///
    /// # Errors
    ///
    /// Unknown connector, unreadable file, wrong format, storage errors.
    pub fn preview_import(
        &self,
        path: &Path,
        connector: &str,
        policy: ImportPolicy,
        force: bool,
    ) -> Result<ImportPlan> {
        let connector = self.connectors.get(connector)?;
        let path = absolute(path)?;
        let bytes = read(&path)?;
        let loaded = self.loaded_opt();
        vocab_exchange::plan_import(
            &*self.db()?,
            loaded.as_deref().map(crate::dictionaries::Loaded::provider),
            connector,
            &bytes,
            &text(&path),
            policy,
            force,
        )
    }

    /// Carries out a previewed import, all or nothing.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] when the file or an affected word
    /// changed since the preview (preview again); I/O or storage errors.
    pub fn apply_import(&self, plan: &ImportPlan) -> Result<TransferSummary> {
        let bytes = read(Path::new(&plan.path))?;
        vocab_exchange::apply_import(&mut *self.db()?, plan, &content_hash(&bytes))
    }

    /// Picks words and renders the file in memory, without writing it.
    ///
    /// # Errors
    ///
    /// Unknown connector; [`crate::ErrorKind::Invalid`] for the trash or a
    /// path that was imported from; storage errors.
    pub fn preview_export(
        &self,
        path: &Path,
        connector: &str,
        request: &ExportRequest,
    ) -> Result<ExportPlan> {
        let connector = self.connectors.get(connector)?;
        let path = absolute(path)?;
        let loaded = self.loaded_opt();
        vocab_exchange::plan_export(
            &*self.db()?,
            loaded.as_deref().map(crate::dictionaries::Loaded::provider),
            connector,
            &text(&path),
            request,
        )
    }

    /// Writes a previewed export and records which words went where.
    ///
    /// # Errors
    ///
    /// I/O or storage errors.
    pub fn apply_export(&self, plan: &ExportPlan) -> Result<TransferSummary> {
        vocab_exchange::apply_export(&mut *self.db()?, plan)
    }
}
