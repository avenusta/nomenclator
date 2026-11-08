//! Reads series titles from folder names. Provides the TV title and season-marker rules that
//! the season folder and episode parsers share.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::lexical::{self, BracketClass, TitlePart as Part, Token, TokenKind, WordClass};
use crate::model::{Evidence, Finding, Rule, Span};
use crate::rules::Rules;

/// Facts from one series folder name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeriesName {
    /// The series title.
    pub title: Finding<String>,
    /// The series year.
    pub year: Finding<u16>,
    /// A season that the name also states, as in a release pack `Show.S01.1080p`.
    pub season: Finding<u16>,
    /// Provider IDs from the bracketed tags, keyed by provider.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// The `{key-value}` tags whose keys state no typed fact, keyed by tag key (X29).
    pub tags: BTreeMap<String, Finding<String>>,
}

/// Reads the title, year, season and provider IDs from a series folder name. If the input is
/// a path, the series folder is its last component, or the parent of that component if that
/// component is a season folder.
///
/// Uses the built-in rules. [`Rules::parse_series_name`] takes other rules.
#[must_use]
pub fn parse_series_name(name_or_path: &str) -> SeriesName {
    Rules::builtin().parse_series_name(name_or_path)
}

impl Rules {
    /// Reads the title, year, season and provider IDs from a series folder name. If the input is
    /// a path, the series folder is its last component, or the parent of that component if that
    /// component is a season folder.
    pub fn parse_series_name(&self, name_or_path: &str) -> SeriesName {
        let rules = self;
        let components: Vec<&str> = name_or_path
            .split('/')
            .filter(|component| !component.is_empty())
            .collect();
        let name = match components.as_slice() {
            [.., parent, last]
                if rules
                    .parse_season_folder(last, Some(parent), true)
                    .season
                    .is_resolved() =>
            {
                parent
            }
            [.., last] => last,
            [] => "",
        };
        let marker = find_season_marker(rules, name, SeasonMarkerOptions::IN_SERIES_NAME);
        let title_end = marker.as_ref().map_or(name.len(), |marker| marker.span.0);
        let parts = title_parts(rules, &name[..title_end], 0);
        SeriesName {
            title: parts.title_finding(),
            year: parts.year_finding(),
            season: marker.map_or(Finding::Unknown, |marker| marker.finding()),
            provider_ids: lexical::provider_ids(rules, name, true),
            tags: lexical::custom_tags(rules, name),
        }
    }
}

/// Returns `true` if `number` is a valid season under `rules`: 0-99 or a plausible year
/// (T16), up to the `latest_plausible_year` preset (X12).
pub(crate) fn is_valid_season(rules: &Rules, number: u32) -> bool {
    number <= 99
        || (u32::from(rules.vocab.presets.earliest_plausible_year)
            ..=u32::from(rules.vocab.presets.latest_plausible_year))
            .contains(&number)
}

/// A title read from the start of some text, with the year that ends it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TitleParts {
    pub title: Option<(String, Span, Rule)>,
    pub year: Option<(u16, Span, Rule)>,
    /// The title without its trailing bare year, and that year. A bare year that ends the title
    /// stays in `title` (`Space 1999`).
    pub title_before_trailing_year: Option<(String, u16)>,
}

impl TitleParts {
    pub fn title_finding(&self) -> Finding<String> {
        match &self.title {
            Some((value, span, rule)) => Finding::Resolved {
                value: value.clone(),
                evidence: Some(Evidence {
                    rule: *rule,
                    span: *span,
                }),
            },
            None => Finding::Unknown,
        }
    }

    pub fn year_finding(&self) -> Finding<u16> {
        match self.year {
            Some((value, span, rule)) => Finding::Resolved {
                value,
                evidence: Some(Evidence { rule, span }),
            },
            None => Finding::Unknown,
        }
    }

    pub fn title_value(&self) -> Option<&str> {
        self.title.as_ref().map(|(value, _, _)| value.as_str())
    }
}

