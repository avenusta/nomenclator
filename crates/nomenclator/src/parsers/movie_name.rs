use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use serde::Serialize;

use crate::lexical::{self, BracketClass, TitlePart as Part, Token, TokenKind, WordClass};
use crate::model::{EntryKind, Evidence, Finding, Format3d, Rule, Span};
use crate::rules::Rules;

/// Facts from one movie file or folder name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MovieName {
    /// The title, cleaned of separators (M2, M6).
    pub title: Finding<String>,
    /// The release year (M3).
    pub year: Finding<u16>,
    /// Provider IDs from the bracketed tags, keyed by provider (M7).
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// The raw text of an `{edition-X}` or `[edition-X]` tag.
    pub edition: Finding<String>,
    /// Edition words after the title, such as `Extended`. They are a hint, never an edition.
    pub edition_hint: Finding<String>,
    /// The 3D layout that the name states (M12).
    pub format_3d: Finding<Format3d>,
    /// The file extension, lowercase. `None` for folders.
    pub container: Option<String>,
    /// `true` if the name alone identifies a movie: it has a bracketed year, a provider tag, or
    /// a release name with a year.
    pub trusted: bool,
    /// The other titles that the name states (M16): after `AKA`, after ` / `, or in a
    /// trailing parenthesised group. The `title` is the first one.
    pub alternate_titles: Vec<AlternateTitle>,
}

/// A title that a name states beside its main title, with its span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlternateTitle {
    /// The display title.
    pub title: String,
    /// The text the title was read from.
    pub span: Span,
}

/// Reads the title, year and provider IDs from one movie file or folder name.
///
/// Uses the built-in rules. [`Rules::parse_movie_name`] takes other rules.
#[must_use]
pub fn parse_movie_name(name: &str, kind: EntryKind) -> MovieName {
    Rules::builtin().parse_movie_name(name, kind)
}

impl Rules {
    /// Reads the title, year and provider IDs from one movie file or folder name.
    pub fn parse_movie_name(&self, name: &str, kind: EntryKind) -> MovieName {
        let rules = self;
        let stem = lexical::stem(name, kind);
        let all_tokens: Vec<(Token, Part)> = lexical::tokenize(stem)
            .into_iter()
            .map(|token| (token, lexical::classify_title_token(rules, stem, token)))
            .collect();
        let edition = edition_tag(stem, &all_tokens);
        let mut tokens: Vec<(Token, Part)> = all_tokens
            .into_iter()
            .filter(|(_, part)| !part.is_dropped_bracket())
            .collect();
        mark_edition_phrases(rules, stem, &mut tokens);
        let provider_ids = lexical::provider_ids(rules, stem, kind == EntryKind::Directory);

        let start = title_start(&tokens);
        let cut = release_cut(&tokens, start, looks_like_release_name(stem, &tokens));
        let (year_index, year) = choose_year(&tokens, start, cut);
        let title_end = year_index.map_or(cut, |index| index.min(cut));

        let title = title_finding(stem, &tokens, start, title_end, year_index.is_some(), cut);
        let (title, alternate_titles) = split_alternate_titles(stem, title);
        let has_bracketed_year = matches!(
            year_index.map(|index| &tokens[index].1),
            Some(Part::Bracket(BracketClass::Year(_)))
        );
        let trusted = matches!(title, Finding::Resolved { .. })
            && (has_bracketed_year
                || !provider_ids.is_empty()
                || (year_index.is_some() && cut < tokens.len()));

        MovieName {
            title,
            year,
            provider_ids,
            edition,
            edition_hint: edition_hint(stem, &tokens, title_end),
            format_3d: format_3d(rules, stem, &tokens, title_end),
            container: lexical::extension(name, kind),
            trusted,
            alternate_titles,
        }
    }
}

fn is_release_signal(tokens: &[(Token, Part)], index: usize) -> bool {
    match &tokens[index].1 {
        Part::Word(WordClass::Technical | WordClass::EpisodeMarker) => true,
        Part::Bracket(BracketClass::Release | BracketClass::ReleaseGroup) => true,
        Part::Dash => is_episode_number_after_dash(tokens, index),
        _ => false,
    }
}

/// Returns the index of the first title token: 1 if a leading bracket is a release group, 0
/// otherwise. A leading bracket is a release group if it is classified as a release or
/// release-group bracket, or if release words follow it.
fn title_start(tokens: &[(Token, Part)]) -> usize {
    match tokens.first().map(|(_, part)| part) {
        Some(Part::Bracket(BracketClass::Release | BracketClass::ReleaseGroup)) => 1,
        Some(Part::Bracket(BracketClass::Other))
            if (1..tokens.len()).any(|index| is_release_signal(tokens, index)) =>
        {
            1
        }
        _ => 0,
    }
}

