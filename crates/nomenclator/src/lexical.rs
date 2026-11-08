//! Recognizes the syntax that movie and TV names share: tokens, spans, years, provider tags
//! and release words. The module only recognizes this syntax. The movie and TV pipelines
//! decide what it means.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::data;
use crate::model::{EntryKind, Finding, Format3d, SidecarFlag, Span};
use crate::rules::{DateOrder, Rules, TagFact};

/// Returns the part of `name` before its extension.
/// Returns all of `name` if `kind` is a folder, because folders have no extension.
pub(crate) fn stem(name: &str, kind: EntryKind) -> &str {
    match (kind, name.rfind('.')) {
        (EntryKind::File, Some(dot)) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// Returns the lowercase extension of a file name.
/// Returns `None` if `kind` is a folder or the name has no extension.
pub(crate) fn extension(name: &str, kind: EntryKind) -> Option<String> {
    match (kind, name.rfind('.')) {
        (EntryKind::File, Some(dot)) if dot > 0 && dot + 1 < name.len() => {
            Some(vocabulary_form(&name[dot + 1..]))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Word,
    /// A balanced `[…]`, `(…)` or `{…}` group; `inner` excludes the brackets.
    Bracket {
        inner: Span,
    },
    /// A dash standing between separators, as in `Title - 2016`.
    Dash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub span: Span,
    pub kind: TokenKind,
}

/// Splits a name into words, bracket groups and standalone dashes. The separators of `rules`
/// and in-word dashes (`x264-GRP`) separate words; a dash between digits (`2013-12-09`) does
/// not.
pub(crate) fn tokenize(rules: &Rules, text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut word_start: Option<usize> = None;
    let mut index = 0;
    let flush = |tokens: &mut Vec<Token>, word_start: &mut Option<usize>, end: usize| {
        if let Some(start) = word_start.take() {
            tokens.push(Token {
                span: Span(start, end),
                kind: TokenKind::Word,
            });
        }
    };
    while let Some(character) = text[index..].chars().next() {
        if let Some(&close) = rules.vocab.bracket_pairs.get(&character)
            && let Some(close_index) = matching_close(text, index, character, close)
        {
            flush(&mut tokens, &mut word_start, index);
            let end = close_index + close.len_utf8();
            tokens.push(Token {
                span: Span(index, end),
                kind: TokenKind::Bracket {
                    inner: Span(index + character.len_utf8(), close_index),
                },
            });
            index = end;
            continue;
        }
        let between_digits = index > 0
            && bytes[index - 1].is_ascii_digit()
            && bytes.get(index + 1).is_some_and(u8::is_ascii_digit);
        let ends_word = rules.vocab.separators.contains(&character)
            || (character == '-' && word_start.is_some() && !between_digits);
        if ends_word {
            flush(&mut tokens, &mut word_start, index);
        } else if character == '-' && word_start.is_none() {
            tokens.push(Token {
                span: Span(index, index + 1),
                kind: TokenKind::Dash,
            });
        } else if word_start.is_none() {
            word_start = Some(index);
        }
        index += character.len_utf8();
    }
    flush(&mut tokens, &mut word_start, bytes.len());
    tokens
}

/// Returns the byte offset of the `close` that balances the `open` at `open_index`.
/// Returns `None` if the group is not closed.
fn matching_close(text: &str, open_index: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (offset, character) in text[open_index..].char_indices() {
        if character == open {
            depth += 1;
        } else if character == close {
            depth -= 1;
            if depth == 0 {
                return Some(open_index + offset);
            }
        }
    }
    None
}

/// What a single word is, before any decision about the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordClass {
    Year(u16),
    Technical,
    TitleLike,
    Edition,
    EpisodeMarker,
    Plain,
}

static TECHNICAL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:\d{3,4}[pi]|\d{3,4}x\d{3,4}|\d{1,2}bit|(?:dd|ddp|dd\+|eac3|ac3|aac|dts)\d(?:\.\d)?)$",
    )
    .expect("valid technical pattern")
});

static EPISODE_MARKER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    // The widths are those of the episode name grammar, and the list form (`S01E01E03`)
    // is the renderer's (R2), so every token that the writer writes is a marker.
    Regex::new(r"(?i)^(?:s\d{1,4})?e\d{1,6}(?:(?:[-~]e?|e)\d{1,6})*$")
        .expect("valid episode pattern")
});

/// A release version such as `v2`.
static VERSION_WORD_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v\d{1,2}$").expect("valid version pattern"));

