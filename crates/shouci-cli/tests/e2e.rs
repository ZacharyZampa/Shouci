//! The `shouci` binary, run as people and scripts run it, against a scratch
//! library holding the sample dictionary (学校, 你好, 猫, 旅行, 米饭, …).
//!
//! Add a flow as a named test that drives a [`Library`].

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use shouci_core::testing::{SAMPLE_DICTIONARY, install_dictionary, scratch_dir};

/// A data directory for one test, removed when it ends.
struct Library {
    dir: PathBuf,
}

impl Library {
    fn new() -> Self {
        let dir = scratch_dir("cli");
        install_dictionary(&dir.join("dictionaries"), "cc-cedict", SAMPLE_DICTIONARY)
            .expect("install the sample dictionary");
        Self { dir }
    }

    fn output(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_shouci"))
            .env("SHOUCI_HOME", &self.dir)
            .env_remove("SHOUCI_DICTIONARIES")
            .args(args)
            .output()
            .expect("run shouci")
    }

    /// Stdout of a command that must succeed.
    fn run(&self, args: &[&str]) -> String {
        let output = self.output(args);
        let (stdout, stderr) = texts(&output);
        assert!(
            output.status.success(),
            "shouci {args:?} failed ({:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        );
        stdout
    }

    /// Stdout and stderr of a command that must fail.
    fn fail(&self, args: &[&str]) -> (String, String) {
        let output = self.output(args);
        let (stdout, stderr) = texts(&output);
        assert_eq!(
            output.status.code(),
            Some(1),
            "shouci {args:?} should fail\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        (stdout, stderr)
    }

    /// A successful command's `--json` result.
    fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.insert(0, "--json");
        let stdout = self.run(&args);
        serde_json::from_str(&stdout).unwrap_or_else(|err| panic!("{err}: {stdout}"))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn texts(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn path(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

#[test]
fn search_reads_characters_pinyin_and_english() {
    let library = Library::new();
    for (query, headword) in [
        ("school", "学校 / 學校 [xué xiào]  school"),
        ("nihao", "你好 [nǐ hǎo]"),
        ("lǚ xíng", "旅行 [lǚ xíng]"),
        ("旅行", "旅行 [lǚ xíng]  to travel; travel"),
    ] {
        let stdout = library.run(&["search", query]);
        assert!(
            stdout.starts_with(&format!("  1  {headword}")),
            "{query}: {stdout}"
        );
    }
    // Words after the first are part of the query
    assert!(library.run(&["search", "to", "travel"]).contains("旅行"));
    // and --as reads it one way only
    let json = library.json(&["search", "mao", "--as", "english"]);
    assert_eq!(json["kind"], "english");
    assert_eq!(json["candidates"], Value::Array(Vec::new()));
}

#[test]
fn search_json_is_the_cores_results() {
    let library = Library::new();
    let json = library.json(&["search", "school"]);
    assert_eq!(json["query"], "school");
    assert_eq!(json["kind"], "english");
    let first = &json["candidates"][0];
    assert_eq!(first["simplified"], "学校");
    assert_eq!(first["pinyin"], "xue2 xiao4");
    assert_eq!(first["pinyin_display"], "xué xiào");
    assert_eq!(first["saved"], Value::Null);
}

#[test]
fn add_saves_once_and_search_marks_it() {
    let library = Library::new();
    let saved = library.run(&["add", "学校"]);
    assert_eq!(saved.trim(), "saved 学校 / 學校 [xué xiào]  school");
    assert!(
        library
            .run(&["add", "xuexiao"])
            .starts_with("already saved: 学校")
    );
    assert!(
        library
            .run(&["list"])
            .contains("学校 / 學校 [xué xiào]  school")
    );
    assert!(
        library
            .run(&["search", "school"])
            .contains("school  · saved")
    );
}

#[test]
fn add_saves_nothing_when_several_words_match() {
    let library = Library::new();
    let (stdout, _) = library.fail(&["add", "lv3"]);
    assert!(stdout.starts_with("several words match"), "{stdout}");
    assert!(
        stdout.contains("旅行") && stdout.contains("旅途"),
        "{stdout}"
    );
    assert_eq!(library.run(&["list"]).trim(), "no saved words");
    // A result can be picked by its number in `shouci search`
    let second = library
        .run(&["search", "lv3"])
        .lines()
        .nth(1)
        .unwrap()
        .to_owned();
    let picked = library.run(&["add", "lv3", "--pick", "2"]);
    assert!(
        second.contains(picked.trim().trim_start_matches("saved ")),
        "{second} / {picked}"
    );
    library.fail(&["add", "lv3", "--pick", "9"]);
}

#[test]
fn unknown_characters_are_kept_to_fill_in_later() {
    let library = Library::new();
    let search = library.run(&["search", "蚌埠住了"]);
    assert!(
        search.starts_with("no dictionary entry for 蚌埠住了"),
        "{search}"
    );
    let saved = library.run(&["add", "蚌埠住了"]);
    assert!(saved.contains("to fill in later"), "{saved}");
    assert!(
        library
            .run(&["list", "--needs-review"])
            .contains("蚌埠住了")
    );
}

#[test]
fn a_word_typed_in_by_hand() {
    let library = Library::new();
    library.run(&[
        "add",
        "加油",
        "--pinyin",
        "jia1 you2",
        "--definition",
        "come on!",
        "--tag",
        "cheers",
        "--collection",
        "Sports",
    ]);
    let shown = library.run(&["show", "加油"]);
    assert!(shown.starts_with("加油 [jiā yóu]\ncome on!\n"), "{shown}");
    assert!(shown.contains("tags: cheers"), "{shown}");
    assert!(shown.contains("collections: Sports"), "{shown}");
    assert!(shown.contains("by hand"), "{shown}");
    // Looked-up words take tags too
    library.run(&["add", "米饭", "--tag", "food"]);
    assert_eq!(library.json(&["show", "米饭"])["item"]["tags"][0], "food");
}

#[test]
fn show_puts_the_dictionary_beside_the_word() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    let json = library.json(&["show", "學校"]);
    assert_eq!(json["item"]["simplified"], "学校");
    assert_eq!(json["dictionaries"][0]["id"], "cc-cedict");
    assert_eq!(json["dictionaries"][0]["entries"][0]["same_word"], true);
    assert!(
        library
            .run(&["show", "学校"])
            .contains(":\n  学校 / 學校 [xué xiào]  school")
    );
}

#[test]
fn edit_changes_a_word() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    let changed = library.run(&["edit", "学校", "--notes", "near home", "--needs-review"]);
    assert!(changed.contains("needs review"), "{changed}");
    let shown = library.run(&["show", "学校"]);
    assert!(shown.contains("notes: near home"), "{shown}");
    // Nothing to change is a usage error
    assert_eq!(library.output(&["edit", "学校"]).status.code(), Some(2));
}

#[test]
fn trash_restore_and_purge() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    library.run(&["add", "米饭"]);
    let moved = library.run(&["delete", "学校", "米饭"]);
    assert!(
        moved.starts_with("moved 学校 and 米饭 to the trash"),
        "{moved}"
    );
    assert_eq!(library.run(&["list"]).trim(), "no saved words");
    assert!(
        library
            .run(&["list", "--view", "trash"])
            .contains("in the trash")
    );

    assert!(
        library
            .run(&["restore", "学校"])
            .contains("brought 学校 back")
    );
    library.fail(&["purge", "学校"]);
    let gone = library.run(&["purge", "米饭"]);
    assert_eq!(gone.trim(), "deleted 米饭 for good");
    library.run(&["delete", "学校"]);
    assert_eq!(library.json(&["purge", "--all"])["changed"], 1);
    assert_eq!(
        library.run(&["list", "--view", "all"]).trim(),
        "no saved words"
    );
}

#[test]
fn archive_keeps_words_out_of_the_way() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    library.run(&["archive", "学校"]);
    assert_eq!(library.run(&["list"]).trim(), "no saved words");
    assert!(
        library
            .run(&["list", "--view", "archived"])
            .contains("archived")
    );
    library.run(&["unarchive", "学校"]);
    assert!(library.run(&["list"]).contains("学校"));
}

#[test]
fn tags_and_collections() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    library.run(&["add", "米饭"]);
    library.run(&["tags", "add", "hsk1", "学校", "米饭"]);
    assert_eq!(library.run(&["tags"]).trim(), "hsk1  (2)");
    library.run(&["tags", "remove", "hsk1", "米饭"]);
    let tagged = library.run(&["list", "--tag", "hsk1"]);
    assert!(
        tagged.contains("学校") && !tagged.contains("米饭"),
        "{tagged}"
    );
    library.run(&["tags", "rename", "hsk1", "first"]);
    assert_eq!(library.json(&["tags"])[0]["name"], "first");

