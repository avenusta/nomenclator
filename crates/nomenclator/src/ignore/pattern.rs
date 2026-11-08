//! Parses one line of an ignore file, in gitignore syntax or as a plain glob (X14), into a
//! pattern, or rejects the line with a reason. Matching folds case and does not access the
//! filesystem.

use super::{IgnoreDialect, RuleDiagnosticReason};
use crate::lexical::fold_case;

#[derive(Debug, Clone)]
pub(super) enum ParsedLine {
    /// Blank or a comment.
    Blank,
    Pattern(Pattern),
    Invalid(RuleDiagnosticReason),
}

#[derive(Debug, Clone)]
pub(super) struct Pattern {
    /// `true` for a `!` line, which re-includes the entries it matches.
    pub is_negation: bool,
    /// `true` for a trailing `/`. The pattern then matches folders only.
    matches_directories_only: bool,
    /// The segments, matched against the whole path below the rule file's folder. A pattern
    /// without an inner or leading `/` is unanchored and matches a name at any depth.
    segments: Vec<Segment>,
}

/// One `/`-separated part of a pattern.
#[derive(Debug, Clone)]
enum Segment {
    /// `**`: any number of folders (at least one when it ends the pattern).
    AnyDepth,
    Glob(Vec<GlobToken>),
}

#[derive(Debug, Clone)]
enum GlobToken {
    /// A case-folded character.
    Literal(char),
    /// `?`
    AnyCharacter,
    /// `*`
    AnyRun,
    /// `[…]`, `[!…]` or `[^…]`
    Class {
        is_negated: bool,
        members: Vec<ClassMember>,
    },
}

#[derive(Debug, Clone)]
enum ClassMember {
    Character(char),
    Range(char, char),
    /// `[:digit:]` and the other POSIX classes, ASCII only, tested on the raw character.
    Named(CharacterTest),
}

type CharacterTest = fn(&char) -> bool;

/// A line element before it is grouped into segments.
enum Element {
    Separator,
    Star,
    Token(GlobToken),
}

/// Parses one line of a rule file in `dialect`. Lines are trimmed of surrounding whitespace.
/// In gitignore syntax, a backslash keeps a trailing space. A glob line has no negation and
/// no escapes, so `!` and `\` are part of the name.
pub(super) fn parse_line(line: &str, dialect: IgnoreDialect) -> ParsedLine {
    let is_glob = dialect == IgnoreDialect::Glob;
    let text = if is_glob {
        line.trim()
    } else {
        trim_line(line)
    };
    if text.is_empty() || text.starts_with('#') {
        return ParsedLine::Blank;
    }
    let (is_negation, text) = match text.strip_prefix('!') {
        Some(rest) if !is_glob => (true, rest),
        _ => (false, text),
    };
    let elements = match parse_elements(text, is_glob) {
        Ok(elements) => elements,
        Err(reason) => return ParsedLine::Invalid(reason),
    };
    let mut raw_segments: Vec<Vec<Element>> = vec![Vec::new()];
    for element in elements {
        match element {
            Element::Separator => raw_segments.push(Vec::new()),
            other => raw_segments
                .last_mut()
                .expect("starts non-empty")
                .push(other),
        }
    }
    let has_leading_slash = raw_segments.len() > 1 && raw_segments[0].is_empty();
    let matches_directories_only =
        raw_segments.len() > 1 && raw_segments.last().is_some_and(Vec::is_empty);
    let raw_segments: Vec<Vec<Element>> = raw_segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .collect();
    if raw_segments.is_empty() {
        return ParsedLine::Invalid(RuleDiagnosticReason::EmptyPattern);
    }
    let is_anchored = has_leading_slash || raw_segments.len() > 1;
    let mut segments: Vec<Segment> = raw_segments.into_iter().map(segment_from).collect();
    if !is_anchored {
        segments.insert(0, Segment::AnyDepth);
    }
    ParsedLine::Pattern(Pattern {
        is_negation,
        matches_directories_only,
        segments,
    })
}

