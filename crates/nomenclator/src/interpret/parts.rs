//! Groups ordered parts: sibling files or folders that together make one representation.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::lexical;
use crate::model::EntryKind;
use crate::rules::Rules;

/// Entries that are ordered parts of one representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stack {
    /// The shared name before the part marker.
    pub name: String,
    /// The parts in order.
    pub paths: Vec<String>,
    /// `true` if the parts are not contiguous from 1 or `a`.
    pub ambiguous: bool,
    /// `missing_part` or `duplicate_part` if the stack is ambiguous.
    pub ambiguity_reason: Option<&'static str>,
}

/// The stacks among the supplied paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stacks {
    /// The stacks, sorted by name, then by paths.
    pub stacks: Vec<Stack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PartIndex {
    Number(u32),
    Letter(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StackKey {
    name: String,
    comparable_name: String,
    parent: String,
    is_directory: bool,
    is_lettered: bool,
}

/// Groups root-relative paths (directories end in `/`) whose names differ only by a
/// trailing part marker, under the built-in rules. Numbers and letters are not mixed in one
/// stack. A single part is not a stack.
///
/// Uses the built-in rules. [`Rules::group_parts`] takes other rules.
#[must_use]
pub fn group_parts(paths: &[String]) -> Stacks {
    Rules::builtin().group_parts(paths)
}

impl Rules {
    /// Groups root-relative paths (directories end in `/`) whose names differ only by a
    /// trailing part marker. Numbers and letters are not mixed in one stack. A single part
    /// is not a stack.
    pub fn group_parts(&self, paths: &[String]) -> Stacks {
        group_paths(self, paths)
    }
}

fn group_paths(rules: &Rules, paths: &[String]) -> Stacks {
    let mut groups: BTreeMap<StackKey, Vec<(PartIndex, String)>> = BTreeMap::new();
    for path in paths {
        let is_directory = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
        let kind = if is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        let Some((stack_name, index)) = part_marker(rules, lexical::stem(name, kind)) else {
            continue;
        };
        let key = StackKey {
            comparable_name: lexical::vocabulary_form(&stack_name),
            name: stack_name,
            parent: parent.to_owned(),
            is_directory,
            is_lettered: matches!(index, PartIndex::Letter(_)),
        };
        groups.entry(key).or_default().push((index, path.clone()));
    }
    let mut stacks: Vec<Stack> = groups
        .into_iter()
        .filter(|(_, parts)| parts.len() >= 2)
        .map(|(key, mut parts)| {
            parts.sort();
            let ambiguity_reason = numbering_problem(&parts);
            Stack {
                name: key.name,
                paths: parts.into_iter().map(|(_, path)| path).collect(),
                ambiguous: ambiguity_reason.is_some(),
                ambiguity_reason,
            }
        })
        .collect();
    stacks.sort_by(|left, right| (&left.name, &left.paths).cmp(&(&right.name, &right.paths)));
    Stacks { stacks }
}

/// Returns the name before a trailing part marker, and the marker's position. Returns
/// `None` if the stem has no part marker.
fn part_marker(rules: &Rules, stem: &str) -> Option<(String, PartIndex)> {
    let captures = rules.patterns.part_marker.captures(stem)?;
    let prefix = captures.get(1).map_or("", |prefix| prefix.as_str());
    let marker = lexical::vocabulary_form(&captures[2]);
    let index = match marker.parse() {
        Ok(number) => PartIndex::Number(number),
        Err(_) => PartIndex::Letter(u32::from(marker.as_bytes()[0] - b'a') + 1),
    };
    let name = prefix.trim_end_matches([' ', '.', '_', '-', '(', '[']);
    Some((name.to_owned(), index))
}

/// Checks that parts are numbered 1, 2, 3, … without gaps or repeats. Returns
/// `duplicate_part` if a number repeats. Returns `missing_part` if a number is missing.
/// Returns `None` otherwise.
fn numbering_problem(sorted_parts: &[(PartIndex, String)]) -> Option<&'static str> {
    let positions: Vec<u32> = sorted_parts
        .iter()
        .map(|(index, _)| match index {
            PartIndex::Number(position) | PartIndex::Letter(position) => *position,
        })
        .collect();
    if positions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Some(crate::model::DUPLICATE_PART);
    }
    let is_contiguous_from_one = positions
        .iter()
        .enumerate()
        .all(|(offset, &position)| position as usize == offset + 1);
    (!is_contiguous_from_one).then_some(crate::model::MISSING_PART)
}
