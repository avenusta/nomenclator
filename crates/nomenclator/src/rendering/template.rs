//! Provides two-way name templates (6.1, R14): a template renders facts into a name and
//! reads a name back into facts.
//!
//! Syntax: `{field}` is a field, `{field:00}` a number padded with zeros to the number
//! of zeros, and `[...]` an optional section that is left out if any field in it has no
//! value. `{{`, `}}`, `[[` and `]]` are the literal characters. A `/` separates folders.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::model::Finding;

/// The fields whose values are numbers: they read as digits and render with padding.
const NUMBER_FIELDS: &[&str] = &[
    "year",
    "series_year",
    "season",
    "episode",
    "last_episode",
    "part",
];

/// A compiled name template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    source: String,
    pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Literal(String),
    Field { name: String, width: usize },
    Optional(Vec<Piece>),
}

/// A template that cannot be compiled, with the byte offset of the fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateError {
    /// What is wrong.
    pub reason: TemplateErrorReason,
    /// The byte offset in the template text.
    pub offset: usize,
}

/// Why a template cannot be compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateErrorReason {
    /// A `{` without its `}`, or a `}` without its `{`.
    UnclosedField,
    /// A `[` without its `]`, or a `]` without its `[`.
    UnclosedSection,
    /// A field without a name, or with a name that is not a lowercase identifier.
    InvalidFieldName,
    /// A padding that is not zeros (`{season:2}`).
    InvalidPadding,
    /// Two fields next to each other, with no literal text between them to read them by.
    AdjacentFields,
}

/// The facts of a name, by field. Numbers are written without padding.
pub type TemplateFacts = BTreeMap<String, String>;

/// The result of rendering facts with a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedTemplate {
    /// The name. `None` if a field outside an optional section has no value.
    pub path: Option<String>,
    /// The fields outside optional sections that have no value, in template order.
    pub missing_fields: Vec<String>,
}

impl Template {
    /// Compiles `text` into a template. Fails if the text is not a valid template.
    pub fn parse(text: &str) -> Result<Template, TemplateError> {
        let (pieces, rest) = parse_pieces(text, 0, false)?;
        if rest != text.len() {
            return Err(TemplateError {
                reason: TemplateErrorReason::UnclosedSection,
                offset: rest,
            });
        }
        check_adjacent(&pieces, &mut None)?;
        Ok(Template {
            source: text.to_owned(),
            pieces,
        })
    }

