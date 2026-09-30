use std::path::Path;

use vocab_core::{ConfirmationState, ItemStatus, Provenance, SourceId, SourceVersion};
use vocab_db::{NewVocabItem, SaveOutcome, list_items, open_user_db, save_vocab_item};
use vocab_exchange::{ExportOptions, ImportOptions, expand_path, export_file, import_file};

fn user_db() -> (tempfile_dir::Dir, rusqlite::Connection) {
    let dir = tempfile_dir::Dir::new();
    let conn = open_user_db(&dir.path().join("user.db")).unwrap();
    (dir, conn)
}

fn saved(conn: &rusqlite::Connection, simplified: &str, definition: &str) {
    let outcome = save_vocab_item(
        conn,
        &NewVocabItem {
            simplified: simplified.to_owned(),
            traditional: simplified.to_owned(),
            pinyin: "ni3 hao3".to_owned(),
            definition: definition.to_owned(),
            status: ItemStatus::Confirmed,
            notes: None,
            source_entry_id: None,
            provenance: Provenance {
                source: SourceId("user".to_owned()),
                source_version: SourceVersion("manual".to_owned()),
                import_origin: None,
                confirmation: ConfirmationState::UserConfirmed,
            },
            origin_export_id: None,
        },
    )
    .unwrap();
    assert!(matches!(outcome, SaveOutcome::Inserted(_)));
}

#[test]
fn import_keeps_existing_word_and_marks_the_target() {
    let (dir, mut conn) = user_db();
    saved(&conn, "你好", "hello from me");
    let path = dir.path().join("in.txt");
    std::fs::write(&path, "你好\tni3 hao3\thello from pleco\n").unwrap();
    let report = import_file(
        &mut conn,
        None,
        &path,
        &ImportOptions {
            codec: "pleco-utf8-text/v1".to_owned(),
            force: false,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(report.summary.contains("duplicates skipped"), "{report:?}");
    let item = list_items(&conn, None).unwrap().remove(0);
    assert_eq!(item.definition, "hello from me");

    let out = dir.path().join("out.txt");
    let exported = export_file(
        &mut conn,
        None,
        &out,
        &ExportOptions {
            codec: "pleco-utf8-text/v1".to_owned(),
            only_new: true,
            ..ExportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(exported.summary, "nothing to export");
}

#[test]
fn anki_export_writes_the_definition() {
    let (dir, mut conn) = user_db();
    saved(&conn, "你好", "hello from me");
    let out = dir.path().join("cards.txt");
    export_file(
        &mut conn,
        None,
        &out,
        &ExportOptions {
            codec: "anki-text/v1".to_owned(),
            ..ExportOptions::default()
        },
    )
    .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("#separator:tab"), "{text}");
    assert!(text.contains("hello from me"), "{text}");
}

#[test]
fn tilde_import_records_the_expanded_path_and_guards_export() {
    let (_dir, mut conn) = user_db();
    let Ok(home) = expand_path(Path::new("~")) else {
        // No HOME on this machine; the unit tests already cover that branch.
        return;
    };
    let scratch = tempfile_dir::InHome::new();
    let literal = scratch.typed();
    let real = home.join(scratch.relative());
    std::fs::write(&real, "你好\tni3 hao3\thello\n").unwrap();

    import_file(
        &mut conn,
        None,
        Path::new(&literal),
        &ImportOptions {
            codec: "pleco-utf8-text/v1".to_owned(),
            force: false,
            dry_run: false,
        },
    )
    .unwrap();

    // The audit stores the expanded path, so the export guard compares like
    // with like instead of missing a literal `~/...` entry.
    let sources = vocab_db::import_sources(&conn).unwrap();
    assert_eq!(
        sources,
        vec![real.to_string_lossy().into_owned()],
        "{sources:?}"
    );

    // Writing back over the file it was imported from is still refused, and
    // refused through the same `~/...` spelling.
    let err = export_file(
        &mut conn,
        None,
        Path::new(&literal),
        &ExportOptions::default(),
    )
    .expect_err("must not overwrite its own import source");
    assert!(
        err.to_string()
            .contains("refusing to overwrite import source"),
        "{err}"
    );
}

mod tempfile_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use vocab_exchange::expand_path;

    pub struct Dir {
        path: PathBuf,
    }

    impl Dir {
        pub fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "vocab-exchange-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A scratch directory inside the real home, so a test can exercise a
    /// literal `~/...` path. `unsafe_code` is forbidden workspace-wide, so
    /// `HOME` cannot be redirected to a temp dir for this.
    pub struct InHome {
        name: String,
    }

    impl InHome {
        pub fn new() -> Self {
            let name = format!(".shouci-transfer-test-{}", std::process::id());
            let home = expand_path(Path::new("~")).expect("home for scratch dir");
            std::fs::create_dir_all(home.join(&name)).expect("create scratch dir");
            Self { name }
        }

        /// The path as a user would type it, tilde and all.
        pub fn typed(&self) -> String {
            format!("~/{}/cards.txt", self.name)
        }

        /// The same path relative to the home directory.
        pub fn relative(&self) -> String {
            format!("{}/cards.txt", self.name)
        }
    }

    impl Drop for InHome {
        fn drop(&mut self) {
            if let Ok(home) = expand_path(Path::new("~")) {
                let _ = std::fs::remove_dir_all(home.join(&self.name));
            }
        }
    }
}