/// Reads the series title in `text`, which starts at byte `offset` of the name. A leading
/// bracket group that more text follows is a release group. A bracketed year, or a bare year
/// that release words follow, ends the title and gives the year. Release words and a ` - NN`
/// episode number also end the title. If the name has only bracket groups, the title is the
/// last plain group.
pub(crate) fn title_parts(rules: &Rules, text: &str, offset: usize) -> TitleParts {
    let tokens: Vec<(Token, Part)> = lexical::tokenize(rules, text)
        .into_iter()
        .map(|token| (token, lexical::classify_title_token(rules, text, token)))
        .filter(|(_, part)| !part.is_dropped_bracket())
        .collect();
    let mut parts = TitleParts::default();
    let start = match tokens.first() {
        Some((_, Part::Bracket(class)))
            if tokens.len() > 1 && !matches!(class, BracketClass::Year(_)) =>
        {
            1
        }
        _ => 0,
    };
    if !tokens[start.min(tokens.len())..]
        .iter()
        .any(|(_, part)| matches!(part, Part::Word(_)))
    {
        if let Some((token, _)) = tokens
            .iter()
            .rev()
            .find(|(_, part)| *part == Part::Bracket(BracketClass::Other))
            && let TokenKind::Bracket { inner } = token.kind
        {
            let value = lexical::display_title(rules, &text[inner.0..inner.1]);
            let span = Span(offset + inner.0, offset + inner.1);
            parts.title = Some((value, span, Rule::WholeName));
        }
        return parts;
    }

    let mut end = tokens.len();
    let mut end_rule = Rule::WholeName;
    for index in start + 1..tokens.len() {
        let ends_title = match &tokens[index].1 {
            Part::Bracket(BracketClass::Year(year)) => {
                let TokenKind::Bracket { inner } = tokens[index].0.kind else {
                    unreachable!("a bracket part comes from a bracket token")
                };
                parts.year = Some((
                    *year,
                    Span(offset + inner.0, offset + inner.1),
                    Rule::BracketedYear,
                ));
                true
            }
            Part::Word(WordClass::Year(year)) if is_release_material_after(&tokens, index) => {
                let span = tokens[index].0.span;
                parts.year = Some((
                    *year,
                    Span(offset + span.0, offset + span.1),
                    Rule::BareYear,
                ));
                true
            }
            Part::Word(WordClass::Technical | WordClass::EpisodeMarker)
            | Part::Bracket(BracketClass::Release | BracketClass::ReleaseGroup) => true,
            Part::Word(WordClass::TitleLike | WordClass::Edition) => {
                (index..tokens.len()).all(|later| tokens[later].1.is_release_material())
            }
            Part::Dash => is_episode_number_after_dash(text, &tokens, index),
            _ => false,
        };
        if ends_title {
            end = index;
            end_rule = if parts.year.is_some() {
                Rule::TextBeforeYear
            } else {
                Rule::TextBeforeReleaseWords
            };
            break;
        }
    }

    let Some((value, span)) = title_text(rules, text, &tokens, start, end) else {
        return parts;
    };
    parts.title = Some((value, Span(offset + span.0, offset + span.1), end_rule));
    if parts.year.is_none()
        && end - start > 1
        && let Part::Word(WordClass::Year(year)) = tokens[end - 1].1
        && let Some((before_year, _)) = title_text(rules, text, &tokens, start, end - 1)
    {
        parts.title_before_trailing_year = Some((before_year, year));
    }
    parts
}

fn title_text(
    rules: &Rules,
    text: &str,
    tokens: &[(Token, Part)],
    start: usize,
    end: usize,
) -> Option<(String, Span)> {
    if start >= end {
        return None;
    }
    let raw_start = tokens[start].0.span.0;
    let raw = text[raw_start..tokens[end - 1].0.span.1]
        .trim_end_matches(TITLE_EDGE_CHARACTERS)
        .trim_start_matches(TITLE_EDGE_CHARACTERS);
    let leading_trimmed = text[raw_start..].len()
        - text[raw_start..]
            .trim_start_matches(TITLE_EDGE_CHARACTERS)
            .len();
    let mut span_start = raw_start + leading_trimmed;
    // A leading ellipsis is part of the title: `...And Found` (T19).
    let mut raw = raw;
    if text[raw_start..span_start].ends_with("...") {
        span_start -= 3;
        raw = &text[span_start..span_start + 3 + raw.len()];
    }
    // A dotted acronym keeps its final dot: `S.O.S. (1080p)` → `S.O.S.`, and for display
    // `S.H.O.W.S01` → `S.H.O.W.`.
    let mut raw_end = span_start + raw.len();
    if text[span_start..].contains(' ')
        && text[raw_end..].starts_with('.')
        && lexical::ends_with_dotted_acronym(raw)
    {
        raw_end += 1;
        raw = &text[span_start..raw_end];
    }
    let display_source = if !raw.contains(' ') && text[raw_end..].starts_with('.') {
        &text[span_start..raw_end + 1]
    } else {
        raw
    };
    // Dots and underscores become spaces only if the text from the start of the title to the
    // end of the name has no space (M6), so `Mr_Robot 2015-06-24` keeps its underscore.
    let value = if text[span_start..].contains(' ') {
        raw.to_owned()
    } else {
        lexical::display_title(rules, display_source)
    };
    if value.is_empty() {
        return None;
    }
    Some((value, Span(span_start, span_start + raw.len())))
}

