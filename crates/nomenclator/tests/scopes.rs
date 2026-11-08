//! Checks that scoped and full interpretation agree (8.1, X23): for every library tree in
//! the corpus, interpreting each scope group on its own and merging the results gives the
//! interpretation of the whole tree.

use std::collections::BTreeSet;

use nomenclator::{CallerInputs, IgnoreFile, LibraryType, Rules};
use serde::Deserialize;
use serde_json::Value;

#[path = "support/cases.rs"]
mod cases;

use cases::{collect_yaml_files, corpus_directory, load_cases};

/// A library tree of one corpus case.
#[derive(Deserialize)]
struct Tree {
    library: LibraryType,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    ignore_files: Vec<IgnoreFile>,
    #[serde(default)]
    inputs: CallerInputs,
}

/// The operations whose input is one library tree.
const TREE_OPERATIONS: &[&str] = &["interpret", "plan_renames", "validate", "end_to_end"];

/// Returns the name and tree of every case that interprets a tree under the built-in
/// rules.
fn corpus_trees() -> Vec<(String, Tree)> {
    let mut trees = Vec::new();
    for file in collect_yaml_files(&corpus_directory()) {
        for case in load_cases(&file) {
            if !TREE_OPERATIONS.contains(&case.operation.as_str()) || !case.uses_builtin_rules() {
                continue;
            }
            let mut input = case.input.clone();
            // Only the tree's fields are read; a profile or a reference is not a tree fact.
            if let Value::Object(fields) = &mut input {
                fields.retain(|key, _| {
                    matches!(
                        key.as_str(),
                        "library" | "paths" | "ignore_files" | "inputs"
                    )
                });
            }
            if let Ok(tree) = serde_json::from_value::<Tree>(input) {
                trees.push((format!("{} › {}", file.display(), case.name), tree));
            }
        }
    }
    trees
}

#[test]
fn merge_scopes_equals_full_interpretation() {
    let rules = Rules::builtin();
    let trees = corpus_trees();
    assert!(trees.len() > 400, "the corpus has {} trees", trees.len());
    let mut failures = Vec::new();
    for (name, tree) in &trees {
        let full =
            rules.interpret_with(tree.library, &tree.paths, &tree.ignore_files, &tree.inputs);
        let groups = Rules::scope_groups(&tree.paths, &tree.inputs);
        let scopes: BTreeSet<Option<&str>> = tree
            .paths
            .iter()
            .map(|path| Rules::scope_of(path))
            .collect();
        let grouped: usize = groups.iter().map(BTreeSet::len).sum();
        assert_eq!(grouped, scopes.len(), "{name}: each scope is in one group");
        let parts = groups
            .iter()
            .map(|group| {
                rules.interpret_scopes(
                    tree.library,
                    group,
                    &tree.paths,
                    &tree.ignore_files,
                    &tree.inputs,
                )
            })
            .collect();
        let merged = rules.merge_scopes(parts, &tree.paths, &tree.inputs);
        if merged != full {
            let full = serde_json::to_value(&full).expect("serializable");
            let merged = serde_json::to_value(&merged).expect("serializable");
            let Value::Object(full_fields) = &full else {
                panic!("an interpretation is a mapping");
            };
            let mut fields: Vec<String> = full_fields
                .iter()
                .filter(|(field, value)| merged.get(*field) != Some(*value))
                .map(|(field, _)| field.clone())
                .collect();
            if fields.is_empty() {
                fields.push("item_keys".to_owned());
            }
            failures.push(format!("{name}: {}", fields.join(", ")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} trees differ:\n{}",
        failures.len(),
        trees.len(),
        failures.join("\n")
    );
}

#[test]
fn merge_scopes_equals_full_interpretation_of_combined_trees() {
    let rules = Rules::builtin();
    let trees = corpus_trees();
    let mut failures = Vec::new();
    let mut combined = 0;
    for library in [LibraryType::Movies, LibraryType::Tv] {
        let of_library: Vec<&Tree> = trees
            .iter()
            .map(|(_, tree)| tree)
            .filter(|tree| tree.library == library)
            .collect();
        // Each combined tree joins the paths of eight cases, so their scopes and their
        // root files meet in one library.
        for (index, chunk) in of_library.chunks(8).enumerate() {
            let mut paths: Vec<String> = chunk.iter().flat_map(|tree| tree.paths.clone()).collect();
            paths.sort();
            paths.dedup();
            let inputs = CallerInputs::default();
            let full = rules.interpret_with(library, &paths, &[], &inputs);
            let parts = Rules::scope_groups(&paths, &inputs)
                .iter()
                .map(|group| rules.interpret_scopes(library, group, &paths, &[], &inputs))
                .collect();
            if rules.merge_scopes(parts, &paths, &inputs) != full {
                failures.push(format!("{library:?} chunk {index}"));
            }
            combined += 1;
        }
    }
    assert!(combined > 40, "{combined} combined trees");
    assert!(
        failures.is_empty(),
        "combined trees differ: {}",
        failures.join(", ")
    );
}