/// Returns `true` if the dash at `dash_index` and the number after it are an episode number
/// in a release name, as in ` - 13` after a title.
fn is_episode_number_after_dash(tokens: &[(Token, Part)], dash_index: usize) -> bool {
    let Some((number, Part::Word(WordClass::Plain))) = tokens.get(dash_index + 1) else {
        return false;
    };
    let is_short_number = (1..=3).contains(&(number.span.1 - number.span.0));
    let followed_by_release_or_nothing = match tokens.get(dash_index + 2) {
        None => true,
        Some((_, part)) => matches!(
            part,
            Part::Word(WordClass::Technical) | Part::Bracket(BracketClass::Release)
        ),
    };
    is_short_number && followed_by_release_or_nothing
}

/// Returns `true` if `stem` is a release name rather than a clean title. A release name has
/// no spaces, has a dot or underscore that joins words (`Super movie.3d`, but not
/// `Mr. Robot`), or has a technical word, a year or a release bracket.
fn looks_like_release_name(stem: &str, tokens: &[(Token, Part)]) -> bool {
    let joins_words_with_dots = stem
        .as_bytes()
        .windows(2)
        .any(|pair| matches!(pair[0], b'.' | b'_') && pair[1] != b' ');
    !stem.contains(' ')
        || joins_words_with_dots
        || tokens.iter().any(|(_, part)| {
            matches!(
                part,
                Part::Word(WordClass::Technical | WordClass::Year(_))
                    | Part::Bracket(BracketClass::Year(_) | BracketClass::Release)
            )
        })
}

/// Returns the index of the first token where release material begins.
/// Returns `tokens.len()` if no release material follows the title.
/// Technical words cut the title anywhere. Title-like and edition words cut it after a year
/// or a technical word. In a release name, they also cut it if only release material follows
/// them. A cut never empties the title.
fn release_cut(tokens: &[(Token, Part)], start: usize, is_release_name: bool) -> usize {
    let release_tail_start = release_tail_start(tokens, start);
    // A name made only of release words is its own title.
    if release_tail_start == start {
        return tokens.len();
    }
    let mut seen_year_or_technical = false;
    for index in start..tokens.len() {
        let cuts = match &tokens[index].1 {
            Part::Word(WordClass::TitleLike | WordClass::Edition) => {
                seen_year_or_technical || (is_release_name && index >= release_tail_start)
            }
            _ => is_release_signal(tokens, index),
        };
        if cuts && index > start {
            return index;
        }
        if matches!(
            tokens[index].1,
            Part::Word(WordClass::Technical | WordClass::Year(_))
                | Part::Bracket(BracketClass::Year(_))
        ) {
            seen_year_or_technical = true;
        }
    }
    tokens.len()
}

/// Returns the index where the trailing run of release material starts. A year counts as
/// release material if a technical word precedes it.
fn release_tail_start(tokens: &[(Token, Part)], start: usize) -> usize {
    let mut technical_seen_before = vec![false; tokens.len()];
    let mut seen = false;
    for (index, (_, part)) in tokens.iter().enumerate() {
        technical_seen_before[index] = seen;
        seen |= matches!(part, Part::Word(WordClass::Technical));
    }
    let mut tail_start = tokens.len();
    for index in (start..tokens.len()).rev() {
        let is_release = match &tokens[index].1 {
            Part::Word(WordClass::Year(_)) => technical_seen_before[index],
            part => part.is_release_material(),
        };
        if !is_release {
            break;
        }
        tail_start = index;
    }
    tail_start
}