    library.run(&["collections", "add", "Food", "米饭"]);
    assert_eq!(library.run(&["collections"]).trim(), "Food  (1)");
    assert!(
        library
            .run(&["list", "--collection", "Food"])
            .contains("米饭")
    );
    library.run(&["collections", "delete", "Food"]);
    assert_eq!(library.run(&["collections"]).trim(), "no collections");
    assert!(library.run(&["list"]).contains("米饭"), "its words stay");
}

#[test]
fn merging_groups_and_finding_words_in_none() {
    let library = Library::new();
    library.run(&["add", "学校"]);
    library.run(&["add", "米饭"]);
    library.run(&["add", "猫"]);
    library.run(&["collections", "add", "Week_1", "学校"]);
    library.run(&["collections", "add", "Week 1", "米饭"]);
    let unsorted = library.run(&["list", "--no-collection"]);
    assert!(
        unsorted.contains("猫") && !unsorted.contains("学校"),
        "{unsorted}"
    );
    library.run(&["collections", "merge", "Week_1", "Week 1"]);
    assert_eq!(library.run(&["collections"]).trim(), "Week 1  (2)");
    library.run(&["tags", "add", "hsk1", "学校"]);
    library.run(&["tags", "add", "HSK 1", "米饭"]);
    library.run(&["tags", "merge", "hsk1", "HSK 1"]);
    assert_eq!(library.run(&["tags"]).trim(), "HSK 1  (2)");
}