pub(crate) fn classify_word(rules: &Rules, word: &str) -> WordClass {
    if let Some(year) = parse_year(rules, word) {
        return WordClass::Year(year);
    }
    let lower = vocabulary_form(word);
    let is_technical =
        |part: &str| rules.vocab.technical_words.contains(part) || TECHNICAL_PATTERN.is_match(part);
    if is_technical(&lower) {
        return WordClass::Technical;
    }
    if EPISODE_MARKER_PATTERN.is_match(&lower) {
        return WordClass::EpisodeMarker;
    }
    if rules.vocab.edition_words.contains(&lower) {
        return WordClass::Edition;
    }
    if rules.vocab.title_like_release_words.contains(&lower)
        || VERSION_WORD_PATTERN.is_match(&lower)
    {
        return WordClass::TitleLike;
    }
    WordClass::Plain
}

/// Returns `true` if the name grammar reads `word` regardless of the vocabularies: a year, an
/// episode code, a technical pattern such as a resolution or codec, a release version, or a
/// number.
pub(crate) fn is_reserved_word(word: &str) -> bool {
    let lower = vocabulary_form(word);
    year_within(
        word,
        data::EARLIEST_PLAUSIBLE_YEAR,
        data::LATEST_PLAUSIBLE_YEAR,
    )
    .is_some()
        || EPISODE_MARKER_PATTERN.is_match(&lower)
        || TECHNICAL_PATTERN.is_match(&lower)
        || VERSION_WORD_PATTERN.is_match(&lower)
        || (!word.is_empty() && word.bytes().all(|byte| byte.is_ascii_digit()))
}

/// What one token means for a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TitlePart {
    Word(WordClass),
    Bracket(BracketClass),
    Dash,
}

impl TitlePart {
    /// Returns `true` if the token is a tag or metadata bracket that is removed from the title.
    /// The facts of such a bracket are read separately.
    pub(crate) fn is_dropped_bracket(&self) -> bool {
        matches!(
            self,
            TitlePart::Bracket(
                BracketClass::Metadata
                    | BracketClass::ProviderTag { .. }
                    | BracketClass::BareProviderId { .. }
                    | BracketClass::CustomTag { .. }
                    | BracketClass::EditionTag(_)
            )
        )
    }

    /// Returns `true` if the token may follow a title without belonging to it: a release word,
    /// a release bracket, an episode marker or a dash.
    pub(crate) fn is_release_material(&self) -> bool {
        matches!(
            self,
            TitlePart::Word(
                WordClass::Technical
                    | WordClass::TitleLike
                    | WordClass::Edition
                    | WordClass::EpisodeMarker
            ) | TitlePart::Bracket(BracketClass::Release | BracketClass::ReleaseGroup)
                | TitlePart::Dash
        )
    }
}

/// Classifies `token` of `text` by its meaning for a title.
pub(crate) fn classify_title_token(rules: &Rules, text: &str, token: Token) -> TitlePart {
    match token.kind {
        TokenKind::Word => TitlePart::Word(classify_word(rules, &text[token.span.0..token.span.1])),
        TokenKind::Bracket { inner } => {
            TitlePart::Bracket(classify_bracket(rules, text, token.span, inner))
        }
        TokenKind::Dash => TitlePart::Dash,
    }
}

/// Returns how many of `words` the longest edition phrase that starts at the first word covers.
/// Returns 0 if no edition phrase starts there.
pub(crate) fn edition_phrase_length(rules: &Rules, words: &[&str]) -> usize {
    let phrases: Vec<Vec<&str>> = rules
        .vocab
        .edition_phrases
        .iter()
        .map(|phrase| phrase.split(' ').collect())
        .collect();
    let longest_phrase = phrases.iter().map(Vec::len).max();
    let comparable: Vec<String> = words
        .iter()
        .take(longest_phrase.unwrap_or_default())
        .map(|word| vocabulary_form(word).replace(['\'', '’'], ""))
        .collect();
    phrases
        .iter()
        .filter(|phrase| {
            phrase.len() <= words.len()
                && phrase
                    .iter()
                    .zip(&comparable)
                    .all(|(phrase_word, word)| word == phrase_word)
        })
        .map(Vec::len)
        .max()
        .unwrap_or(0)
}

/// Returns the 3D layout that `word` names.
/// Returns `None` if `word` names no layout.
pub(crate) fn format_3d_word(rules: &Rules, word: &str) -> Option<Format3d> {
    rules
        .vocab
        .format_3d_words
        .get(&vocabulary_form(word))
        .copied()
}

/// Parses `word` as a four-digit year between the earliest and the latest plausible year of
/// the rules (X5, 1900 to 2099 by default).
/// Returns `None` if `word` is not such a year.
pub(crate) fn parse_year(rules: &Rules, word: &str) -> Option<u16> {
    year_within(
        word,
        rules.vocab.presets.earliest_plausible_year,
        rules.vocab.presets.latest_plausible_year,
    )
}

