//! Fetch or reuse `dictionary.db` via [`vocab_dictionary::ensure_dictionary_db`].
//!
//! ```text
//! cargo run -p vocab-dictionary --example ensure -- [dictionary.db]
//! cargo run -p vocab-dictionary --example ensure -- --force [dictionary.db]
//! ```

use std::path::PathBuf;

fn main() -> vocab_core::Result<()> {
    let mut force = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        if arg == "--force" {
            force = true;
        } else {
            path = Some(PathBuf::from(arg));
        }
    }
    let path = path.unwrap_or_else(|| PathBuf::from("data/dictionary/dictionary.db"));
    if force {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("fetched"));
    }
    vocab_dictionary::ensure_dictionary_db(&path)?;
    println!("dictionary ready: {}", path.display());
    Ok(())
}