#[test]
fn smart_collections_keep_a_filter_to_list_and_export_later() {
    let library = Library::new();
    for word in ["学校", "猫", "米饭"] {
        library.run(&["add", word]);
    }
    library.run(&["tags", "add", "drilled", "猫"]);
    library.run(&["collections", "add", "Food", "米饭"]);
    let filtered = library.run(&[
        "list",
        "--without-tag",
        "drilled",
        "--without-collection",
        "Food",
    ]);
    assert!(
        filtered.contains("学校") && !filtered.contains("猫") && !filtered.contains("米饭"),
        "{filtered}"
    );

    let saved = library.run(&[
        "smart",
        "save",
        "To Drill",
        "--without-tag",
        "drilled",
        "--without-collection",
        "Food",
    ]);
    assert_eq!(
        saved.trim(),
        "saved smart collection To Drill (1 word): Not tagged drilled · Not in Food"
    );
    let listed = library.run(&["list", "--smart", "to drill"]);
    assert!(
        listed.contains("学校") && !listed.contains("猫"),
        "{listed}"
    );
    assert_eq!(
        library.run(&["smart"]).trim(),
        "To Drill  (1)  Not tagged drilled · Not in Food"
    );

    library.run(&["tags", "rename", "drilled", "done"]);
    let changed = library.run(&[
        "smart",
        "save",
        "To Drill",
        "--any-tag",
        "done",
        "--hsk",
        "none",
    ]);
    assert_eq!(
        changed.trim(),
        "changed smart collection To Drill (1 word): No HSK level · Tagged done"
    );
    let json = library.json(&["smart", "list"]);
    assert_eq!(json[0]["filter"]["any_tags"][0], "done");
    assert_eq!(json[0]["item_ids"].as_array().unwrap().len(), 1);

    let out = library.file("drill.txt");
    library.run(&["export", path(&out), "--all", "--smart", "To Drill"]);
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(
        written.contains("猫") && !written.contains("学校"),
        "{written}"
    );

    library.run(&["smart", "rename", "To Drill", "Done"]);
    let (_, err) = library.fail(&["list", "--smart", "To Drill"]);
    assert!(err.contains("no smart collection named To Drill"), "{err}");
    library.run(&["smart", "delete", "Done"]);
    assert!(library.run(&["smart"]).starts_with("no smart collections"));
}