fn year_within(word: &str, earliest: u16, latest: u16) -> Option<u16> {
    if word.len() != 4 || !word.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    word.parse()
        .ok()
        .filter(|year| (earliest..=latest).contains(year))
}

/// What a bracket group holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BracketClass {
    Year(u16),
    ProviderTag {
        provider: String,
        id: String,
    },
    /// A provider ID that has a prefixed shape, alone in brackets.
    BareProviderId {
        provider: String,
        id: String,
    },
    /// A `{key-value}` tag whose key states no typed fact (X29).
    CustomTag {
        key: String,
        value: String,
    },
    EditionTag(String),
    /// Checksums, subtitle or audio markers, and unfilled templates. These brackets are removed
    /// from the title entirely.
    Metadata,
    /// Release words or a number. The title ends before this bracket.
    Release,
    /// Ends with a release-group marker such as `subs` or `raws`.
    ReleaseGroup,
    /// Part of the title, such as `[REC]`.
    Other,
}

/// A key, its separator and its value: `tmdb-1`, `imdbid=tt0000001`, `edition-Final Cut`.
static TAG_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^([a-z][a-z0-9_]*?)(\s*[-=]\s*)(.*)$").expect("valid tag pattern")
});
static METADATA_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:[0-9a-f]{8}|(?:multi(?:ple)?|dual)[ ._-]?(?:audio|subs?|subtitles?)?|字)$")
        .expect("valid metadata pattern")
});
/// Classifies the bracket group at `span` of `text`, whose content is at `inner`.
pub(crate) fn classify_bracket(rules: &Rules, text: &str, span: Span, inner: Span) -> BracketClass {
    let trimmed = text[inner.0..inner.1].trim();
    if let Some(year) = parse_year(rules, trimmed) {
        return BracketClass::Year(year);
    }
    if let Some(class) = classify_tag(rules, trimmed, text[span.0..].starts_with('{')) {
        return class;
    }
    if trimmed.contains('{') || METADATA_PATTERN.is_match(trimmed) {
        return BracketClass::Metadata;
    }
    if !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return BracketClass::Release;
    }
    let lower = vocabulary_form(trimmed);
    if rules
        .vocab
        .release_group_endings
        .iter()
        .any(|ending| lower.ends_with(ending.as_str()))
    {
        return BracketClass::ReleaseGroup;
    }
    let word_classes: Vec<WordClass> = tokenize(rules, trimmed)
        .iter()
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| classify_word(rules, &trimmed[token.span.0..token.span.1]))
        .collect();
    let has_technical_word = word_classes.contains(&WordClass::Technical);
    let is_only_release_words = !word_classes.is_empty()
        && word_classes
            .iter()
            .all(|class| matches!(class, WordClass::TitleLike | WordClass::Edition));
    if has_technical_word || is_only_release_words {
        BracketClass::Release
    } else {
        BracketClass::Other
    }
}

/// Classifies `text` as a tag: a provider ID alone, a known key with its value, or a curly
/// `key-value` tag (X29).
/// Returns `None` if `text` is not a tag.
fn classify_tag(rules: &Rules, text: &str, is_curly: bool) -> Option<BracketClass> {
    if let Some(provider) = bare_id_provider(rules, text) {
        return Some(BracketClass::BareProviderId {
            provider,
            id: text.to_owned(),
        });
    }
    let captures = TAG_PATTERN.captures(text)?;
    let key = vocabulary_form(&captures[1]);
    let value = &captures[3];
    match rules.tag(&key) {
        Some((_, TagFact::Edition)) if &captures[2] == "-" && !value.is_empty() => {
            Some(BracketClass::EditionTag(value.trim().to_owned()))
        }
        Some((provider, TagFact::ProviderId { id: shape }))
            if !value.contains(char::is_whitespace) =>
        {
            Some(if shape.is_valid(value) {
                BracketClass::ProviderTag {
                    provider,
                    id: value.to_owned(),
                }
            } else {
                // An unfilled template (`{imdb-}`) or an invalid ID.
                BracketClass::Metadata
            })
        }
        None if is_curly && !value.is_empty() => Some(BracketClass::CustomTag {
            key,
            value: value.to_owned(),
        }),
        _ => None,
    }
}

/// Returns the provider whose prefixed shape `text` has.
/// Returns `None` if no provider has that shape.
fn bare_id_provider(rules: &Rules, text: &str) -> Option<String> {
    rules.vocab.tags.iter().find_map(|(key, fact)| match fact {
        TagFact::ProviderId { id: shape } if shape.is_prefixed() && shape.has_form(text) => {
            Some(key.clone())
        }
        _ => None,
    })
}

