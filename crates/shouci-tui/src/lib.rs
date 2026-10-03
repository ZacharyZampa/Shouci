//! Ratatui terminal UI over `shouci-core`: a search + save screen and the
//! saved-word lists.

mod app;
mod state;
mod theme;
mod ui;

pub use app::{App, run};

#[cfg(test)]
mod tests {
    use super::App;

    #[test]
    fn app_does_not_start_quitting() {
        let library = shouci_core::testing::sandbox();
        assert!(!App::new(&library).should_quit);
    }
}
