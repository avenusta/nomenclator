//! Runs the YAML cases in `corpus/` against the library. See `corpus/README.md` for the format.

use nomenclator::Rules;
#[path = "support/cases.rs"]
mod cases;
#[path = "support/expectations.rs"]
mod expectations;

use cases::{CorpusCase, collect_yaml_files, corpus_directory, load_cases};
use expectations::expect_subset;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Fails if a non-pending case fails or if a pending case now passes. `just promote`
/// removes the pending mark from cases that pass. Prints the pending progress per file.
#[test]
fn corpus_cases_match_expected_facts() {
    let promote = std::env::var_os("PROMOTE").is_some();
    let show_pending_in = std::env::var("PENDING").ok();
    let mut failures = Vec::new();
    let mut now_passing: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    let mut pending_passed_and_total: BTreeMap<PathBuf, (usize, usize)> = BTreeMap::new();
    let rules = Rules::builtin();

    for file in collect_yaml_files(&corpus_directory()) {
        for (index, case) in load_cases(&file).into_iter().enumerate() {
            let outcome = case_rules(&case)
                .and_then(|case_rules| run_case(case_rules.as_ref().unwrap_or(rules), &case));
            if !case.pending {
                if let Err(reason) = outcome {
                    failures.push(format!("{} › {}: {reason}", file.display(), case.name));
                }
                continue;
            }
            let counts = pending_passed_and_total.entry(file.clone()).or_default();
            counts.1 += 1;
            match outcome {
                Ok(()) => {
                    counts.0 += 1;
                    now_passing.entry(file.clone()).or_default().push(index);
                }
                Err(reason)
                    if show_pending_in
                        .as_deref()
                        .is_some_and(|wanted| file.to_string_lossy().contains(wanted)) =>
                {
                    println!("PENDING {} › {}: {reason}", file.display(), case.name);
                }
                Err(_) => {}
            }
        }
    }

    for (file, (passed, total)) in &pending_passed_and_total {
        let relative = file.strip_prefix(corpus_directory()).unwrap_or(file);
        println!(
            "{passed:>5}/{total:<5} pending now pass  {}",
            relative.display()
        );
    }
    if promote {
        for (file, indexes) in &now_passing {
            remove_pending_marks(file, indexes);
        }
        now_passing.clear();
    }
    let promotable: usize = now_passing.values().map(Vec::len).sum();
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        promotable == 0,
        "{promotable} pending cases now pass; run `just promote` to remove their mark"
    );
}

/// Builds the rules from a case's overlays and languages. A case without `languages`
/// declares none, which selects every language. Returns `None` if the case uses the
/// built-in rules.
fn case_rules(case: &CorpusCase) -> Result<Option<Rules>, String> {
    if case.uses_builtin_rules() {
        return Ok(None);
    }
    Rules::from_documents(
        case.languages.as_deref().unwrap_or_default(),
        &case.overlays,
    )
    .map(Some)
    .map_err(|diagnostics| format!("overlays rejected: {diagnostics:?}"))
}

fn run_case(rules: &Rules, case: &CorpusCase) -> Result<(), String> {
    let actual = rules
        .run(&case.operation, case.input.clone())
        .map_err(|error| error.to_string())?;
    expect_subset(&case.expect, &actual, "expect")
}

/// Checks that `corpus/README.md` documents exactly the operations that `Rules::run` runs.
#[test]
fn corpus_readme_lists_every_operation() {
    let readme = std::fs::read_to_string(corpus_directory().join("README.md"))
        .expect("corpus/README.md is readable");
    let documented: Vec<&str> = readme
        .lines()
        .filter(|line| line.starts_with("| `"))
        .flat_map(|line| line.split(" | ").next())
        .flat_map(|cell| cell.split('`').skip(1).step_by(2))
        .collect();
    let mut documented: Vec<&str> = documented;
    documented.sort_unstable();
    let mut operations: Vec<&str> = Rules::operations().collect();
    operations.sort_unstable();
    assert_eq!(documented, operations);
}

/// Drops `pending: true` from the cases at `indexes`. Each case starts with `- ` at column 0.
fn remove_pending_marks(file: &Path, indexes: &[usize]) {
    let text = std::fs::read_to_string(file).expect("corpus file is readable");
    let mut case_index = None;
    let kept: Vec<&str> = text
        .split_inclusive('\n')
        .filter(|line| {
            if line.starts_with("- ") {
                case_index = Some(case_index.map_or(0, |index| index + 1));
            }
            let is_mark = line.trim_end() == "  pending: true";
            !(is_mark && case_index.is_some_and(|index| indexes.contains(&index)))
        })
        .collect();
    std::fs::write(file, kept.concat()).expect("corpus file is writable");
}