/// Reads the provider IDs from every bracket group in `text`.
/// Returns an ambiguous finding for a provider that is claimed with different values.
/// Uses a provider ID alone in brackets only if no tag supplies an ID of that provider. Uses
/// an unbracketed ID that ends the text only if `accept_final_bare_id` is `true` (M7: the
/// final token of a folder name).
pub(crate) fn provider_ids(
    rules: &Rules,
    text: &str,
    accept_final_bare_id: bool,
) -> BTreeMap<String, Finding<String>> {
    let mut claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut bare_ids: BTreeMap<String, String> = BTreeMap::new();
    for token in tokenize(rules, text) {
        let TokenKind::Bracket { inner } = token.kind else {
            continue;
        };
        match classify_bracket(rules, text, token.span, inner) {
            BracketClass::ProviderTag { provider, id } => {
                let values = claims.entry(provider).or_default();
                if !values.contains(&id) {
                    values.push(id);
                }
            }
            BracketClass::BareProviderId { provider, id }
                if rules.valid_provider_id(&provider, &id).is_some() =>
            {
                bare_ids.entry(provider).or_insert(id);
            }
            _ => {}
        }
    }
    // A bare ID ending the last path component, as in `Movie (2021) tt8049994`.
    let last_component = text.rsplit('/').next().unwrap_or(text);
    if let Some(last_word) = last_component.split(' ').next_back()
        && accept_final_bare_id
        && last_component.contains(' ')
        && let Some(provider) = bare_id_provider(rules, last_word)
        && rules.valid_provider_id(&provider, last_word).is_some()
    {
        bare_ids
            .entry(provider)
            .or_insert_with(|| last_word.to_owned());
    }
    for (provider, id) in bare_ids {
        claims.entry(provider).or_insert_with(|| vec![id]);
    }
    claims
        .into_iter()
        .map(|(provider, mut values)| {
            let finding = if values.len() == 1 {
                Finding::Resolved {
                    value: values.remove(0),
                    evidence: None,
                }
            } else {
                Finding::Ambiguous {
                    candidates: values,
                    reason: crate::model::CONFLICTING_PROVIDER_ID,
                }
            };
            (provider, finding)
        })
        .collect()
}

/// Reads the `{key-value}` tags of `text` whose keys state no typed fact (X29).
/// Returns an ambiguous finding for a key that is stated with different values.
pub(crate) fn custom_tags(rules: &Rules, text: &str) -> BTreeMap<String, Finding<String>> {
    let mut claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for token in tokenize(rules, text) {
        let TokenKind::Bracket { inner } = token.kind else {
            continue;
        };
        if let BracketClass::CustomTag { key, value } =
            classify_bracket(rules, text, token.span, inner)
        {
            let values = claims.entry(key).or_default();
            if !values.contains(&value) {
                values.push(value);
            }
        }
    }
    claims
        .into_iter()
        .map(|(key, mut values)| {
            let finding = if values.len() == 1 {
                Finding::Resolved {
                    value: values.remove(0),
                    evidence: None,
                }
            } else {
                Finding::Ambiguous {
                    candidates: values,
                    reason: crate::model::CONFLICTING_TAG,
                }
            };
            (key, finding)
        })
        .collect()
}

static ACRONYM_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:[A-Z]\.){2,}(?:[A-Z]\b)?").expect("valid acronym pattern"));

/// Returns `true` if `text` ends in a dotted acronym that is written without its final dot
/// (`S.O.S`, M6).
pub(crate) fn ends_with_dotted_acronym(text: &str) -> bool {
    ACRONYM_PATTERN
        .find_iter(text)
        .last()
        .is_some_and(|acronym| acronym.end() == text.len() && !text.ends_with('.'))
}

/// Returns the display title of `raw`. If the text has no spaces, dots and underscores become
/// spaces, except inside dotted acronyms. Letter case is kept.
pub(crate) fn display_title(rules: &Rules, raw: &str) -> String {
    display_title_among(rules, raw, raw.contains(' '))
}

