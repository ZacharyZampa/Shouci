//! What kind of text a query is, so nobody has to pick a search mode.

use vocab_pinyin::{normalize, segment};

/// How a query is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum QueryKind {
    English,
    Pinyin,
    Chinese,
}

impl QueryKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Pinyin => "pinyin",
            Self::Chinese => "chinese",
        }
    }
}

impl std::str::FromStr for QueryKind {
    type Err = vocab_core::VocabError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "english" => Ok(Self::English),
            "pinyin" => Ok(Self::Pinyin),
            "chinese" | "hanzi" => Ok(Self::Chinese),
            other => Err(vocab_core::VocabError::invalid(format!(
                "unknown query kind '{other}' (expected english, pinyin, or chinese)"
            ))),
        }
    }
}

/// The first guess for a query:
///
/// - Chinese characters alone → [`QueryKind::Chinese`];
/// - Chinese mixed with letters (`猫māo`), or letters with tone digits
///   (`lv3`) → [`QueryKind::Pinyin`];
/// - anything else → [`QueryKind::English`]. Untoned pinyin (`nihao`) is
///   indistinguishable from English here; searches fall through to pinyin.
#[must_use]
pub fn detect(query: &str) -> QueryKind {
    let mut has_cjk = false;
    let mut has_other_alpha = false;
    let mut has_digit = false;
    let mut has_latin = false;
    for c in query.chars() {
        if c.is_ascii_digit() {
            has_digit = true;
        } else if c.is_ascii_alphabetic() {
            has_latin = true;
        }
        if is_han(c) {
            has_cjk = true;
        } else if c.is_alphabetic() {
            has_other_alpha = true;
        }
    }
    if has_cjk {
        if has_other_alpha {
            QueryKind::Pinyin
        } else {
            QueryKind::Chinese
        }
    } else if has_digit && has_latin {
        QueryKind::Pinyin
    } else {
        QueryKind::English
    }
}

/// CJK Unified Ideographs and Extension A.
pub(crate) fn is_han(c: char) -> bool {
    let n = u32::from(c);
    (0x3400..=0x4dbf).contains(&n) || (0x4e00..=0x9fff).contains(&n)
}

/// Letters only, and they spell pinyin syllables: `wo`, `jingzi`, `nihao`,
/// `can`, but not `school` or `a`.
#[must_use]
pub fn looks_like_pinyin(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty()
        || !q
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'' || c.is_ascii_whitespace())
    {
        return false;
    }
    !segment(normalize(q).as_str()).is_empty()
}

#[cfg(test)]
mod tests {
    use super::{QueryKind, detect, looks_like_pinyin};

    #[test]
    fn detects_common_queries() {
        assert_eq!(detect("旅行"), QueryKind::Chinese);
        assert_eq!(detect("lv3"), QueryKind::Pinyin);
        assert_eq!(detect("lv3xing2"), QueryKind::Pinyin);
        assert_eq!(detect("to travel"), QueryKind::English);
        assert_eq!(detect("nihao"), QueryKind::English);
        assert_eq!(detect("猫māo"), QueryKind::Pinyin);
    }

    #[test]
    fn untoned_pinyin_is_detected() {
        for yes in ["jingzi", "nihao", "xuexiao", "wo", "shi", "can"] {
            assert!(looks_like_pinyin(yes), "{yes}");
        }
        for no in ["school", "hello", "cat", "a", ""] {
            assert!(!looks_like_pinyin(no), "{no}");
        }
    }
}
