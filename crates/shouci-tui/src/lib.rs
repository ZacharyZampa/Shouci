//! Ratatui terminal UI: a search + save screen and a saved-items list.

mod app;
mod state;
mod theme;
mod ui;

#[cfg(test)]
mod search_quality;

pub use app::{App, run};

#[cfg(test)]
mod tests {
    use super::App;
    use std::path::PathBuf;
    use vocab_dictionary::{CedictSource, SqliteDictionary, build_dictionary_db};
    use vocab_search::{DeterministicRanker, SearchService};

    #[test]
    fn app_does_not_start_quitting() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/dictionary/cedict-sample.u8");
        let artifact = std::fs::read(path).unwrap();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        build_dictionary_db(&mut conn, &CedictSource::default(), &artifact).unwrap();
        let dict = SqliteDictionary::from_connection(conn).unwrap();
        let service = SearchService::new(dict, DeterministicRanker::default());
        let user_conn = rusqlite::Connection::open_in_memory().unwrap();
        assert!(!App::new(service, user_conn).should_quit);
    }
}