/// Returns the [`display_title`] of a name that is read with its siblings.
/// Keeps dots and underscores if `spaced` is `true`, which means that the name, or a name
/// that is read the same way, has spaces (M6).
pub(crate) fn display_title_among(rules: &Rules, raw: &str, spaced: bool) -> String {
    if spaced {
        return raw.to_owned();
    }
    let separators: Vec<char> = rules.vocab.separators.iter().copied().collect();
    let mut result = String::with_capacity(raw.len());
    let mut position = 0;
    for acronym in ACRONYM_PATTERN.find_iter(raw) {
        result.push_str(&raw[position..acronym.start()].replace(separators.as_slice(), " "));
        // A dotted acronym is written with its final dot (`A.I.`, `S.W.A.T.`), whether or not
        // the name's next separator dot was read as part of it.
        result.push_str(acronym.as_str().trim_end_matches('.'));
        result.push('.');
        position = acronym.end();
        result.push(' ');
    }
    result.push_str(&raw[position..].replace(separators.as_slice(), " "));
    result
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns the BCP 47 tag that a sidecar word names. The word may be an ISO 639 code, an
/// English language name, or a tag with a region or script (`pt-BR`, `zh-Hant`). The match
/// ignores case.
/// Returns `None` if `word` names no language.
pub(crate) fn language_tag(rules: &Rules, word: &str) -> Option<String> {
    let lower = vocabulary_form(word);
    if let Some(tag) = rules.vocab.languages.get(&lower) {
        return Some(tag.clone());
    }
    let (base, subtag) = lower.split_once('-')?;
    let base = rules.vocab.languages.get(base)?;
    let is_region = (subtag.len() == 2 && subtag.bytes().all(|byte| byte.is_ascii_alphabetic()))
        || (subtag.len() == 3 && subtag.bytes().all(|byte| byte.is_ascii_digit()));
    let is_script = subtag.len() == 4 && subtag.bytes().all(|byte| byte.is_ascii_alphabetic());
    if is_region {
        Some(format!("{base}-{}", subtag.to_ascii_uppercase()))
    } else if is_script {
        Some(format!(
            "{base}-{}{}",
            subtag[..1].to_ascii_uppercase(),
            &subtag[1..]
        ))
    } else {
        None
    }
}

/// Returns the flag that a sidecar word sets, ignoring case.
/// Returns `None` for `hi`, because `hi` may also be Hindi.
pub(crate) fn sidecar_flag(rules: &Rules, word: &str) -> Option<SidecarFlag> {
    let lower = vocabulary_form(word);
    if rules.vocab.default_flag_words.contains(&lower) {
        Some(SidecarFlag::Default)
    } else if rules.vocab.forced_flag_words.contains(&lower) {
        Some(SidecarFlag::Forced)
    } else if rules.vocab.hearing_impaired_flag_words.contains(&lower) {
        Some(SidecarFlag::HearingImpaired)
    } else {
        None
    }
}

/// Returns the byte length of the prefix of `name` that equals `prefix` ignoring case
/// ([`fold_case`]). Folding may change byte lengths (`ẞ` / `ß`), so the length is measured
/// in `name`.
/// Returns `None` if no prefix of `name` equals `prefix`.
pub(crate) fn caseless_prefix_length(name: &str, prefix: &str) -> Option<usize> {
    let wanted = fold_case_text(prefix);
    let mut folded = String::new();
    for (index, character) in name.char_indices() {
        folded.push(fold_case(character));
        if folded == wanted {
            return Some(index + character.len_utf8());
        }
        if !wanted.starts_with(&folded) {
            return None;
        }
    }
    None
}

/// The dashes that separate the parts of a name, such as a title and an episode number:
/// the hyphen-minus and the en dash (U+2013).
pub(crate) const DASHES: [char; 2] = ['-', '\u{2013}'];

/// The byte that replaces masked technical material. It is neither a letter, a digit nor a
/// word separator, so masked text never forms or splits a number.
pub(crate) const MASK: u8 = b'#';

static RESOLUTION_WORD_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\d{3,4}[pi]").expect("valid resolution pattern"));
static DIMENSIONS_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\d{3,4}x(\d{3,4})").expect("valid dimensions pattern"));
static CODEC_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[xh]\.?26[45]").expect("valid codec pattern"));
static CHECKSUM_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[[0-9A-Fa-f]{8}\]|\([0-9A-Fa-f]{8}\)").expect("valid checksum pattern")
});

/// Returns `text` with resolutions, dimensions, aspect ratios, audio layouts, codecs that
/// contain digits, and checksums replaced byte for byte by [`MASK`]. The numbers inside them
/// are then never read as seasons, episodes or dates. Byte offsets are unchanged.
pub(crate) fn mask_technical(rules: &Rules, text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut mask_matches = |pattern: &Regex, accept: &dyn Fn(&regex::Captures) -> bool| {
        let current = String::from_utf8(bytes.clone()).expect("masking keeps UTF-8");
        for captures in pattern.captures_iter(&current) {
            let whole = captures.get(0).expect("match");
            if is_boundary_before(&current, whole.start())
                && is_boundary_after(&current, whole.end())
                && accept(&captures)
            {
                bytes[whole.start()..whole.end()].fill(MASK);
            }
        }
    };
    mask_matches(&CHECKSUM_PATTERN, &|_| true);
    mask_matches(&rules.patterns.audio_layout, &|_| true);
    mask_matches(&CODEC_PATTERN, &|_| true);
    mask_matches(&RESOLUTION_WORD_PATTERN, &|_| true);
    mask_matches(&DIMENSIONS_PATTERN, &|captures| {
        rules.vocab.video_heights.contains(&captures[1])
    });
    mask_matches(&rules.patterns.aspect_ratio, &|_| true);
    String::from_utf8(bytes).expect("masking keeps UTF-8")
}

