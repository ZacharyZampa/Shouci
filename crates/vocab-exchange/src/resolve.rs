use vocab_core::{DictionaryEntry, ItemStatus};

use crate::card::{ExchangeCard, Field};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DictHit {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub glosses: Vec<String>,
    pub entry_id: Option<i64>,
    pub source_id: String,
    pub source_version: String,
}

impl DictHit {
    pub(crate) fn from_entry(entry: &DictionaryEntry) -> Self {
        Self {
            simplified: entry.simplified.clone(),
            traditional: entry.traditional.clone(),
            pinyin: entry.pinyin.clone(),
            glosses: entry.glosses.clone(),
            entry_id: entry.stable_entry_id,
            source_id: entry.provenance.source.as_str().to_owned(),
            source_version: entry.provenance.source_version.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedCard {
    pub simplified: String,
    pub traditional: String,
    pub pinyin: String,
    pub definition: String,
    pub notes: Option<String>,
    pub tags: Vec<String>,
    pub status: ItemStatus,
    pub source_entry_id: Option<i64>,
    pub source_id: String,
    pub source_version: String,
    pub line: usize,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    Drop {
        line: usize,
        raw: String,
        message: String,
    },
    Card(ResolvedCard),
}

pub(crate) fn resolve(card: &ExchangeCard, hits: &[DictHit], app: &str) -> Resolution {
    let headword = card.headword.trim();
    if headword.is_empty() {
        return Resolution::Drop {
            line: card.line,
            raw: card.raw.clone(),
            message: "record has no headword".to_owned(),
        };
    }
    let exact: Vec<&DictHit> = hits
        .iter()
        .filter(|hit| hit.simplified == headword || hit.traditional == headword)
        .collect();
    let pinyin = present_text(&card.pinyin);
    let narrowed: Vec<&DictHit> = match pinyin {
        Some(reading) if !reading.is_empty() => {
            let key = reading_key(reading);
            exact
                .into_iter()
                .filter(|hit| reading_key(&hit.pinyin) == key)
                .collect()
        }
        _ => exact,
    };
    if let Some(hit) = single(&narrowed) {
        if traditional_disagrees(card.traditional.as_deref(), hit) {
            return Resolution::Card(unresolved(card, app, headword));
        }
        return Resolution::Card(from_hit(card, hit, app));
    }
    if narrowed.is_empty() && has_full_user_card(card) {
        return Resolution::Card(user_card(card, app, headword));
    }
    Resolution::Card(unresolved(card, app, headword))
}

pub(crate) fn reading_key(pinyin: &str) -> String {
    let trimmed = pinyin.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        vocab_pinyin::normalize(trimmed).as_str().to_owned()
    }
}

pub(crate) fn definition_matches(stored: &str, glosses: &[String]) -> bool {
    let stored = stored.trim();
    if stored.is_empty() {
        return false;
    }
    let joined = glosses.join("; ");
    stored == joined.trim() || glosses.iter().any(|gloss| gloss.trim() == stored)
}

fn from_hit(card: &ExchangeCard, hit: &DictHit, app: &str) -> ResolvedCard {
    let inbound = present_text(&card.definition).unwrap_or("");
    let (definition, source_id, source_version) =
        if inbound.is_empty() || definition_matches(inbound, &hit.glosses) {
            (
                hit.glosses.join("; "),
                hit.source_id.clone(),
                hit.source_version.clone(),
            )
        } else {
            (inbound.to_owned(), app.to_owned(), "import".to_owned())
        };
    ResolvedCard {
        simplified: hit.simplified.clone(),
        traditional: hit.traditional.clone(),
        pinyin: hit.pinyin.clone(),
        definition,
        notes: notes_of(card),
        tags: card.tags.clone(),
        status: ItemStatus::Confirmed,
        source_entry_id: hit.entry_id,
        source_id,
        source_version,
        line: card.line,
        raw: card.raw.clone(),
    }
}

fn user_card(card: &ExchangeCard, app: &str, headword: &str) -> ResolvedCard {
    let traditional = card
        .traditional
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(headword);
    ResolvedCard {
        simplified: headword.to_owned(),
        traditional: traditional.to_owned(),
        pinyin: present_text(&card.pinyin).unwrap_or("").to_owned(),
        definition: present_text(&card.definition).unwrap_or("").to_owned(),
        notes: notes_of(card),
        tags: card.tags.clone(),
        status: ItemStatus::Confirmed,
        source_entry_id: None,
        source_id: app.to_owned(),
        source_version: "import".to_owned(),
        line: card.line,
        raw: card.raw.clone(),
    }
}

fn unresolved(card: &ExchangeCard, app: &str, headword: &str) -> ResolvedCard {
    let mut card = user_card(card, app, headword);
    card.status = ItemStatus::NeedsReview;
    card.source_entry_id = None;
    if card.definition.is_empty() {
        card.definition = format!("unresolved import: {headword}");
    }
    card
}

fn has_full_user_card(card: &ExchangeCard) -> bool {
    matches!(
        (
            present_text(&card.pinyin).filter(|value| !value.is_empty()),
            present_text(&card.definition).filter(|value| !value.is_empty()),
        ),
        (Some(_), Some(_))
    )
}

fn traditional_disagrees(file_traditional: Option<&str>, hit: &DictHit) -> bool {
    match file_traditional
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(traditional) => traditional != hit.traditional,
        None => false,
    }
}

fn single<'a>(hits: &[&'a DictHit]) -> Option<&'a DictHit> {
    if hits.len() == 1 { Some(hits[0]) } else { None }
}

