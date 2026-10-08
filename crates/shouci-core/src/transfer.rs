//! Importing and exporting through connectors.

use std::path::{Path, PathBuf};

use vocab_core::Result;
use vocab_exchange::{
    ExportPlan, ExportRequest, ImportPlan, ImportPolicy, TransferSummary, content_hash,
};

use crate::dto::{ConnectorView, DetectedImport};
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

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            Error::not_found(format!("no file at {}", path.display()))
        } else {
            Error::io(format!("cannot read {}: {err}", path.display()))
        }
    })
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

    /// A format's name for people (`Pleco` for `pleco`), or the id itself
    /// when this build has no such format.
    #[must_use]
    pub fn connector_name(&self, id: &str) -> String {
        self.connectors.get(id).map_or_else(
            |_| id.to_owned(),
            |connector| connector.info().name.to_owned(),
        )
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
        let bytes = read_file(&path)?;
        let loaded = self.loaded_opt();
        vocab_exchange::plan_import(
            &*self.reader()?,
            loaded.as_deref().map(crate::dictionaries::Loaded::provider),
            connector,
            &bytes,
            &text(&path),
            policy,
            force,
        )
    }

    /// [`Shouci::preview_import`] with whichever connector reads the file
    /// best: the fewest error lines, then the fewest words it could not
    /// resolve or had to drop. A tie goes to the connector listed first.
    /// The plan's `connector_id` says which one it was, and `unambiguous`
    /// whether any other read the file as well.
    ///
    /// # Errors
    ///
    /// When no connector can read the file, the first one's reason.
    pub fn detect_import(
        &self,
        path: &Path,
        policy: ImportPolicy,
        force: bool,
    ) -> Result<DetectedImport> {
        let mut best: Option<((u32, u32), ImportPlan)> = None;
        let mut unambiguous = true;
        let mut first_error = None;
        for connector in self.connectors().into_iter().filter(|c| c.can_import) {
            match self.preview_import(path, &connector.id, policy, force) {
                Ok(plan) => {
                    let counts = plan.counts();
                    let misfit = (counts.errors, counts.unresolved + counts.drops);
                    match &best {
                        Some((fit, _)) if misfit > *fit => {}
                        Some((fit, _)) if misfit == *fit => unambiguous = false,
                        _ => {
                            unambiguous = true;
                            best = Some((misfit, plan));
                        }
                    }
                }
                Err(err) => {
                    first_error.get_or_insert(err);
                }
            }
        }
        match (best, first_error) {
            (Some((_, plan)), _) => Ok(DetectedImport { plan, unambiguous }),
            (None, Some(err)) => Err(err),
            (None, None) => Err(Error::unavailable("this build has no format to import")),
        }
    }

    /// Carries out a previewed import, all or nothing.
    ///
    /// # Errors
    ///
    /// [`crate::ErrorKind::Conflict`] when the file or an affected word
    /// changed since the preview (preview again); I/O or storage errors.
    pub fn apply_import(&self, plan: &ImportPlan) -> Result<TransferSummary> {
        let bytes = read_file(Path::new(&plan.path))?;
        vocab_exchange::apply_import(&mut *self.writer()?, plan, &content_hash(&bytes))
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
        let ranks = loaded.as_deref().map(crate::dictionaries::Loaded::ranks);
        vocab_exchange::plan_export_where(
            &*self.reader()?,
            loaded.as_deref().map(crate::dictionaries::Loaded::provider),
            connector,
            &text(&path),
            request,
            &|item| crate::library::admits(&request.filter, ranks, &item.simplified),
        )
    }

    /// Writes a previewed export and records which words went where.
    ///
    /// # Errors
    ///
    /// I/O or storage errors.
    pub fn apply_export(&self, plan: &ExportPlan) -> Result<TransferSummary> {
        vocab_exchange::apply_export(&mut *self.writer()?, plan)
    }
}