/// Separators and unbalanced brackets that never start or end a title.
pub(crate) const TITLE_EDGE_CHARACTERS: &[char] = &[
    ' ',
    '.',
    '_',
    lexical::DASHES[0],
    lexical::DASHES[1],
    '(',
    '[',
    ',',
    '#',
];

fn is_release_material_after(tokens: &[(Token, Part)], index: usize) -> bool {
    tokens[index + 1..]
        .iter()
        .find(|(_, part)| *part != Part::Dash)
        .is_some_and(|(_, part)| part.is_release_material())
}

/// Returns `true` if the dash at `dash_index` and the number after it are an episode number,
/// not part of a series title, as in ` - 01 - ` or ` - 12 [1080p]`.
fn is_episode_number_after_dash(text: &str, tokens: &[(Token, Part)], dash_index: usize) -> bool {
    let Some((number, Part::Word(WordClass::Plain))) = tokens.get(dash_index + 1) else {
        return false;
    };
    let is_number = text[number.span.0..number.span.1]
        .bytes()
        .all(|byte| byte.is_ascii_digit());
    let followed_by_dash_or_release = tokens
        .get(dash_index + 2)
        .is_some_and(|(_, part)| part.is_release_material());
    is_number && followed_by_dash_or_release
}

/// A season stated in a name: `Season 2`, `S02`, `3. Staffel`, `2nd Season`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeasonMarker {
    pub span: Span,
    pub season: u16,
    /// `true` if the marker uses a word such as `Series` that means a season only under a series
    /// folder.
    pub uses_series_word: bool,
}

impl SeasonMarker {
    fn finding(&self) -> Finding<u16> {
        Finding::Resolved {
            value: self.season,
            evidence: Some(Evidence {
                rule: Rule::SeasonMarker,
                span: self.span,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SeasonMarkerOptions {
    /// `true` if `S01E02` also states a season, as in a release name used as a folder.
    pub accept_episode_tokens: bool,
    /// `true` if `Series 5` counts only after a title (`Show Series 5`). A name that starts with
    /// it states no season.
    pub series_word_needs_title_before: bool,
}

impl SeasonMarkerOptions {
    pub const IN_SERIES_NAME: Self = Self {
        accept_episode_tokens: true,
        series_word_needs_title_before: true,
    };
    pub const IN_SEASON_FOLDER: Self = Self {
        accept_episode_tokens: false,
        series_word_needs_title_before: false,
    };
}

static SEASON_TOKEN_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)s(\d{1,4})(e\d{1,5})?").expect("valid season token pattern"));

/// Finds the first season marker in `text` that has word boundaries on both sides.
/// Returns `None` if `text` has no season marker.
pub(crate) fn find_season_marker(
    rules: &Rules,
    text: &str,
    options: SeasonMarkerOptions,
) -> Option<SeasonMarker> {
    let mut markers = Vec::new();
    let is_series_word = |word: &str| {
        let composed: String = lexical::vocabulary_form(word);
        rules.vocab.series_season_words.contains(&composed)
    };
    let bounded = |start: usize, end: usize| {
        lexical::is_boundary_before(text, start) && lexical::is_boundary_after(text, end)
    };
    for captures in rules.patterns.season_word_then_number.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        if bounded(whole.start(), whole.end()) {
            markers.push((
                whole.start(),
                whole.end(),
                captures.get(2).expect("number").as_str(),
                is_series_word(&captures[1]),
            ));
        }
    }
    for captures in rules.patterns.ordinal_then_season_word.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        if bounded(whole.start(), whole.end()) {
            markers.push((
                whole.start(),
                whole.end(),
                captures.get(1).expect("number").as_str(),
                is_series_word(&captures[2]),
            ));
        }
    }
    for captures in SEASON_TOKEN_PATTERN.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        let has_episode = captures.get(2).is_some();
        if bounded(whole.start(), whole.end()) && (!has_episode || options.accept_episode_tokens) {
            markers.push((
                whole.start(),
                whole.end(),
                captures.get(1).expect("number").as_str(),
                false,
            ));
        }
    }
    markers.sort_by_key(|(start, ..)| *start);
    markers
        .into_iter()
        .find_map(|(start, end, number, uses_series_word)| {
            let title_before = !text[..start].trim_matches(TITLE_EDGE_CHARACTERS).is_empty();
            if uses_series_word && options.series_word_needs_title_before && !title_before {
                return None;
            }
            let season: u32 = number.parse().ok()?;
            is_valid_season(rules, season).then_some(SeasonMarker {
                span: Span(start, end),
                season: season as u16,
                uses_series_word,
            })
        })
}
