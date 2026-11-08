//! Evaluates exclusion by built-in exclusions and caller-supplied ignore rules, with
//! inheritance and anchoring.

mod pattern;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::lexical::fold_case_text;
use crate::model::EntryKind;
use crate::rules::{IgnoreNesting, Rules, Vocabularies};
use pattern::{ParsedLine, Pattern};

/// A rule file that the caller found in the library, by root-relative path. A no-media marker
/// (`.nomedia` unless the rules rename it) excludes its folder. Any other file (`.ignore`, or
/// a rule set that the caller adds) contains rules for its folder in its dialect.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IgnoreFile {
    /// The file's root-relative path.
    pub path: String,
    /// `None` if the caller could not read the file.
    #[serde(default)]
    pub contents: Option<String>,
    /// The syntax of its lines (X14).
    #[serde(default)]
    pub dialect: IgnoreDialect,
}

/// The syntax of a rule file's lines (X14).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum IgnoreDialect {
    /// gitignore: negation with `!`, escapes with `\`, a trailing `/` for folders only.
    #[default]
    Gitignore,
    /// One glob per line. A pattern without `/` matches a name at any depth. A pattern with
    /// `/` is relative to the rule file's folder. There is no negation and no escaping.
    Glob,
}

/// The exclusion result for one entry, with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IgnoreEvaluation {
    /// `None` if an unreadable rule file could change the result.
    pub excluded: Option<bool>,
    /// The reason for the exclusion, if the entry is excluded.
    pub exclusion: Option<Exclusion>,
    /// Unreadable rule files that could decide this entry.
    pub unreadable_rule_files: Vec<String>,
    /// Problems in the supplied rule files, wherever they apply.
    pub rule_diagnostics: Vec<RuleDiagnostic>,
}

/// The reason an entry is excluded: the entry or folder that matched, and the built-in or
/// rule that matched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Exclusion {
    /// The root-relative path that matched: the entry itself or a folder above it.
    pub path: String,
    /// The kind of built-in or rule that matched.
    pub reason: ExclusionReason,
    /// The rule file that matched; `None` for a built-in.
    pub rule_file: Option<String>,
    /// The 1-based line of `rule_file` whose pattern matched.
    pub line: Option<usize>,
}

/// The built-in or rule that excluded an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    /// A name that starts with a dot.
    HiddenEntry,
    /// NAS, OS and sync-tool entries.
    SystemJunk,
    /// An unfinished download, such as a `.part` file.
    PartialDownload,
    /// Folders that other tools generate, such as `metadata` and `.trickplay`.
    GeneratedMetadata,
    /// A `.nomedia` marker in the folder or one above it.
    NoMediaMarker,
    /// A rule file without patterns (empty, blank or comments only), which excludes its folder.
    EmptyRuleFile,
    /// A pattern in a rule file.
    Rule,
    /// A sample, under the excluded-samples preset (E6, X12).
    Sample,
}

/// A problem in a supplied rule file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleDiagnostic {
    /// The rule file's root-relative path.
    pub rule_file: String,
    /// The 1-based line; `None` for a problem with the whole file.
    pub line: Option<usize>,
    /// The problem.
    pub reason: RuleDiagnosticReason,
}

/// A problem with a rule file or one of its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleDiagnosticReason {
    /// The line's `[` has no closing `]`. The line matches no entry.
    UnclosedBracket,
    /// The line contains a `[:class:]` that does not exist. The line matches no entry.
    UnknownCharacterClass,
    /// The line ends in an unescaped `\`. The line matches no entry.
    TrailingBackslash,
    /// The line contains only `!` or slashes. The line matches no entry.
    EmptyPattern,
    /// The file's path is outside the library root. The file is not applied.
    OutsideLibraryRoot,
}

/// Rule files compiled once, for evaluating many entries of one library.
#[derive(Debug, Clone, Default)]
pub struct IgnoreRules {
    /// Keyed by the root-relative folder path (`""` for the root).
    folders: BTreeMap<String, FolderRules>,
    diagnostics: Vec<RuleDiagnostic>,
    built_ins: BuiltIns,
    nesting: IgnoreNesting,
}

/// The vocabularies of the built-in exclusions (I6-I9, X13, X14), which no rule file can
/// override.
#[derive(Debug, Clone)]
struct BuiltIns {
    built_in_ignore_groups: BTreeSet<String>,
    no_media_markers: BTreeSet<String>,
    system_junk_names: BTreeSet<String>,
    system_junk_file_suffixes: BTreeSet<String>,
    partial_download_suffixes: BTreeSet<String>,
    generated_metadata_folders: BTreeSet<String>,
    generated_metadata_folder_suffixes: BTreeSet<String>,
}