fn present_text(field: &Field<String>) -> Option<&str> {
    match field {
        Field::Present(value) => Some(value.as_str()),
        Field::Omitted => None,
    }
}

fn notes_of(card: &ExchangeCard) -> Option<String> {
    match &card.notes {
        Field::Present(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        Field::Present(_) | Field::Omitted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{DictHit, Resolution, definition_matches, resolve};
    use crate::card::{ExchangeCard, Field};
    use vocab_core::ItemStatus;

    fn hit(pinyin: &str, gloss: &str) -> DictHit {
        DictHit {
            simplified: "行".to_owned(),
            traditional: "行".to_owned(),
            pinyin: pinyin.to_owned(),
            glosses: vec![gloss.to_owned()],
            entry_id: Some(1),
            source_id: "cc-cedict".to_owned(),
            source_version: "1".to_owned(),
        }
    }

    fn card(pinyin: Field<String>, definition: Field<String>) -> ExchangeCard {
        ExchangeCard {
            line: 1,
            raw: "行".to_owned(),
            headword: "行".to_owned(),
            traditional: None,
            pinyin,
            definition,
            notes: Field::Omitted,
            tags: Vec::new(),
        }
    }

    #[test]
    fn unique_hit_fills_an_omitted_definition() {
        let resolved = resolve(
            &card(Field::Present("xing2".to_owned()), Field::Omitted),
            &[hit("xing2", "to walk")],
            "pleco",
        );
        let Resolution::Card(card) = resolved else {
            panic!("expected a card");
        };
        assert_eq!(card.status, ItemStatus::Confirmed);
        assert_eq!(card.definition, "to walk");
        assert_eq!(card.source_id, "cc-cedict");
        assert_eq!(card.source_entry_id, Some(1));
    }

    #[test]
    fn two_hits_are_not_picked() {
        let mut other = hit("hang2", "firm");
        other.entry_id = Some(2);
        let resolved = resolve(
            &card(Field::Omitted, Field::Omitted),
            &[hit("xing2", "to walk"), other],
            "pleco",
        );
        let Resolution::Card(card) = resolved else {
            panic!("expected a card");
        };
        assert_eq!(card.status, ItemStatus::NeedsReview);
        assert!(card.source_entry_id.is_none());
    }

    #[test]
    fn full_user_card_with_no_hit_is_confirmed() {
        let resolved = resolve(
            &card(
                Field::Present("xing2".to_owned()),
                Field::Present("a custom gloss".to_owned()),
            ),
            &[],
            "anki",
        );
        let Resolution::Card(card) = resolved else {
            panic!("expected a card");
        };
        assert_eq!(card.status, ItemStatus::Confirmed);
        assert_eq!(card.source_id, "anki");
        assert_eq!(card.definition, "a custom gloss");
        assert!(card.source_entry_id.is_none());
    }

    #[test]
    fn differing_gloss_is_kept() {
        let resolved = resolve(
            &card(
                Field::Present("xing2".to_owned()),
                Field::Present("my gloss".to_owned()),
            ),
            &[hit("xing2", "to walk")],
            "pleco",
        );
        let Resolution::Card(card) = resolved else {
            panic!("expected a card");
        };
        assert_eq!(card.definition, "my gloss");
        assert_eq!(card.source_id, "pleco");
        assert_eq!(card.source_entry_id, Some(1));
        assert!(definition_matches("to walk", &["to walk".to_owned()]));
    }
}
