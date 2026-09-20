use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(pub String);

impl SourceId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceVersion(pub String);

impl SourceVersion {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmationState {
    DictionaryAuthority,
    UserConfirmed,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub source: SourceId,
    pub source_version: SourceVersion,
    pub import_origin: Option<String>,
    pub confirmation: ConfirmationState,
}