#[test]
fn import_then_export_only_what_is_new() {
    let library = Library::new();
    let pleco = fixture("pleco/v1/valid/basic-flashcards.txt");
    let preview = library.run(&["import", path(&pleco), "--dry-run"]);
    assert!(preview.contains("would add 2 words"), "{preview}");
    assert_eq!(library.run(&["list"]).trim(), "no saved words");

    let imported = library.run(&["import", path(&pleco)]);
    assert!(
        imported.starts_with("added 2 words from Pleco file"),
        "{imported}"
    );
    library.run(&["add", "学校"]);

    // To Pleco, only 学校 is new: the other two came from there
    let out = library.file("to-pleco.txt");
    let exported = library.run(&["export", path(&out)]);
    assert!(
        exported.starts_with("wrote 1 word to Pleco file"),
        "{exported}"
    );
    assert!(std::fs::read_to_string(&out).unwrap().contains("学校"));
    let again = library.run(&["export", path(&out)]);
    assert!(again.starts_with("nothing to export"), "{again}");

    // Writing over a file takes --replace
    let (_, stderr) = library.fail(&["export", path(&out), "--all"]);
    assert!(stderr.contains("--replace"), "{stderr}");
    let all = library.json(&["export", path(&out), "--all", "--replace"]);
    assert_eq!(all["written"], 3);

    // An export to Anki reads back as Anki
    let anki = library.file("anki.txt");
    library.run(&["export", path(&anki), "--to", "anki"]);
    let plan = library.json(&["import", path(&anki), "--dry-run"]);
    assert_eq!(plan["connector_id"], "anki");
}

#[test]
fn an_import_with_error_lines_changes_nothing_without_force() {
    let library = Library::new();
    let broken = library.file("broken.txt");
    std::fs::write(&broken, "你好\tni3 hao3\thello\n\tni3 hao3\thello\n").unwrap();
    let (stdout, stderr) = library.fail(&["import", path(&broken), "--from", "pleco"]);
    assert!(stdout.contains("line 2: error"), "{stdout}");
    assert!(stderr.contains("--force"), "{stderr}");
    assert_eq!(library.run(&["list"]).trim(), "no saved words");
    library.run(&["import", path(&broken), "--from", "pleco", "--force"]);
    assert!(library.run(&["list"]).contains("你好"));
}

#[test]
fn errors_say_what_went_wrong() {
    let library = Library::new();
    let (_, stderr) = library.fail(&["show", "学校"]);
    assert_eq!(stderr.trim(), "shouci: no saved word 学校");
    let (_, stderr) = library.fail(&["--json", "show", "学校"]);
    let error: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(error["error"]["kind"], "not_found");
}

#[test]
fn dictionaries_list_and_choose() {
    let library = Library::new();
    let listed = library.json(&["dictionaries"]);
    assert_eq!(listed[0]["id"], "cc-cedict");
    assert_eq!(listed[0]["enabled"], true);
    library.fail(&["dictionaries", "use", "nope"]);
}
