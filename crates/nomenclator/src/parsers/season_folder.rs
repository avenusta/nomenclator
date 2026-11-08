use std::collections::BTreeMap;

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::lexical::{self, TokenKind};
use crate::model::{Evidence, Finding, Rule, Span};
use crate::parsers::series_name::{
    SeasonMarkerOptions, TITLE_EDGE_CHARACTERS, find_season_marker, is_valid_season, title_parts,
};
use crate::rules::Rules;

/// The season that a folder name states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonFolder {
    /// The season. 0 for a specials folder.
    pub season: Finding<u16>,
    /// `true` if the name is a season word alone (`Season`, `Staffel`), with no number.
    pub is_bare_season_folder: bool,
    /// Provider IDs from the bracketed tags, keyed by provider.
    pub provider_ids: BTreeMap<String, Finding<String>>,
}

/// Reads the season that a folder name states (T9). `parent_name` is the folder that
/// contains it. `parent_is_series_folder` is `true` if the caller knows that the parent is a
/// series folder. Only then is a bare number (`2`) or a word such as `Series 2` read as a
/// season. Text before the season marker must be the series title of the parent
/// (`Show.S01.1080p` under `Show`).
///
/// Uses the built-in rules. [`Rules::parse_season_folder`] takes other rules.
#[must_use]
pub fn parse_season_folder(
    name: &str,
    parent_name: Option<&str>,
    parent_is_series_folder: bool,
) -> SeasonFolder {
    Rules::builtin().parse_season_folder(name, parent_name, parent_is_series_folder)
}

impl Rules {
    /// Reads the season that a folder name states (T9). `parent_name` is the folder that
    /// contains it. `parent_is_series_folder` is `true` if the caller knows that the parent is a
    /// series folder. Only then is a bare number (`2`) or a word such as `Series 2` read as a
    /// season. Text before the season marker must be the series title of the parent
    /// (`Show.S01.1080p` under `Show`).
    pub fn parse_season_folder(
        &self,
        name: &str,
        parent_name: Option<&str>,
        parent_is_series_folder: bool,
    ) -> SeasonFolder {
        let rules = self;
        let original = name;
        let name = unwrap_single_bracket_group(rules, name.trim());
        let offset = name.as_ptr() as usize - original.as_ptr() as usize;
        let folded = lexical::vocabulary_form(name);
        let season = match season(rules, name, &folded, parent_name, parent_is_series_folder) {
            Finding::Resolved {
                value,
                evidence: Some(evidence),
            } => Finding::Resolved {
                value,
                evidence: Some(Evidence {
                    rule: evidence.rule,
                    span: Span(evidence.span.0 + offset, evidence.span.1 + offset),
                }),
            },
            other => other,
        };
        SeasonFolder {
            season,
            is_bare_season_folder: rules
                .season_words
                .contains(&folded.nfc().collect::<String>()),
            provider_ids: lexical::provider_ids(rules, name, true),
        }
    }
}

fn season(
    rules: &Rules,
    name: &str,
    folded: &str,
    parent_name: Option<&str>,
    parent_is_series_folder: bool,
) -> Finding<u16> {
    let whole_name = Span(0, name.len());
    // The vocabulary holds NFC entries, and a filesystem can return decomposed names.
    let composed: String = folded.nfc().collect();
    if rules.specials_folder_names.contains(&composed) {
        return resolved(0, Rule::SpecialsFolder, whole_name);
    }
    if !name.is_empty() && name.len() <= 4 && name.bytes().all(|byte| byte.is_ascii_digit()) {
        let number: u32 = name.parse().expect("digits parse");
        return if parent_is_series_folder && is_valid_season(rules, number) {
            resolved(number as u16, Rule::BareNumberSeasonFolder, whole_name)
        } else {
            Finding::Unknown
        };
    }
    let Some(marker) = find_season_marker(rules, name, SeasonMarkerOptions::IN_SEASON_FOLDER)
    else {
        return Finding::Unknown;
    };
    if marker.uses_series_word && !parent_is_series_folder {
        return Finding::Unknown;
    }
    let text_before = &name[..marker.span.0];
    if !text_before.trim_matches(TITLE_EDGE_CHARACTERS).is_empty() {
        let names_parent = parent_name.is_some_and(|parent| {
            let parent_title = title_parts(rules, parent, 0);
            let prefix_title = title_parts(rules, text_before, 0);
            match (parent_title.title_value(), prefix_title.title_value()) {
                (Some(parent), Some(prefix)) => {
                    lexical::title_key(rules, parent) == lexical::title_key(rules, prefix)
                }
                _ => false,
            }
        });
        if !names_parent {
            return Finding::Unknown;
        }
    }
    resolved(marker.season, Rule::SeasonMarker, marker.span)
}

fn resolved(value: u16, rule: Rule, span: Span) -> Finding<u16> {
    Finding::Resolved {
        value,
        evidence: Some(Evidence { rule, span }),
    }
}

/// Returns the contents of a name that is a single bracket group, so `[Show.S04.1080p]` is
/// read as `Show.S04.1080p`. Returns `name` unchanged otherwise.
fn unwrap_single_bracket_group<'a>(rules: &Rules, name: &'a str) -> &'a str {
    match lexical::tokenize(rules, name).as_slice() {
        [token] => match token.kind {
            TokenKind::Bracket { inner } if token.span == Span(0, name.len()) => {
                &name[inner.0..inner.1]
            }
            _ => name,
        },
        _ => name,
    }
}
