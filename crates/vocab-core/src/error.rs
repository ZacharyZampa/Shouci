use std::fmt;

/// What went wrong, for callers that react differently to different failures
/// (a UI shows "already saved" for [`ErrorKind::Conflict`] but a retry for
/// [`ErrorKind::Unavailable`]). The message is always written for people.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ErrorKind {
    /// The item, tag, collection, dictionary, or connector does not exist.
    NotFound,
    /// The change would collide with existing data, or the data changed
    /// since it was read (a stale import plan).
    Conflict,
    /// The request itself is malformed: an empty name, an unknown option.
    Invalid,
    /// A dependency is not ready: the dictionary is still loading or failed.
    Unavailable,
    /// A file could not be read as the expected format.
    Format,
    /// A file or directory could not be read or written.
    Io,
    /// The database failed.
    Storage,
    /// Anything else.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabError {
    kind: ErrorKind,
    message: String,
}

impl VocabError {
    /// An [`ErrorKind::Internal`] error.
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Internal, message)
    }

    pub fn with_kind(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Conflict, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Invalid, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Unavailable, message)
    }

    pub fn format(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Format, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Io, message)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::with_kind(ErrorKind::Storage, message)
    }

    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for VocabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VocabError {}

impl From<String> for VocabError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for VocabError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for VocabError {
    /// A busy or locked database (another process is writing and did not
    /// finish within the busy timeout) is [`ErrorKind::Unavailable`]: worth
    /// retrying. Everything else is [`ErrorKind::Storage`].
    fn from(value: rusqlite::Error) -> Self {
        let busy = matches!(
            value.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        );
        if busy {
            Self::unavailable(format!(
                "the library is busy in another Shouci window or process; try again ({value})"
            ))
        } else {
            Self::storage(format!("database error: {value}"))
        }
    }
}

pub type Result<T> = std::result::Result<T, VocabError>;

#[cfg(test)]
mod tests {
    use super::{ErrorKind, VocabError};

    #[test]
    fn kind_and_message_survive() {
        let err = VocabError::conflict("学校 is already saved");
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert_eq!(err.to_string(), "学校 is already saved");
    }

    #[test]
    fn plain_strings_are_internal() {
        assert_eq!(VocabError::from("boom").kind(), ErrorKind::Internal);
    }
}
