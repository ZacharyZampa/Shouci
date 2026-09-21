//! Process-level checks of the `vocab` binary.
//!
//! Add a search case by appending a row. Add a save/import flow as a named
//! test that calls [`run`]. Requires `dictionary.db` (see README).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn dictionary() -> PathBuf {
    if let Ok(db) = std::env::var("VOCAB_DICTIONARY") {
        if !db.is_empty() {
            let path = PathBuf::from(db);
            if path.is_file() {
                return path;
            }
        }
    }
    let repo =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/dictionary/dictionary.db");
    if repo.is_file() {
        return repo;
    }
    if let Ok(home) = std::env::var("HOME") {
        let installed =
            PathBuf::from(home).join("Library/Application Support/pleco-companion/dictionary.db");
        if installed.is_file() {
            return installed;
        }
    }
    panic!("e2e tests need dictionary.db — ingest it first (see README Dictionary)");
}

fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn temp_user_db() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "vocab-e2e-{}-{}-{}",
        std::process::id(),
        nanos,
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join("user.db")
}

fn run(user_db: Option<&Path>, args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vocab"));
    cmd.arg("--dictionary").arg(dictionary());
    if let Some(path) = user_db {
        cmd.arg("--user-db").arg(path);
    }
    let output = cmd.args(args).output().expect("spawn vocab");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "vocab {args:?} failed {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    stdout
}

fn has_headword(stdout: &str, simplified: &str) {
    assert!(
        stdout.lines().any(|line| line.contains(simplified)),
        "expected {simplified:?} in:\n{stdout}"
    );
}

#[test]
fn search_examples_surface_expected_headwords() {
    for (mode, query, headword) in [
        ("auto", "jingzi", "镜子"),
        ("english", "school", "学校"),
        ("pinyin", "nihao", "你好"),
        ("chinese", "旅行", "旅行"),
    ] {
        let stdout = run(None, &["search", mode, query, "--limit", "10"]);
        has_headword(&stdout, headword);
    }
}

#[test]
fn add_unique_chinese_then_list_and_refuse_duplicate() {
    let user_db = temp_user_db();
    let saved = run(Some(&user_db), &["add", "学校", "--mode", "chinese"]);
    assert!(
        saved.starts_with("saved:") && saved.contains("学校"),
        "{saved}"
    );

    has_headword(&run(Some(&user_db), &["list"]), "学校");

    let again = run(Some(&user_db), &["add", "学校", "--mode", "chinese"]);
    assert!(again.starts_with("already saved:"), "{again}");
}

#[test]
fn add_unknown_word_saves_needs_review() {
    let user_db = temp_user_db();
    let stdout = run(Some(&user_db), &["add", "zzzqqqnotaword"]);
    assert!(stdout.contains("needs_review"), "{stdout}");
}

#[test]
fn import_pleco_fixture_then_list() {
    let user_db = temp_user_db();
    let fixture = fixture_path("pleco/v1/valid/basic-flashcards.txt");
    let imported = run(
        Some(&user_db),
        &["import", fixture.to_str().expect("utf8 path")],
    );
    assert!(imported.contains("imported"), "{imported}");
    has_headword(&run(Some(&user_db), &["list"]), "你好");
}

#[test]
fn export_after_add_writes_pleco_text() {
    let user_db = temp_user_db();
    run(Some(&user_db), &["add", "学校", "--mode", "chinese"]);
    let out = user_db.parent().expect("temp dir").join("export.txt");
    let exported = run(
        Some(&user_db),
        &["export", out.to_str().expect("utf8 path")],
    );
    assert!(exported.contains("exported"), "{exported}");
    let text = std::fs::read_to_string(&out).expect("export file");
    assert!(text.contains("学校"), "{text}");
}
