use std::path::{Path, PathBuf};

use vocab_dictionary::{CedictSource, build_dictionary_db};

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dictionary/cedict-sample.u8");
    std::fs::read(&path).expect("missing cedict sample fixture")
}

fn build_once(dir: &Path, name: &str) -> Vec<u8> {
    let path = dir.join(name);
    let mut conn = rusqlite::Connection::open(&path).expect("open build db");
    build_dictionary_db(&mut conn, &CedictSource::default(), &fixture()).expect("build");
    drop(conn);
    std::fs::read(&path).expect("read built db")
}

#[test]
fn rebuild_produces_byte_identical_files() {
    let dir = std::env::temp_dir().join(format!("vocab-determinism-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let first = build_once(&dir, "first.db");
    let second = build_once(&dir, "second.db");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        first, second,
        "rebuilding the same artifact must be byte-identical"
    );
}