impl BuiltIns {
    fn of(vocab: &Vocabularies) -> BuiltIns {
        BuiltIns {
            built_in_ignore_groups: vocab.built_in_ignore_groups.clone(),
            no_media_markers: vocab.no_media_markers.clone(),
            system_junk_names: vocab.system_junk_names.clone(),
            system_junk_file_suffixes: vocab.system_junk_file_suffixes.clone(),
            partial_download_suffixes: vocab.partial_download_suffixes.clone(),
            generated_metadata_folders: vocab.generated_metadata_folders.clone(),
            generated_metadata_folder_suffixes: vocab.generated_metadata_folder_suffixes.clone(),
        }
    }
}

impl Default for BuiltIns {
    fn default() -> Self {
        BuiltIns::of(&Rules::builtin().vocab)
    }
}

#[derive(Debug, Clone, Default)]
struct FolderRules {
    /// A `.nomedia` marker or a rule file without patterns: the folder is excluded.
    marker: Option<(String, ExclusionReason)>,
    patterns: Vec<RuleLine>,
    unreadable_rule_files: Vec<String>,
}

#[derive(Debug, Clone)]
struct RuleLine {
    rule_file: String,
    line: usize,
    pattern: Pattern,
}

impl Rules {
    /// Evaluates whether an entry is excluded by these rules' built-ins or the supplied rule
    /// files.
    pub fn evaluate_ignore(
        &self,
        path: &str,
        kind: EntryKind,
        ignore_files: &[IgnoreFile],
    ) -> IgnoreEvaluation {
        self.ignore_rules(ignore_files).evaluate(path, kind)
    }

    /// Compiles rule files once, with these rules' built-in exclusions.
    pub fn ignore_rules(&self, ignore_files: &[IgnoreFile]) -> IgnoreRules {
        let mut sorted_files: Vec<&IgnoreFile> = ignore_files.iter().collect();
        sorted_files.sort_by(|first, second| first.path.cmp(&second.path));
        let mut rules = IgnoreRules {
            built_ins: BuiltIns::of(&self.vocab),
            nesting: self.vocab.presets.ignore_nesting,
            ..IgnoreRules::default()
        };
        for file in sorted_files {
            rules.add_file(file);
        }
        rules
    }
}

impl IgnoreRules {
    /// Compiles rule files once, with the built-in exclusions. Files in one folder apply in
    /// path order, so the result does not depend on the order in which they were supplied.
    #[must_use]
    pub fn new(ignore_files: &[IgnoreFile]) -> Self {
        Rules::builtin().ignore_rules(ignore_files)
    }

    /// Returns the problems in the supplied rule files.
    #[must_use]
    pub fn diagnostics(&self) -> &[RuleDiagnostic] {
        &self.diagnostics
    }