/// Returns `true` if no letter or digit is directly before `position`.
pub(crate) fn is_boundary_before(text: &str, position: usize) -> bool {
    text[..position]
        .chars()
        .next_back()
        .is_none_or(|previous| !previous.is_alphanumeric())
}

/// Returns `true` if no letter or digit is at `position`.
pub(crate) fn is_boundary_after(text: &str, position: usize) -> bool {
    text[position..]
        .chars()
        .next()
        .is_none_or(|next| !next.is_alphanumeric())
}

/// Returns the comparison form of a title: lowercase letters and digits only, with ligatures
/// spelled out. `Æon Example`, `aeon.example` and `Aeon Example` have the same form.
pub(crate) fn title_key(rules: &Rules, title: &str) -> String {
    title_key_with(&rules.vocab.letter_folds, title)
}

/// Returns the comparison form of a title without letter folds, for vocabulary entries that
/// are compared in that form before any rules exist.
pub(crate) fn plain_title_key(title: &str) -> String {
    title_key_with(&BTreeMap::new(), title)
}

fn title_key_with(letter_folds: &BTreeMap<char, String>, title: &str) -> String {
    let mut key = String::with_capacity(title.len());
    for character in title.nfc().flat_map(char::to_lowercase) {
        if let Some(spelled) = letter_folds.get(&character) {
            key.push_str(spelled);
        } else if character.is_alphanumeric() {
            key.push(character);
        }
    }
    key
}

/// Returns the byte length of the shortest prefix of `text` whose [`title_key`] is `key`.
/// Returns `None` if `key` is empty or no prefix has that key.
pub(crate) fn prefix_with_title_key(rules: &Rules, text: &str, key: &str) -> Option<usize> {
    if key.is_empty() {
        return None;
    }
    let mut consumed = String::new();
    for (index, character) in text.char_indices() {
        consumed.push(character);
        let consumed_key = title_key(rules, &consumed);
        if consumed_key == key {
            return Some(index + character.len_utf8());
        }
        if !key.starts_with(&consumed_key) {
            return None;
        }
    }
    None
}

/// Case-folds one character without depending on a locale. The function takes the uppercase
/// of `character`, then the lowercase of that result. Each step is kept only if it gives a
/// single character. `ς`, `σ` and `Σ` all fold to `σ`.
pub(crate) fn fold_case(character: char) -> char {
    if character.is_ascii() {
        return character.to_ascii_lowercase();
    }
    fn single(mut characters: impl Iterator<Item = char>) -> Option<char> {
        let first = characters.next()?;
        characters.next().is_none().then_some(first)
    }
    let upper = single(character.to_uppercase()).unwrap_or(character);
    single(upper.to_lowercase()).unwrap_or(upper)
}

/// Returns the form in which a vocabulary stores and looks up a word: NFC, then
/// case-folded. Every vocabulary entry and every lookup uses this form (X25).
pub(crate) fn vocabulary_form(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    fold_case_text(&text.nfc().collect::<String>())
}

/// Returns the height and scan type of the resolution that `token` states: `1080p`,
/// `1080i`, `1080p24` (the frame rate is left out), dimensions such as `1920x1080`, a height
/// of `data::BARE_RESOLUTIONS`, or a word of `data::RESOLUTION_WORDS` such as `4K`.
/// Returns `None` if the token states no resolution or a height outside 240-4320.
pub(crate) fn resolution(token: &str) -> Option<(u32, char)> {
    let folded = fold_case_text(token);
    if let Some((_, height)) = resolution_word(&folded) {
        return Some((height, 'p'));
    }
    let is_height = |text: &str| {
        (3..=4).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    let (height, scan) = match folded.split_once('x') {
        Some((width, height)) if is_height(width) && is_height(height) => (height, 'p'),
        Some(_) => return None,
        None => {
            let digits = folded.len()
                - folded
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .len();
            let (height, rest) = folded.split_at(digits);
            match rest.chars().next() {
                None if data::BARE_RESOLUTIONS.contains(&height) => (height, 'p'),
                Some(scan @ ('p' | 'i'))
                    if rest.len() <= 3 && rest[1..].bytes().all(|byte| byte.is_ascii_digit()) =>
                {
                    (height, scan)
                }
                _ => return None,
            }
        }
    };
    let height: u32 = height.parse().ok().filter(|_| is_height(height))?;
    (240..=4320).contains(&height).then_some((height, scan))
}

/// Returns the word of `data::RESOLUTION_WORDS` that `folded` is, with its height.
/// Returns `None` if `folded` is not a resolution word.
pub(crate) fn resolution_word(folded: &str) -> Option<(&'static str, u32)> {
    data::RESOLUTION_WORDS
        .iter()
        .find(|(word, _)| *word == folded)
        .copied()
}

/// Returns `text` with every character case-folded. The result is used for comparisons only.
pub(crate) fn fold_case_text(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    text.chars().map(fold_case).collect()
}

/// A calendar date in a name, with the order its parts were read in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateMatch {
    pub span: Span,
    /// `YYYY-MM-DD` dates. The list has two candidates if the day and month order is unknown.
    pub candidates: Vec<String>,
}