    /// Returns the template text.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the names of the fields that the template uses.
    pub(crate) fn fields(&self) -> BTreeSet<&str> {
        fn collect<'a>(pieces: &'a [Piece], fields: &mut BTreeSet<&'a str>) {
            for piece in pieces {
                match piece {
                    Piece::Literal(_) => {}
                    Piece::Field { name, .. } => {
                        fields.insert(name);
                    }
                    Piece::Optional(inner) => collect(inner, fields),
                }
            }
        }
        let mut fields = BTreeSet::new();
        collect(&self.pieces, &mut fields);
        fields
    }

    /// Returns the names of the fields that the template uses outside optional sections.
    pub(crate) fn required_fields(&self) -> BTreeSet<&str> {
        self.pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Field { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Returns `true` if the template has an optional section or uses a field twice.
    pub(crate) fn has_optional_or_repeated_field(&self) -> bool {
        let fields = self
            .pieces
            .iter()
            .filter(|piece| matches!(piece, Piece::Field { .. }))
            .count();
        fields != self.required_fields().len()
            || self
                .pieces
                .iter()
                .any(|piece| matches!(piece, Piece::Optional(_)))
    }

    /// Returns the regular expression of an episode token (X34). A space in the template's
    /// text matches a space, `.`, `-`, `_` or nothing, as in a release word spelling (X19).
    pub(crate) fn token_pattern(&self) -> String {
        self.pieces
            .iter()
            .map(|piece| match piece {
                Piece::Literal(literal) => literal
                    .split(' ')
                    .map(regex::escape)
                    .collect::<Vec<_>>()
                    .join("[ ._-]?"),
                Piece::Field { name, .. } if name == "season" => r"(?P<season>\d{1,4})".to_owned(),
                Piece::Field { name, .. } => format!(r"(?P<{name}>\d{{1,6}})"),
                Piece::Optional(_) => String::new(),
            })
            .collect()
    }

    /// Returns the literal text of the template, with each field replaced by a space.
    pub(crate) fn literal_text(&self) -> String {
        fn collect(pieces: &[Piece], text: &mut String) {
            for piece in pieces {
                match piece {
                    Piece::Literal(literal) => text.push_str(literal),
                    Piece::Field { .. } => text.push(' '),
                    Piece::Optional(inner) => collect(inner, text),
                }
            }
        }
        let mut text = String::new();
        collect(&self.pieces, &mut text);
        text
    }

    /// Renders `facts`. A field outside an optional section that has no value is reported in
    /// `missing_fields`, and no path is rendered.
    #[must_use]
    pub fn render(&self, facts: &TemplateFacts) -> RenderedTemplate {
        let mut missing = Vec::new();
        let text = render_pieces(&self.pieces, facts, &mut missing);
        RenderedTemplate {
            path: missing.is_empty().then_some(text),
            missing_fields: missing,
        }
    }

    /// Reads a name into the facts that the template encodes. Of the readings that match
    /// the whole name, the ones that fill the most fields are kept, so an optional section is
    /// read if its text is present, and of those the ones whose fields hold the least text.
    /// Returns ambiguous if several readings remain, and unknown if none matches. A field
    /// that the template uses twice must have one value.
    pub fn read(&self, name: &str) -> Finding<TemplateFacts> {
        let mut readings: Vec<TemplateFacts> =
            match_pieces(&self.pieces, name, 0, TemplateFacts::new())
                .into_iter()
                .filter(|(end, _)| *end == name.len())
                .map(|(_, facts)| facts)
                .collect();
        readings.sort();
        readings.dedup();
        let most = readings.iter().map(BTreeMap::len).max().unwrap_or_default();
        readings.retain(|facts| facts.len() == most);
        // Of those, the readings whose fields hold the least text match the most of the
        // template's own text, as in `- part2` read as a part, not as a version label.
        let text = |facts: &TemplateFacts| facts.values().map(String::len).sum::<usize>();
        let least = readings.iter().map(text).min().unwrap_or_default();
        readings.retain(|facts| text(facts) == least);
        match readings.len() {
            0 => Finding::Unknown,
            1 => Finding::Resolved {
                value: readings.remove(0),
                evidence: None,
            },
            _ => Finding::Ambiguous {
                candidates: readings,
                reason: "several_readings",
            },
        }
    }
}

/// The most partial readings that one call keeps, which bounds the work on long names.
const MAX_READINGS: usize = 256;

/// Returns every way that `pieces` match `name` from byte `start`: the end of each match
/// and the facts it reads.
fn match_pieces(
    pieces: &[Piece],
    name: &str,
    start: usize,
    facts: TemplateFacts,
) -> Vec<(usize, TemplateFacts)> {
    let Some((piece, rest)) = pieces.split_first() else {
        return vec![(start, facts)];
    };
    let mut results = Vec::new();
    let continue_from =
        |end: usize, facts: TemplateFacts, results: &mut Vec<(usize, TemplateFacts)>| {
            if results.len() < MAX_READINGS {
                results.extend(match_pieces(rest, name, end, facts));
            }
        };
    match piece {
        Piece::Literal(literal) => {
            if name[start..].starts_with(literal.as_str()) {
                continue_from(start + literal.len(), facts, &mut results);
            }
        }
        Piece::Field { name: field, .. } => {
            let is_number = NUMBER_FIELDS.contains(&field.as_str());
            let text = &name[start..];
            let limit = text.find('/').unwrap_or(text.len());
            let mut ends: Vec<usize> = text[..limit]
                .char_indices()
                .map(|(index, character)| index + character.len_utf8())
                .collect();
            if is_number {
                let digits = text.bytes().take_while(u8::is_ascii_digit).count();
                ends.retain(|&end| end <= digits);
            }
            for end in ends {
                let raw = &text[..end];
                let value = if is_number {
                    match raw.parse::<u64>() {
                        Ok(number) => number.to_string(),
                        Err(_) => continue,
                    }
                } else {
                    raw.to_owned()
                };
                if facts.get(field).is_some_and(|existing| *existing != value) {
                    continue;
                }
                let mut next = facts.clone();
                next.insert(field.clone(), value);
                continue_from(start + end, next, &mut results);
            }
        }
        Piece::Optional(inner) => {
            for (end, inner_facts) in match_pieces(inner, name, start, facts.clone()) {
                continue_from(end, inner_facts, &mut results);
            }
            continue_from(start, facts, &mut results);
        }
    }
    results
}

fn parse_pieces(
    text: &str,
    start: usize,
    in_section: bool,
) -> Result<(Vec<Piece>, usize), TemplateError> {
    let bytes = text.as_bytes();
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut index = start;
    let flush = |literal: &mut String, pieces: &mut Vec<Piece>| {
        if !literal.is_empty() {
            pieces.push(Piece::Literal(std::mem::take(literal)));
        }
    };
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("{{")
            || rest.starts_with("}}")
            || rest.starts_with("[[")
            || rest.starts_with("]]")
        {
            literal.push(bytes[index] as char);
            index += 2;
            continue;
        }
        match bytes[index] {
            b'{' => {
                let close = rest.find('}').ok_or(TemplateError {
                    reason: TemplateErrorReason::UnclosedField,
                    offset: index,
                })?;
                let inner = &rest[1..close];
                let (name, padding) = inner.split_once(':').unwrap_or((inner, ""));
                let is_identifier = !name.is_empty()
                    && name.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit()
                    });
                if !is_identifier {
                    return Err(TemplateError {
                        reason: TemplateErrorReason::InvalidFieldName,
                        offset: index,
                    });
                }
                if !padding.bytes().all(|byte| byte == b'0') {
                    return Err(TemplateError {
                        reason: TemplateErrorReason::InvalidPadding,
                        offset: index,
                    });
                }
                flush(&mut literal, &mut pieces);
                pieces.push(Piece::Field {
                    name: name.to_owned(),
                    width: padding.len(),
                });
                index += close + 1;
            }
            b'}' => {
                return Err(TemplateError {
                    reason: TemplateErrorReason::UnclosedField,
                    offset: index,
                });
            }
            b'[' => {
                flush(&mut literal, &mut pieces);
                let (inner, end) = parse_pieces(text, index + 1, true)?;
                if end >= text.len() || bytes[end] != b']' {
                    return Err(TemplateError {
                        reason: TemplateErrorReason::UnclosedSection,
                        offset: index,
                    });
                }
                pieces.push(Piece::Optional(inner));
                index = end + 1;
            }
            b']' if in_section => {
                flush(&mut literal, &mut pieces);
                return Ok((pieces, index));
            }
            b']' => {
                return Err(TemplateError {
                    reason: TemplateErrorReason::UnclosedSection,
                    offset: index,
                });
            }
            _ => {
                let character = rest.chars().next().expect("a character");
                literal.push(character);
                index += character.len_utf8();
            }
        }
    }
    flush(&mut literal, &mut pieces);
    if in_section {
        return Err(TemplateError {
            reason: TemplateErrorReason::UnclosedSection,
            offset: start.saturating_sub(1),
        });
    }
    Ok((pieces, index))
}