    fn add_file(&mut self, file: &IgnoreFile) {
        let (folder, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
        let is_inside_library = !file.path.starts_with('/')
            && !folder
                .split('/')
                .any(|component| component == ".." || component == ".");
        if !is_inside_library {
            self.diagnostics.push(RuleDiagnostic {
                rule_file: file.path.clone(),
                line: None,
                reason: RuleDiagnosticReason::OutsideLibraryRoot,
            });
            return;
        }
        let folder_rules = self.folders.entry(folder.to_owned()).or_default();
        let Some(contents) = &file.contents else {
            folder_rules.unreadable_rule_files.push(file.path.clone());
            return;
        };
        if self
            .built_ins
            .no_media_markers
            .contains(&fold_case_text(name))
        {
            folder_rules
                .marker
                .get_or_insert((file.path.clone(), ExclusionReason::NoMediaMarker));
            return;
        }
        // Some editors write a UTF-8 byte order mark first. It is not part of the first pattern.
        let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
        let mut has_pattern_lines = false;
        for (index, line) in contents.lines().enumerate() {
            match pattern::parse_line(line, file.dialect) {
                ParsedLine::Blank => {}
                ParsedLine::Pattern(pattern) => {
                    has_pattern_lines = true;
                    folder_rules.patterns.push(RuleLine {
                        rule_file: file.path.clone(),
                        line: index + 1,
                        pattern,
                    });
                }
                ParsedLine::Invalid(reason) => {
                    has_pattern_lines = true;
                    self.diagnostics.push(RuleDiagnostic {
                        rule_file: file.path.clone(),
                        line: Some(index + 1),
                        reason,
                    });
                }
            }
        }
        if !has_pattern_lines {
            folder_rules
                .marker
                .get_or_insert((file.path.clone(), ExclusionReason::EmptyRuleFile));
        }
    }

    /// Evaluates an entry from the library root down to the entry. An excluded folder
    /// excludes everything below it, and no rule below it can re-include an entry. Built-ins
    /// apply first and no rule overrides them. Rule files apply from the root down, and the
    /// last matching pattern takes precedence.
    #[must_use]
    pub fn evaluate(&self, path: &str, kind: EntryKind) -> IgnoreEvaluation {
        let path = path.strip_suffix('/').unwrap_or(path);
        let components: Vec<&str> = if path.is_empty() {
            Vec::new()
        } else {
            path.split('/').collect()
        };
        let mut unreadable_rule_files: Vec<String> = Vec::new();
        for depth in 0..=components.len() {
            let entry = &components[..depth];
            let entry_path = entry.join("/");
            let is_directory = depth < components.len() || kind == EntryKind::Directory;
            let exclusion = |reason, rule_file: Option<&String>, line| Exclusion {
                path: entry_path.clone(),
                reason,
                rule_file: rule_file.cloned(),
                line,
            };
            if let Some(name) = entry.last()
                && let Some(reason) = built_in_exclusion(&self.built_ins, name, is_directory)
            {
                return self.result(Some(exclusion(reason, None, None)), Vec::new());
            }
            if unreadable_rule_files.is_empty()
                && let Some(rule_line) = self.last_matching_rule(entry, is_directory)
                && !rule_line.pattern.is_negation
            {
                let found = exclusion(
                    ExclusionReason::Rule,
                    Some(&rule_line.rule_file),
                    Some(rule_line.line),
                );
                return self.result(Some(found), Vec::new());
            }
            let own_folder = if is_directory {
                self.folders.get(&entry_path)
            } else {
                None
            };
            if let Some(folder) = own_folder {
                unreadable_rule_files.extend(folder.unreadable_rule_files.iter().cloned());
                if unreadable_rule_files.is_empty()
                    && let Some((rule_file, reason)) = &folder.marker
                {
                    let found = exclusion(*reason, Some(rule_file), None);
                    return self.result(Some(found), Vec::new());
                }
            }
        }
        self.result(None, unreadable_rule_files)
    }

    /// Returns the last pattern that matches the entry among the rule files of the folders
    /// above it. All of these rule files apply, or under the nearest-only preset only the
    /// nearest folder that has patterns applies (I4, X12).
    fn last_matching_rule(&self, entry: &[&str], is_directory: bool) -> Option<&RuleLine> {
        let nearest_with_patterns = (0..entry.len()).rev().find(|depth| {
            self.folders
                .get(&entry[..*depth].join("/"))
                .is_some_and(|folder| !folder.patterns.is_empty())
        });
        let first_depth = match self.nesting {
            IgnoreNesting::Stack => 0,
            IgnoreNesting::Nearest => nearest_with_patterns.unwrap_or(0),
        };
        let mut last_match = None;
        for folder_depth in first_depth..entry.len() {
            let Some(folder) = self.folders.get(&entry[..folder_depth].join("/")) else {
                continue;
            };
            let relative_components = &entry[folder_depth..];
            for rule_line in &folder.patterns {
                if rule_line.pattern.matches(relative_components, is_directory) {
                    last_match = Some(rule_line);
                }
            }
        }
        last_match
    }

    fn result(
        &self,
        exclusion: Option<Exclusion>,
        unreadable_rule_files: Vec<String>,
    ) -> IgnoreEvaluation {
        let excluded = if exclusion.is_some() {
            Some(true)
        } else if unreadable_rule_files.is_empty() {
            Some(false)
        } else {
            None
        };
        IgnoreEvaluation {
            excluded,
            exclusion,
            unreadable_rule_files,
            rule_diagnostics: self.diagnostics.clone(),
        }
    }
}

/// Returns the built-in exclusion for one name, which no rule file can override. Returns
/// `None` if no built-in exclusion matches the name.
fn built_in_exclusion(
    built_ins: &BuiltIns,
    name: &str,
    is_directory: bool,
) -> Option<ExclusionReason> {
    let folded = fold_case_text(name);
    let is_on = |group: &str| built_ins.built_in_ignore_groups.contains(group);
    let has_suffix = |suffixes: &BTreeSet<String>| {
        suffixes
            .iter()
            .any(|suffix| folded.ends_with(suffix.as_str()))
    };
    if is_on("system_junk")
        && (built_ins.system_junk_names.contains(&folded)
            || (!is_directory && has_suffix(&built_ins.system_junk_file_suffixes)))
    {
        return Some(ExclusionReason::SystemJunk);
    }
    if is_on("partial_downloads")
        && !is_directory
        && has_suffix(&built_ins.partial_download_suffixes)
    {
        return Some(ExclusionReason::PartialDownload);
    }
    if is_on("generated_metadata")
        && is_directory
        && (built_ins.generated_metadata_folders.contains(&folded)
            || has_suffix(&built_ins.generated_metadata_folder_suffixes))
    {
        return Some(ExclusionReason::GeneratedMetadata);
    }
    (is_on("hidden_entries") && name.starts_with('.')).then_some(ExclusionReason::HiddenEntry)
}

/// Returns `true` if a glob in gitignore syntax matches a root-relative path. A glob
/// without `/` matches a name at any depth (I1). An invalid glob matches nothing.
pub(crate) fn glob_matches(glob: &str, path: &str) -> bool {
    let ParsedLine::Pattern(pattern) = pattern::parse_line(glob, IgnoreDialect::Gitignore) else {
        return false;
    };
    let is_directory = path.ends_with('/');
    let components: Vec<&str> = path
        .trim_end_matches('/')
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    !components.is_empty() && pattern.matches(&components, is_directory)
}
