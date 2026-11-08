//! Interprets one scope of a library, and merges the interpretations of scopes into the
//! interpretation of the whole library (8.1, X23).
//!
//! A scope is a folder at the library root with every entry below it, or the files at the
//! library root. The names in one scope never change the interpretation of another scope.
//! Only an override joins scopes: a `merge` group or an extra's `owner` in another scope.
//! A scope group is the scopes that the overrides join, and a caller re-interprets only
//! the groups whose entries changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::ignore::IgnoreFile;
use crate::inputs::{CallerInputs, normalized};
use crate::parsers::extras::LibraryType;
use crate::rules::Rules;

use super::{Interpretation, input_diagnostics};

/// Scopes that are interpreted together. `None` is the files at the library root.
pub type ScopeGroup = BTreeSet<Option<String>>;

impl Rules {
    /// Returns the scope of a root-relative path: its folder at the library root, or `None`
    /// for a file at the library root. A folder at the root (`Show/`) is in its own scope.
    #[must_use]
    pub fn scope_of(path: &str) -> Option<&str> {
        let (first, _) = path.split_once('/')?;
        Some(first)
    }

    /// Returns the scope groups of a library: the scopes of `paths`, joined where an
    /// override of `inputs` links them (X23). Each scope is in one group. The groups are
    /// sorted, and the root scope (`None`) sorts first.
    #[must_use]
    pub fn scope_groups(paths: &[String], inputs: &CallerInputs) -> Vec<ScopeGroup> {
        let scopes: BTreeSet<Option<String>> = paths
            .iter()
            .map(|path| Rules::scope_of(path).map(str::to_owned))
            .collect();
        let mut groups: Vec<ScopeGroup> = scopes
            .iter()
            .map(|scope| BTreeSet::from([scope.clone()]))
            .collect();
        let scope_of_input = |path: &str| {
            let path = normalized(path);
            Some(Rules::scope_of(&path).map(str::to_owned)).filter(|scope| scopes.contains(scope))
        };
        let folder_scope = |folder: &str| {
            let folder = format!("{}/", normalized(folder));
            Some(Rules::scope_of(&folder).map(str::to_owned)).filter(|scope| scopes.contains(scope))
        };
        let mut links: Vec<Vec<Option<String>>> = inputs
            .overrides
            .merge
            .iter()
            .map(|group| {
                group
                    .iter()
                    .filter_map(|path| scope_of_input(path))
                    .collect()
            })
            .collect();
        links.extend(inputs.overrides.extras.iter().filter_map(|extra| {
            let owner = folder_scope(extra.owner.as_deref()?)?;
            Some(vec![scope_of_input(&extra.path)?, owner])
        }));
        for link in links {
            let (joined, rest): (Vec<ScopeGroup>, Vec<ScopeGroup>) = groups
                .into_iter()
                .partition(|group| link.iter().any(|scope| group.contains(scope)));
            groups = rest;
            groups.push(joined.into_iter().flatten().collect());
        }
        groups.retain(|group| !group.is_empty());
        groups.sort();
        groups
    }

    /// Interprets the entries of `paths` that are in the scopes of `group` (X23). The
    /// ignore files that apply are those at the library root and those inside the group.
    /// The result has no input diagnostics, because an input for another group is not
    /// unused; [`Rules::merge_scopes`] reports them for the whole library.
    pub fn interpret_scopes(
        &self,
        library: LibraryType,
        group: &ScopeGroup,
        paths: &[String],
        ignore_files: &[IgnoreFile],
        inputs: &CallerInputs,
    ) -> Interpretation {
        let in_scope = |path: &str| group.contains(&Rules::scope_of(path).map(str::to_owned));
        let scope_paths: Vec<String> = paths
            .iter()
            .filter(|path| in_scope(path))
            .cloned()
            .collect();
        let scope_ignore_files: Vec<IgnoreFile> = ignore_files
            .iter()
            .filter(|file| !file.path.contains('/') || in_scope(&file.path))
            .cloned()
            .collect();
        let mut interpretation =
            self.interpret_with(library, &scope_paths, &scope_ignore_files, inputs);
        interpretation.input_diagnostics.clear();
        interpretation
    }

    /// Merges the interpretations of the scope groups of a library into the interpretation
    /// of the whole library (X23). `parts` holds one interpretation per scope group. `paths` holds
    /// every entry of the library and `inputs` the caller inputs, for the input
    /// diagnostics. The result equals the interpretation of the whole library.
    pub fn merge_scopes(
        &self,
        parts: Vec<Interpretation>,
        paths: &[String],
        inputs: &CallerInputs,
    ) -> Interpretation {
        let mut keyed_items = Vec::new();
        let mut merged = Interpretation {
            items: Vec::new(),
            item_keys: Vec::new(),
            extras_of: BTreeMap::new(),
            video_subtitles: BTreeMap::new(),
            excluded_entries: Vec::new(),
            undecided_entries: Vec::new(),
            invalid_entries: Vec::new(),
            unclaimed_files: Vec::new(),
            rule_diagnostics: Vec::new(),
            release_junk: Vec::new(),
            input_diagnostics: Vec::new(),
            rules: self.identity().clone(),
        };
        for part in parts {
            keyed_items.extend(part.item_keys.into_iter().zip(part.items));
            merged.extras_of.extend(part.extras_of);
            merged.video_subtitles.extend(part.video_subtitles);
            merged.excluded_entries.extend(part.excluded_entries);
            merged.undecided_entries.extend(part.undecided_entries);
            merged.invalid_entries.extend(part.invalid_entries);
            merged.unclaimed_files.extend(part.unclaimed_files);
            merged.release_junk.extend(part.release_junk);
            for diagnostic in part.rule_diagnostics {
                // Every scope reads the ignore files at the library root.
                if !merged.rule_diagnostics.contains(&diagnostic) {
                    merged.rule_diagnostics.push(diagnostic);
                }
            }
        }
        // The sort is stable, so the items of one block keep their order.
        keyed_items.sort_by(|left, right| left.0.cmp(&right.0));
        (merged.item_keys, merged.items) = keyed_items.into_iter().unzip();
        merged
            .excluded_entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        merged
            .undecided_entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        merged
            .invalid_entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        merged.unclaimed_files.sort();
        merged.release_junk.sort();
        merged.input_diagnostics = input_diagnostics(self, inputs, paths, &merged.items);
        merged
    }
}
