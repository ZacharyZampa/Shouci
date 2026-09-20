use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemStatus {
    Confirmed,
    NeedsReview,
    Exported,
    Archived,
}

impl ItemStatus {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs_review",
            Self::Exported => "exported",
            Self::Archived => "archived",
        }
    }
}

impl std::str::FromStr for ItemStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "confirmed" => Ok(Self::Confirmed),
            "needs_review" => Ok(Self::NeedsReview),
            "exported" => Ok(Self::Exported),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown vocabulary status: {other}")),
        }
    }
}

impl fmt::Display for ItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ItemStatus;

    #[test]
    fn round_trips_stable_string_form() {
        for status in [
            ItemStatus::Confirmed,
            ItemStatus::NeedsReview,
            ItemStatus::Exported,
            ItemStatus::Archived,
        ] {
            assert_eq!(status.as_str().parse(), Ok(status));
        }
    }

    #[test]
    fn unknown_string_is_rejected() {
        assert!("pending".parse::<ItemStatus>().is_err());
    }
}