/// Chooses the year of the name. Returns a bracketed year before the release cut if one
/// exists. Falls back to the last bare year before the cut that is followed by release
/// material or nothing. A leading number is the title, not a year.
fn choose_year(
    tokens: &[(Token, Part)],
    start: usize,
    cut: usize,
) -> (Option<usize>, Finding<u16>) {
    let bracketed = (start + 1..cut.min(tokens.len())).find_map(|index| match tokens[index].1 {
        Part::Bracket(BracketClass::Year(year)) => Some((index, year, Rule::BracketedYear)),
        _ => None,
    });
    let bare = || {
        (start + 1..cut.min(tokens.len())).rev().find_map(|index| {
            let Part::Word(WordClass::Year(year)) = tokens[index].1 else {
                return None;
            };
            let next = (index + 1..tokens.len()).find(|&next| tokens[next].1 != Part::Dash);
            let followed_by_release_or_nothing = next.is_none_or(|next| next >= cut);
            followed_by_release_or_nothing.then_some((index, year, Rule::BareYear))
        })
    };
    let Some((index, year, rule)) = bracketed.or_else(bare) else {
        return (None, Finding::Unknown);
    };
    let year_span = match tokens[index].0.kind {
        TokenKind::Bracket { inner } => inner,
        _ => tokens[index].0.span,
    };
    let evidence = Evidence {
        rule,
        span: year_span,
    };
    (
        Some(index),
        Finding::Resolved {
            value: year,
            evidence: Some(evidence),
        },
    )
}

/// Track names that disc-ripping tools generate (`title00`, `title_t01`, `t01`, `VTS_01_1`).
/// Such a name gives no title (M2).
static DISC_RIP_TRACK_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:title[ ._-]?t?\d{2,3}|t\d{2}|vts[ ._-]\d{1,2}[ ._-]\d{1,2})$")
        .expect("valid disc rip track pattern")
});

fn title_finding(
    stem: &str,
    tokens: &[(Token, Part)],
    start: usize,
    end: usize,
    ended_by_year: bool,
    cut: usize,
) -> Finding<String> {
    if start >= end {
        return Finding::Unknown;
    }
    let raw_span = Span(tokens[start].0.span.0, tokens[end - 1].0.span.1);
    let raw = stem[raw_span.0..raw_span.1].trim_end_matches([' ', '.', '_', '-']);
    let span = Span(raw_span.0, raw_span.0 + raw.len());
    let value = lexical::display_title(raw);
    if value.is_empty() || DISC_RIP_TRACK_PATTERN.is_match(raw) {
        return Finding::Unknown;
    }
    let rule = if ended_by_year {
        Rule::TextBeforeYear
    } else if cut < tokens.len() {
        Rule::TextBeforeReleaseWords
    } else {
        Rule::WholeName
    };
    Finding::Resolved {
        value,
        evidence: Some(Evidence { rule, span }),
    }
}

/// Returns the first `{edition-X}` or `[edition-X]` tag, with its value kept as written.
/// Returns `Unknown` if the name has no edition tag.
fn edition_tag(stem: &str, tokens: &[(Token, Part)]) -> Finding<String> {
    tokens
        .iter()
        .find_map(|(token, part)| {
            let (Part::Bracket(BracketClass::EditionTag(value)), TokenKind::Bracket { inner }) =
                (part, token.kind)
            else {
                return None;
            };
            let value_start = inner.0 + stem[inner.0..inner.1].find(value.as_str())?;
            Some(Finding::Resolved {
                value: value.clone(),
                evidence: Some(Evidence {
                    rule: Rule::EditionTag,
                    span: Span(value_start, value_start + value.len()),
                }),
            })
        })
        .unwrap_or(Finding::Unknown)
}

/// Marks the words of each multi-word edition phrase (`Directors Cut`) as edition words.
fn mark_edition_phrases(rules: &Rules, stem: &str, tokens: &mut [(Token, Part)]) {
    let mut index = 0;
    while index < tokens.len() {
        let words: Vec<&str> = tokens[index..]
            .iter()
            .take_while(|(token, _)| token.kind == TokenKind::Word)
            .map(|(token, _)| &stem[token.span.0..token.span.1])
            .collect();
        let phrase_length = lexical::edition_phrase_length(rules, &words);
        for (_, part) in &mut tokens[index..index + phrase_length] {
            *part = Part::Word(WordClass::Edition);
        }
        index += phrase_length.max(1);
    }
}

/// Returns the first run of edition words after the title.
/// Returns `Unknown` if no edition word follows the title.
fn edition_hint(stem: &str, tokens: &[(Token, Part)], title_end: usize) -> Finding<String> {
    let is_edition_word = |index: &usize| tokens[*index].1 == Part::Word(WordClass::Edition);
    let Some(first) = (title_end..tokens.len()).find(is_edition_word) else {
        return Finding::Unknown;
    };
    let last = (first..tokens.len())
        .take_while(is_edition_word)
        .last()
        .unwrap_or(first);
    let span = Span(tokens[first].0.span.0, tokens[last].0.span.1);
    Finding::Resolved {
        value: lexical::display_title(&stem[span.0..span.1]),
        evidence: Some(Evidence {
            rule: Rule::EditionWords,
            span,
        }),
    }
}

