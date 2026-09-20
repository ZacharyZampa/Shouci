//! One-shot ingest of a plain-text lexicon artifact into `dictionary.db`.
//!
//! Usage:
//! ```text
//! cargo run -p vocab-dictionary --example ingest -- \
//!   <cedict.u8> <output.db> [--frequency FILE] [--hsk FILE]
//! ```
//!
//! The build is transactional and wipes prior rows, so re-running against the
//! same artifacts reproduces the same database content.

use std::path::PathBuf;

use vocab_dictionary::{
    CedictSource, FrequencySource, HskSource, IngestSource, build_dictionary_db_with_layers,
};

fn main() -> vocab_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let artifact_path = PathBuf::from(
        args.next()
            .expect("usage: ingest <cedict> <output.db> [--frequency FILE] [--hsk FILE]"),
    );
    let output_path = PathBuf::from(
        args.next()
            .expect("usage: ingest <cedict> <output.db> [--frequency FILE] [--hsk FILE]"),
    );
    let mut frequency_path = None;
    let mut hsk_path = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--frequency" => {
                frequency_path = Some(PathBuf::from(
                    args.next().expect("--frequency needs a file"),
                ));
            }
            "--hsk" => {
                hsk_path = Some(PathBuf::from(args.next().expect("--hsk needs a file")));
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let artifact = std::fs::read(&artifact_path)
        .map_err(|err| vocab_core::VocabError::new(format!("read artifact: {err}")))?;
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| vocab_core::VocabError::new(format!("create output dir: {err}")))?;
        }
    }
    let mut conn = rusqlite::Connection::open(&output_path).map_err(|err| {
        vocab_core::VocabError::new(format!("open {}: {err}", output_path.display()))
    })?;

    let frequency_bytes = frequency_path
        .as_ref()
        .map(|path| {
            std::fs::read(path)
                .map_err(|err| vocab_core::VocabError::new(format!("read frequency: {err}")))
        })
        .transpose()?;
    let hsk_bytes = hsk_path
        .as_ref()
        .map(|path| {
            std::fs::read(path)
                .map_err(|err| vocab_core::VocabError::new(format!("read hsk: {err}")))
        })
        .transpose()?;

    let frequency = FrequencySource::default();
    let hsk = HskSource::default();
    let mut layers: Vec<(&dyn IngestSource, &[u8])> = Vec::new();
    if let Some(bytes) = frequency_bytes.as_deref() {
        layers.push((&frequency, bytes));
    }
    if let Some(bytes) = hsk_bytes.as_deref() {
        layers.push((&hsk, bytes));
    }

    let stats =
        build_dictionary_db_with_layers(&mut conn, &CedictSource::default(), &artifact, &layers)?;
    println!(
        "ingested {} entries, {} glosses (freq {}, hsk {}) -> {}",
        stats.entries,
        stats.glosses,
        stats.frequency_updated,
        stats.hsk_updated,
        output_path.display()
    );
    Ok(())
}
