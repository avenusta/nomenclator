//! Loads the corpus cases. The library runner, the wire runner and the scope test all
//! read the corpus through this module, so they read each case in the same way.

// Each runner uses a different part of this module.
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One corpus case, as `corpus/README.md` describes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusCase {
    pub name: String,
    #[serde(default)]
    pub pending: bool,
    /// The ID of a decision whose ruling contradicts this case's expectation. The case
    /// cannot pass until the decision is settled.
    #[serde(default)]
    pub diverges: Option<String>,
    /// The overlay documents (YAML or JSON) applied to the built-in rules for this case.
    #[serde(default)]
    pub overlays: Vec<String>,
    /// The library's declared languages (X11). `None` means every language.
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    pub operation: String,
    pub input: Value,
    pub expect: Value,
}

impl CorpusCase {
    /// Returns `true` if the case runs under the built-in rules.
    pub fn uses_builtin_rules(&self) -> bool {
        self.overlays.is_empty() && self.languages.is_none()
    }
}

/// Returns the corpus folder.
pub fn corpus_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Returns the cases of one corpus file. Panics if the file is not valid.
pub fn load_cases(file: &Path) -> Vec<CorpusCase> {
    let text = std::fs::read_to_string(file).expect("corpus file is readable");
    nomenclator::parse_document(&text).unwrap_or_else(|error| panic!("{}: {error}", file.display()))
}

/// Returns every YAML file below `directory`, sorted by path.
pub fn collect_yaml_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("corpus directory exists") {
        let path = entry.expect("corpus entry is readable").path();
        if path.is_dir() {
            files.extend(collect_yaml_files(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "yaml")
        {
            files.push(path);
        }
    }
    files.sort();
    files
}
