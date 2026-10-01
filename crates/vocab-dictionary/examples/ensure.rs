//! Fetch or reuse `cc-cedict.db` via [`vocab_dictionary::ensure_dictionary_db`].
//!
//! ```text
//! cargo run -p vocab-dictionary --example ensure -- [data/dictionaries/cc-cedict.db]
//! cargo run -p vocab-dictionary --example ensure -- --force [data/dictionaries/cc-cedict.db]
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
    let path = path.unwrap_or_else(|| PathBuf::from("data/dictionaries/cc-cedict.db"));
    if force {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("fetched"));
    }
    let outcome = vocab_dictionary::ensure_dictionary_db(&path, &mut |stage| {
        eprintln!("{stage:?}…");
    })?;
    if let vocab_dictionary::Ensured::RefreshFailed(err) = &outcome {
        eprintln!("refresh failed; keeping the existing build: {err}");
    }
    println!("dictionary ready ({outcome:?}): {}", path.display());
    Ok(())
}
