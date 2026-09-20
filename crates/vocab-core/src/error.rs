use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabError(pub String);

impl VocabError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for VocabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VocabError {}

impl From<String> for VocabError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for VocabError {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<rusqlite::Error> for VocabError {
    fn from(value: rusqlite::Error) -> Self {
        Self(format!("sqlite error: {value}"))
    }
}

pub type Result<T> = std::result::Result<T, VocabError>;