impl Pattern {
    /// Returns `true` if the pattern matches an entry, given the entry's path components below
    /// the rule file's folder (at least one).
    pub(super) fn matches(&self, relative_components: &[&str], is_directory: bool) -> bool {
        if self.matches_directories_only && !is_directory {
            return false;
        }
        let component_characters: Vec<Vec<char>> = relative_components
            .iter()
            .map(|component| component.chars().collect())
            .collect();
        segments_match(&self.segments, &component_characters)
    }
}

fn trim_line(line: &str) -> &str {
    let start_trimmed = line.trim_start();
    let trimmed = start_trimmed.trim_end();
    let trailing_backslashes = trimmed.chars().rev().take_while(|&c| c == '\\').count();
    let keeps_escaped_whitespace =
        trailing_backslashes % 2 == 1 && trimmed.len() < start_trimmed.len();
    if keeps_escaped_whitespace {
        let escaped_length = start_trimmed[trimmed.len()..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        &start_trimmed[..trimmed.len() + escaped_length]
    } else {
        trimmed
    }
}

fn parse_elements(text: &str, is_glob: bool) -> Result<Vec<Element>, RuleDiagnosticReason> {
    let characters: Vec<char> = text.chars().collect();
    let mut elements = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let element = match characters[index] {
            '\\' if !is_glob => {
                index += 1;
                let escaped = characters
                    .get(index)
                    .ok_or(RuleDiagnosticReason::TrailingBackslash)?;
                Element::Token(GlobToken::Literal(fold_case(*escaped)))
            }
            '/' => Element::Separator,
            '*' => Element::Star,
            '?' => Element::Token(GlobToken::AnyCharacter),
            '[' => {
                let (class, next_index) = parse_class(&characters, index + 1)?;
                index = next_index;
                elements.push(Element::Token(class));
                continue;
            }
            other => Element::Token(GlobToken::Literal(fold_case(other))),
        };
        elements.push(element);
        index += 1;
    }
    Ok(elements)
}

/// Parses a bracket class whose contents start at `start`. Returns the class and the index
/// after `]`. Fails with the reason if the class is invalid.
fn parse_class(
    characters: &[char],
    start: usize,
) -> Result<(GlobToken, usize), RuleDiagnosticReason> {
    let mut index = start;
    let is_negated = matches!(characters.get(index), Some('!' | '^'));
    if is_negated {
        index += 1;
    }
    let first_member_index = index;
    let mut members = Vec::new();
    loop {
        let character = *characters
            .get(index)
            .ok_or(RuleDiagnosticReason::UnclosedBracket)?;
        if character == ']' && index > first_member_index {
            return Ok((
                GlobToken::Class {
                    is_negated,
                    members,
                },
                index + 1,
            ));
        }
        if character == '[' && characters.get(index + 1) == Some(&':') {
            let (named, next_index) = parse_named_class(characters, index + 2)?;
            members.push(ClassMember::Named(named));
            index = next_index;
            continue;
        }
        let (low, after_low) = class_character(characters, index)?;
        let is_range = characters.get(after_low) == Some(&'-')
            && characters
                .get(after_low + 1)
                .is_some_and(|&next| next != ']');
        if is_range {
            let (high, after_high) = class_character(characters, after_low + 1)?;
            members.push(ClassMember::Range(low, high));
            index = after_high;
        } else {
            members.push(ClassMember::Character(low));
            index = after_low;
        }
    }
}

/// Reads one possibly escaped character inside a class. Returns the character and the index
/// after it. Fails with `UnclosedBracket` if the class ends first.
fn class_character(
    characters: &[char],
    index: usize,
) -> Result<(char, usize), RuleDiagnosticReason> {
    match characters.get(index) {
        Some('\\') => characters
            .get(index + 1)
            .map(|&escaped| (escaped, index + 2))
            .ok_or(RuleDiagnosticReason::UnclosedBracket),
        Some(&character) => Ok((character, index + 1)),
        None => Err(RuleDiagnosticReason::UnclosedBracket),
    }
}

