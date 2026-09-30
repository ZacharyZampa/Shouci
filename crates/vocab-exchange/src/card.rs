#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Field<T> {
    Present(T),
    Omitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExchangeCard {
    pub line: usize,
    pub raw: String,
    pub headword: String,
    pub traditional: Option<String>,
    pub pinyin: Field<String>,
    pub definition: Field<String>,
    pub notes: Field<String>,
    pub tags: Vec<String>,
}