/// Returns the 3D layout that is named after the title. A layout word takes precedence over a
/// bare `3d`.
/// Returns an ambiguous finding if two different layouts are named.
fn format_3d(
    rules: &Rules,
    stem: &str,
    tokens: &[(Token, Part)],
    title_end: usize,
) -> Finding<Format3d> {
    let mut words: Vec<Span> = Vec::new();
    for (token, _) in &tokens[title_end.min(tokens.len())..] {
        match token.kind {
            TokenKind::Word => words.push(token.span),
            TokenKind::Bracket { inner } => words.extend(
                lexical::tokenize(&stem[inner.0..inner.1])
                    .into_iter()
                    .filter(|word| word.kind == TokenKind::Word)
                    .map(|word| Span(inner.0 + word.span.0, inner.0 + word.span.1)),
            ),
            TokenKind::Dash => {}
        }
    }
    let mut found: Vec<(Format3d, Span)> = words
        .into_iter()
        .filter_map(|span| {
            lexical::format_3d_word(rules, &stem[span.0..span.1]).map(|format| (format, span))
        })
        .collect();
    if found
        .iter()
        .any(|(format, _)| *format != Format3d::Unspecified)
    {
        found.retain(|(format, _)| *format != Format3d::Unspecified);
    }
    let mut layouts: Vec<Format3d> = Vec::new();
    for (format, _) in &found {
        if !layouts.contains(format) {
            layouts.push(*format);
        }
    }
    match (layouts.as_slice(), found.first()) {
        ([], _) | (_, None) => Finding::Unknown,
        ([format], Some((_, span))) => Finding::Resolved {
            value: *format,
            evidence: Some(Evidence {
                rule: Rule::Format3dWord,
                span: *span,
            }),
        },
        _ => Finding::Ambiguous {
            candidates: layouts,
            reason: "conflicting_3d_layouts",
        },
    }
}

static AKA_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[ ._]aka[ ._]").expect("valid aka pattern"));

/// Splits the other titles from a resolved title (M16): the text after `AKA` or after
/// ` / `, and a trailing parenthesised group of words. Square brackets and a leading
/// parenthesis stay part of the title (`[REC]`, `(500) Days of Summer`). Returns the
/// title unchanged if it states no other title.
fn split_alternate_titles(
    stem: &str,
    title: Finding<String>,
) -> (Finding<String>, Vec<AlternateTitle>) {
    let Finding::Resolved {
        evidence: Some(evidence),
        ..
    } = &title
    else {
        return (title, Vec::new());
    };
    let (start, mut end) = (evidence.span.0, evidence.span.1);
    let mut alternates: Vec<AlternateTitle> = Vec::new();
    let part = |from: usize, to: usize| {
        let text = &stem[from..to];
        let trimmed_start = text.len() - text.trim_start_matches([' ', '.', '_', '-']).len();
        let text = text.trim_matches([' ', '.', '_', '-']);
        (
            Span(from + trimmed_start, from + trimmed_start + text.len()),
            lexical::display_title(text),
        )
    };
    loop {
        let raw = &stem[start..end];
        // A trailing parenthesised group of words after the title.
        let last = lexical::tokenize(raw).into_iter().last();
        if let Some(Token {
            kind: TokenKind::Bracket { inner },
            span,
        }) = last
            && span.0 > 0
            && span.1 == raw.len()
            && raw.as_bytes()[span.0] == b'('
            && raw[inner.0..inner.1].chars().any(char::is_alphabetic)
        {
            let (alternate_span, value) = part(start + inner.0, start + inner.1);
            alternates.insert(
                0,
                AlternateTitle {
                    title: value,
                    span: alternate_span,
                },
            );
            end = start + span.0;
            continue;
        }
        let separator = AKA_PATTERN
            .find(raw)
            .map(|found| (found.start(), found.end()))
            .or_else(|| raw.find(" / ").map(|at| (at, at + 3)));
        if let Some((from, to)) = separator.filter(|(from, to)| *from > 0 && *to < raw.len()) {
            let (alternate_span, value) = part(start + to, end);
            alternates.insert(
                0,
                AlternateTitle {
                    title: value,
                    span: alternate_span,
                },
            );
            end = start + from;
            continue;
        }
        break;
    }
    if alternates.is_empty() {
        return (title, alternates);
    }
    let (span, value) = part(start, end);
    let title = Finding::Resolved {
        value,
        evidence: Some(Evidence {
            rule: evidence.rule,
            span,
        }),
    };
    (title, alternates)
}