/// Parses a `[:name:]` whose name starts at `start`. Returns its test and the index after
/// `:]`.
fn parse_named_class(
    characters: &[char],
    start: usize,
) -> Result<(CharacterTest, usize), RuleDiagnosticReason> {
    let name_length = characters[start..]
        .windows(2)
        .position(|pair| pair == [':', ']'])
        .ok_or(RuleDiagnosticReason::UnknownCharacterClass)?;
    let name: String = characters[start..start + name_length].iter().collect();
    let test: CharacterTest = match name.as_str() {
        "alnum" => char::is_ascii_alphanumeric,
        "alpha" => char::is_ascii_alphabetic,
        "blank" => |&c| c == ' ' || c == '\t',
        "cntrl" => char::is_ascii_control,
        "digit" => char::is_ascii_digit,
        "graph" => char::is_ascii_graphic,
        "lower" => char::is_ascii_lowercase,
        "print" => |&c| c.is_ascii_graphic() || c == ' ',
        "punct" => char::is_ascii_punctuation,
        "space" => |&c| c.is_ascii_whitespace() || c == '\x0b',
        "upper" => char::is_ascii_uppercase,
        "xdigit" => char::is_ascii_hexdigit,
        _ => return Err(RuleDiagnosticReason::UnknownCharacterClass),
    };
    Ok((test, start + name_length + 2))
}

/// Builds a segment from its elements. `**` alone matches any depth. Any other run of stars
/// is one `*`.
fn segment_from(elements: Vec<Element>) -> Segment {
    if elements.len() == 2
        && elements
            .iter()
            .all(|element| matches!(element, Element::Star))
    {
        return Segment::AnyDepth;
    }
    let mut tokens: Vec<GlobToken> = Vec::new();
    for element in elements {
        let token = match element {
            Element::Star if matches!(tokens.last(), Some(GlobToken::AnyRun)) => continue,
            Element::Star => GlobToken::AnyRun,
            Element::Token(token) => token,
            Element::Separator => unreachable!("segments are split on separators"),
        };
        tokens.push(token);
    }
    Segment::Glob(tokens)
}

fn segments_match(segments: &[Segment], components: &[Vec<char>]) -> bool {
    match segments.split_first() {
        None => components.is_empty(),
        Some((Segment::AnyDepth, [])) => !components.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=components.len()).any(|skipped| segments_match(rest, &components[skipped..]))
        }
        Some((Segment::Glob(tokens), rest)) => {
            components.split_first().is_some_and(|(first, remaining)| {
                glob_matches(tokens, first) && segments_match(rest, remaining)
            })
        }
    }
}

fn glob_matches(tokens: &[GlobToken], name: &[char]) -> bool {
    match tokens.split_first() {
        None => name.is_empty(),
        Some((GlobToken::AnyRun, rest)) => {
            (0..=name.len()).any(|skipped| glob_matches(rest, &name[skipped..]))
        }
        Some((token, rest)) => name.split_first().is_some_and(|(&character, remaining)| {
            token_matches(token, character) && glob_matches(rest, remaining)
        }),
    }
}

fn token_matches(token: &GlobToken, character: char) -> bool {
    let folded = fold_case(character);
    match token {
        GlobToken::Literal(literal) => *literal == folded,
        GlobToken::AnyCharacter => true,
        GlobToken::AnyRun => unreachable!("runs are matched by glob_matches"),
        GlobToken::Class {
            is_negated,
            members,
        } => {
            let is_member = members.iter().any(|member| match member {
                ClassMember::Character(member) => fold_case(*member) == folded,
                ClassMember::Range(low, high) => {
                    (*low..=*high).contains(&character)
                        || (fold_case(*low)..=fold_case(*high)).contains(&folded)
                }
                ClassMember::Named(test) => test(&character),
            });
            is_member != *is_negated
        }
    }
}