static YEAR_FIRST_DATE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((?:19|20)\d{2})([-._ ])(\d{1,2})[-._ ](\d{1,2})").expect("valid date pattern")
});
static YEAR_LAST_DATE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{1,2})([-._ ])(\d{1,2})[-._ ]((?:19|20)\d{2})").expect("valid date pattern")
});

/// Finds the first date in `text`. Reads a year-first date as year, month, day. Reads a day-
/// or month-first date by the part that is above 12. Keeps both orders if neither part is
/// above 12.
/// Returns `None` if `text` contains no date.
pub(crate) fn find_date(rules: &Rules, text: &str) -> Option<DateMatch> {
    let is_plausible = |year: &str| parse_year(rules, year).is_some();
    let is_digit_boundary = |position: usize| {
        let before = text[..position].chars().next_back();
        let after = text[position..].chars().next();
        (before, after)
    };
    for captures in YEAR_FIRST_DATE_PATTERN.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        let separators_agree = text[whole.start()..whole.end()]
            .bytes()
            .filter(|byte| !byte.is_ascii_digit())
            .all(|byte| byte == captures[2].as_bytes()[0]);
        let (before, _) = is_digit_boundary(whole.start());
        let (_, after) = is_digit_boundary(whole.end());
        if !separators_agree
            || before.is_some_and(|c| c.is_ascii_digit())
            || after.is_some_and(|c| c.is_ascii_digit())
        {
            continue;
        }
        if !is_plausible(&captures[1]) {
            continue;
        }
        if let Some(date) = calendar_date(&captures[1], &captures[3], &captures[4]) {
            return Some(DateMatch {
                span: Span(whole.start(), whole.end()),
                candidates: vec![date],
            });
        }
    }
    for captures in YEAR_LAST_DATE_PATTERN.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        let (before, _) = is_digit_boundary(whole.start());
        let (_, after) = is_digit_boundary(whole.end());
        if before.is_some_and(|c| c.is_ascii_digit()) || after.is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let (first, second, year) = (&captures[1], &captures[3], &captures[4]);
        if !is_plausible(year) {
            continue;
        }
        let day_first = calendar_date(year, second, first);
        let month_first = calendar_date(year, first, second);
        // The `date_order` preset decides the order only if both readings are dates (T13, X12).
        let preferred = match (rules.vocab.presets.date_order, &day_first, &month_first) {
            (DateOrder::DayFirst, Some(_), Some(_)) => [day_first, None],
            (DateOrder::MonthFirst, Some(_), Some(_)) => [month_first, None],
            _ => [day_first, month_first],
        };
        let candidates: Vec<String> =
            preferred
                .into_iter()
                .flatten()
                .fold(Vec::new(), |mut unique, date| {
                    if !unique.contains(&date) {
                        unique.push(date);
                    }
                    unique
                });
        if !candidates.is_empty() {
            return Some(DateMatch {
                span: Span(whole.start(), whole.end()),
                candidates,
            });
        }
    }
    None
}

fn calendar_date(year: &str, month: &str, day: &str) -> Option<String> {
    let (month, day): (u8, u8) = (month.parse().ok()?, day.parse().ok()?);
    let is_valid = (1..=12).contains(&month) && (1..=31).contains(&day);
    is_valid.then(|| format!("{year}-{month:02}-{day:02}"))
}