/// Fails if two fields meet without literal text between them, in the order the
/// template reads, because no reading could tell where one ends.
fn check_adjacent(
    pieces: &[Piece],
    previous_was_field: &mut Option<bool>,
) -> Result<(), TemplateError> {
    for piece in pieces {
        match piece {
            Piece::Literal(_) => *previous_was_field = Some(false),
            Piece::Field { .. } => {
                if *previous_was_field == Some(true) {
                    return Err(TemplateError {
                        reason: TemplateErrorReason::AdjacentFields,
                        offset: 0,
                    });
                }
                *previous_was_field = Some(true);
            }
            Piece::Optional(inner) => check_adjacent(inner, previous_was_field)?,
        }
    }
    Ok(())
}

fn render_pieces(pieces: &[Piece], facts: &TemplateFacts, missing: &mut Vec<String>) -> String {
    let mut text = String::new();
    for piece in pieces {
        match piece {
            Piece::Literal(literal) => text.push_str(literal),
            Piece::Field { name, width } => match facts.get(name).filter(|value| !value.is_empty())
            {
                Some(value) => text.push_str(&padded(value, *width)),
                None => missing.push(name.clone()),
            },
            Piece::Optional(inner) => {
                let mut inner_missing = Vec::new();
                let rendered = render_pieces(inner, facts, &mut inner_missing);
                if inner_missing.is_empty() {
                    text.push_str(&rendered);
                }
            }
        }
    }
    text
}

fn padded(value: &str, width: usize) -> String {
    if width > 0 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{value:0>width$}")
    } else {
        value.to_owned()
    }
}
