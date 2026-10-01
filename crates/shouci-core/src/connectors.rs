//! The connectors this build knows.
//!
//! To add a format: create a crate that depends on `vocab-core` and
//! implements [`Connector`], add it as an optional dependency and feature in
//! this crate's `Cargo.toml`, and register it in [`Registry::builtin`].

use vocab_core::connector::Connector;
use vocab_core::{Result, VocabError};

use crate::dto::ConnectorView;

pub struct Registry {
    connectors: Vec<Box<dyn Connector>>,
}

impl Registry {
    /// Every connector compiled into this build.
    #[must_use]
    // One push per feature; `vec![]` cannot take `#[cfg]` items.
    #[allow(unused_mut, clippy::vec_init_then_push)]
    pub fn builtin() -> Self {
        let mut connectors: Vec<Box<dyn Connector>> = Vec::new();
        #[cfg(feature = "pleco")]
        connectors.push(Box::new(vocab_pleco::Pleco));
        #[cfg(feature = "anki")]
        connectors.push(Box::new(vocab_anki::Anki));
        Self { connectors }
    }

    /// # Errors
    ///
    /// [`vocab_core::ErrorKind::NotFound`] naming the known connectors.
    pub fn get(&self, id: &str) -> Result<&dyn Connector> {
        self.connectors
            .iter()
            .find(|connector| connector.info().id.eq_ignore_ascii_case(id))
            .map(Box::as_ref)
            .ok_or_else(|| {
                let known: Vec<&str> = self.connectors.iter().map(|c| c.info().id).collect();
                VocabError::not_found(format!(
                    "no connector '{id}' (this build has: {})",
                    known.join(", ")
                ))
            })
    }

    #[must_use]
    pub fn views(&self) -> Vec<ConnectorView> {
        self.connectors
            .iter()
            .map(|connector| {
                let info = connector.info();
                ConnectorView {
                    id: info.id.to_owned(),
                    name: info.name.to_owned(),
                    format: info.format.to_owned(),
                    extensions: info.extensions.iter().map(|e| (*e).to_owned()).collect(),
                    can_import: info.can_import,
                    can_export: info.can_export,
                }
            })
            .collect()
    }
}
