//! Load the GPL bank and the Open Trivia DB snapshot into the three room pools.

use std::fs;
use std::path::Path;

use innkeeper_world::{OpenQuestion, PoolCounts, RawQuestion, TriviaPools};
use serde::Deserialize;
use tracing::{info, warn};

use crate::config::Config;

#[derive(Deserialize)]
struct OursFile {
    rounds: std::collections::BTreeMap<String, Vec<FileItem>>,
}

#[derive(Deserialize)]
struct OpenFile {
    questions: Vec<OpenItem>,
}

#[derive(Deserialize)]
struct FileItem {
    question: String,
    correct_answer: String,
    incorrect_answers: Vec<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct OpenItem {
    category: String,
    question: String,
    correct_answer: String,
    incorrect_answers: Vec<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

pub(crate) fn load(config: &Config) -> (TriviaPools, PoolCounts) {
    let ours_path = config.trivia_dir.join("questions.json");
    let ours = read_ours(&ours_path);
    let open = read_open(&config.opentdb);
    TriviaPools::from_banks(ours, open)
}

pub(crate) fn log_counts(counts: &PoolCounts) {
    info!(
        adventure = counts.adventure,
        general = counts.general,
        screens = counts.screens,
        dropped_ours = counts.dropped_ours,
        dropped_open = counts.dropped_open,
        "trivia question pools"
    );
}

fn read_ours(path: &Path) -> Vec<RawQuestion> {
    let Some(text) = read_text(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<OursFile>(&text) {
        Ok(file) => file
            .rounds
            .into_values()
            .flatten()
            .map(raw_from_item)
            .collect(),
        Err(error) => {
            warn!(path = %path.display(), %error, "trivia questions.json not read");
            Vec::new()
        }
    }
}

fn read_open(path: &Path) -> Vec<OpenQuestion> {
    let Some(text) = read_text(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<OpenFile>(&text) {
        Ok(file) => file
            .questions
            .into_iter()
            .map(|item| OpenQuestion {
                category: item.category,
                question: raw_from_item(FileItem {
                    question: item.question,
                    correct_answer: item.correct_answer,
                    incorrect_answers: item.incorrect_answers,
                    kind: item.kind,
                }),
            })
            .collect(),
        Err(error) => {
            warn!(path = %path.display(), %error, "opentdb snapshot not read");
            Vec::new()
        }
    }
}

fn raw_from_item(item: FileItem) -> RawQuestion {
    RawQuestion {
        boolean: item.kind.as_deref() == Some("boolean"),
        question: item.question,
        correct: item.correct_answer,
        incorrect: item.incorrect_answers,
    }
}

fn read_text(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            warn!(path = %path.display(), %error, "trivia file missing");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_checked_in_banks_fill_the_three_rooms() {
        let ours = read_ours(&repo_root().join("data/trivia/questions.json"));
        let open = read_open(&repo_root().join("third_party/opentdb/opentdb.json"));
        assert!(!ours.is_empty());
        assert!(!open.is_empty());
        let (_pools, counts) = TriviaPools::from_banks(ours, open);
        assert!(counts.adventure >= 200, "{counts:?}");
        assert!(counts.general >= 200, "{counts:?}");
        assert!(counts.screens >= 200, "{counts:?}");
    }
}