/// Returns the case-folded alphanumeric words of `text`. The words compare names that differ
/// only in case, punctuation or separators (`Rush.Hour` and `Rush Hour`).
pub(crate) fn comparison_words(text: &str) -> Vec<String> {
    fold_case_text(text)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Returns the [`comparison_words`] of a title without the edition words and phrases that end
/// it (`Movie Super-Sized R-Rated Version` → `movie`). Phrases are compared with their words
/// joined, so `Director's Cut` and `Super-Sized` match regardless of punctuation.
pub(crate) fn comparison_words_without_trailing_editions(
    rules: &Rules,
    title: &str,
) -> Vec<String> {
    let mut words = comparison_words(title);
    let joined_edition_phrases: Vec<String> = rules
        .vocab
        .edition_phrases
        .iter()
        .map(|phrase| phrase.replace(' ', ""))
        .chain(rules.vocab.edition_words.iter().cloned())
        .collect();
    'strip: while words.len() > 1 {
        for length in 1..words.len() {
            let tail = words[words.len() - length..].concat();
            if joined_edition_phrases.contains(&tail) {
                words.truncate(words.len() - length);
                continue 'strip;
            }
        }
        break;
    }
    words
}

/// Returns the rest of a title after a leading short alias. A short alias is an all-caps
/// acronym of two to four letters that more words follow (`AVP Alien vs. Predator` →
/// `Alien vs. Predator`).
/// Returns `None` if the title does not start with a short alias.
pub(crate) fn without_leading_alias(title: &str) -> Option<&str> {
    let (alias, rest) = title.trim().split_once(' ')?;
    let is_alias = (2..=4).contains(&alias.chars().count())
        && alias.chars().all(|letter| letter.is_uppercase());
    let rest = rest.trim_start();
    (is_alias && rest.chars().any(char::is_alphanumeric)).then_some(rest)
}

/// Returns `true` if `stem` contains an episode token such as `S01E02` or `e05`.
pub(crate) fn has_episode_marker(rules: &Rules, stem: &str) -> bool {
    tokenize(rules, stem).iter().any(|token| {
        token.kind == TokenKind::Word
            && classify_word(rules, &stem[token.span.0..token.span.1]) == WordClass::EpisodeMarker
    })
}

/// Returns the words of a title in the comparison form for matching and title agreement
/// (P4, M1). The form is never displayed. The words are case-folded, ligatures are spelled
/// out, accents are stripped, a leading article from the data is dropped, and Roman numerals
/// I-XX and spelled numbers one-twenty are read as digits (`The Northern Passage II` →
/// `northern`, `passage`, `2`).
pub(crate) fn title_comparison_words(rules: &Rules, title: &str) -> Vec<String> {
    fold_title_comparison_words(rules, comparison_words(&title.nfc().collect::<String>()))
}

/// Applies the folds of [`title_comparison_words`] to words already in [`comparison_words`] form.
pub(crate) fn fold_title_comparison_words(rules: &Rules, words: Vec<String>) -> Vec<String> {
    let mut folded: Vec<String> = words
        .iter()
        .map(|word| without_accents(&title_key(rules, word)))
        .filter(|word| !word.is_empty())
        .collect();
    if folded.len() > 1 && rules.vocab.leading_articles.contains(&folded[0]) {
        folded.remove(0);
    }
    folded
        .into_iter()
        .map(|word| {
            rules
                .vocab
                .number_words
                .get(&word)
                .map_or(word, |number| number.to_string())
        })
        .collect()
}

/// Returns `text` without its combining marks. The function decomposes `text` (NFD), drops
/// the combining marks and recomposes the rest (`é` → `e`, `ό` → `ο`).
pub(crate) fn without_accents(text: &str) -> String {
    text.nfd()
        .filter(|character| !unicode_normalization::char::is_combining_mark(*character))
        .nfc()
        .collect()
}

/// Returns the comparison key of a title for matching: its [`title_comparison_words`] joined.
pub(crate) fn title_comparison_key(rules: &Rules, title: &str) -> String {
    title_comparison_words(rules, title).concat()
}

/// Returns `true` if a folder title and a file title name the same work (M1, P4). The titles
/// agree if their comparison words are equal, with articles, numbers and accents folded,
/// after two removals: edition words that end the file title (`Super-Sized`, `Director's Cut`)
/// and a short alias that leads either title (`AVP Alien vs. Predator`).
pub(crate) fn titles_agree(rules: &Rules, folder_title: &str, file_title: &str) -> bool {
    let words = |title: &str| fold_title_comparison_words(rules, comparison_words(title));
    let words_without_editions = |title: &str| {
        fold_title_comparison_words(
            rules,
            comparison_words_without_trailing_editions(rules, title),
        )
    };
    let folder_words = words(folder_title);
    let file_words = words_without_editions(file_title);
    folder_words == file_words
        || without_leading_alias(folder_title).is_some_and(|rest| words(rest) == file_words)
        || without_leading_alias(file_title)
            .is_some_and(|rest| words_without_editions(rest) == folder_words)
}
