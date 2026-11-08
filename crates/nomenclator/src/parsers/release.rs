//! Reads the release facts of a name (X19): resolution, source, codecs, bit depth, HDR,
//! audio, language markers, release group, checksum, revision, streaming service and
//! container. Every fact is optional and carries the span it was read from.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::data;
use crate::lexical;
use crate::model::{EntryKind, Evidence, Finding, Rule, Span};
use crate::rules::Rules;

/// The facts a release name states after its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseFacts {
    /// The vertical resolution with its scan type (`1080p`, `1080i`), or the dimensions
    /// as written (`720x432`).
    pub resolution: Finding<String>,
    /// The source medium: `bluray`, `uhd_bluray`, `web`, `hdtv`, `dvd` and others.
    pub source: Finding<String>,
    /// `true` if the release is a remux of its source.
    pub remux: bool,
    /// `true` if the release is a screener (`DVDScr`, `SCR`).
    pub screener: bool,
    /// The video codec: `h264`, `h265`, `xvid`, `av1` and others.
    pub video_codec: Finding<String>,
    /// The video bit depth.
    pub bit_depth: Finding<u8>,
    /// The HDR formats, in name order: `hdr10`, `hdr10_plus`, `dolby_vision`, `hlg`, `hdr`.
    pub hdr: Vec<ReleaseMarker>,
    /// The audio codecs, in name order: `dolby_digital`, `dts_hd`, `aac` and others.
    pub audio_codecs: Vec<ReleaseMarker>,
    /// The audio channel layout (`5.1`, `7.1`, `2.0`).
    pub audio_channels: Finding<String>,
    /// The audio language markers, in name order: `multi`, `dual`, or a BCP 47 tag.
    pub audio_languages: Vec<ReleaseMarker>,
    /// The subtitle language markers, in name order: `multi`, `subbed`, or a BCP 47 tag.
    pub subtitle_languages: Vec<ReleaseMarker>,
    /// The release group: the `-GROUP` ending the name, or a leading `[Group]`.
    pub release_group: Finding<String>,
    /// The CRC32 checksum in brackets, in uppercase.
    pub checksum: Finding<String>,
    /// The revision markers, in name order: `proper`, `repack`, `real`, `rerip`.
    pub revision: Vec<ReleaseMarker>,
    /// The streaming service: `netflix`, `amazon_prime`, `disney_plus` and others.
    pub streaming_service: Finding<String>,
    /// The lowercase file extension; `None` for folders.
    pub container: Option<String>,
}

/// A release fact that a name can state several times, with its span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseMarker {
    /// The normalized value.
    pub value: String,
    /// The text the value was read from.
    pub span: Span,
}

/// A kind of release fact, as the vocabulary in `data.rs` lists it and an overlay's
/// `release_words` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReleaseFact {
    Source,
    Remux,
    VideoCodec,
    BitDepth,
    Hdr,
    AudioCodec,
    AudioLanguage,
    SubtitleLanguage,
    Revision,
    StreamingService,
}

/// One release word spelling, compiled with its fact and value. Audio codecs also read a
/// channel layout written right after them (`DDP5.1`).
#[derive(Debug, Clone)]
pub(crate) struct ReleaseWordPattern {
    pub fact: ReleaseFact,
    pub value: String,
    pub pattern: Regex,
}

impl ReleaseWordPattern {
    /// Compiles `spelling`, in the form of [`release_spelling_form`]. A space between its
    /// words matches a space, `.`, `-`, `_` or nothing.
    pub(crate) fn compile(spelling: &str, fact: ReleaseFact, value: &str) -> Self {
        let body = spelling
            .split(' ')
            .map(regex::escape)
            .collect::<Vec<_>>()
            .join("[ ._-]?");
        let channels = if fact == ReleaseFact::AudioCodec {
            r"(?:[ ._-]?([1-9]\.[0-2]))?"
        } else {
            ""
        };
        ReleaseWordPattern {
            fact,
            value: value.to_owned(),
            pattern: Regex::new(&format!("(?i){body}{channels}"))
                .expect("valid release word pattern"),
        }
    }
}

/// Returns the form in which the rules store a release word spelling: the vocabulary form
/// (X25), with its words separated by single spaces. `WEB-DL` and `web.dl` are both
/// `web dl`, because a separator in a spelling matches any separator.
pub(crate) fn release_spelling_form(spelling: &str) -> String {
    lexical::vocabulary_form(spelling)
        .split([' ', '.', '-', '_'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The release patterns of one rule set, compiled.
#[derive(Debug, Clone)]
pub(crate) struct ReleasePatterns {
    /// Each release word spelling, compiled, in spelling order.
    pub words: Vec<ReleaseWordPattern>,
    /// An audio channel layout or an audio channel word.
    pub audio_channels: Regex,
}

impl ReleasePatterns {
    /// Compiles the release word spellings and the audio channel words.
    pub(crate) fn compile(
        release_words: &BTreeMap<String, (ReleaseFact, String)>,
        audio_channel_words: &BTreeMap<String, String>,
    ) -> Self {
        ReleasePatterns {
            words: release_words
                .iter()
                .map(|(spelling, (fact, value))| {
                    ReleaseWordPattern::compile(spelling, *fact, value)
                })
                .collect(),
            audio_channels: audio_channels_pattern(audio_channel_words.keys().map(String::as_str)),
        }
    }
}

/// Returns the pattern of an audio channel layout (`5.1`, `5.1ch`) or one of `words`.
fn audio_channels_pattern<'a>(words: impl Iterator<Item = &'a str>) -> Regex {
    let mut words: Vec<String> = words.map(regex::escape).collect();
    // Longest first, so a word is never cut short by one it starts with.
    words.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let alternatives = std::iter::once(r"[1-9]\.[0-2](?:ch)?".to_owned())
        .chain(words)
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i){alternatives}")).expect("valid channels pattern")
}

static RESOLUTION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    let words = data::RESOLUTION_WORDS
        .iter()
        .map(|(word, _)| regex::escape(word))
        .chain(
            data::BARE_RESOLUTIONS
                .iter()
                .map(|height| regex::escape(height)),
        )
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"(?i)\d{{3,4}}[pi](?:\d{{2}})?|\d{{3,4}}x\d{{3,4}}|{words}"
    ))
    .expect("valid resolution pattern")
});
static YEAR_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:19|20)\d{2}").expect("valid year pattern"));
static CHECKSUM_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\[(]([0-9A-Fa-f]{8})[\])]").expect("valid checksum pattern"));
static DOMAIN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:www\.)?[a-z0-9.-]+\.(?:com|net|org|info|to|tv|mx|me)$")
        .expect("valid domain pattern")
});
static TRAILING_TAG_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\[\]]*\]$").expect("valid trailing tag pattern"));

/// Reads the release facts of `name` under the built-in rules.
///
/// Uses the built-in rules; [`Rules::parse_release`] takes others.
#[must_use]
pub fn parse_release(name: &str, kind: EntryKind) -> ReleaseFacts {
    Rules::builtin().parse_release(name, kind)
}

impl Rules {
    /// Reads the release facts of `name`. Release words count only after the title, so a
    /// title such as `Cam` is never a source. A fact stated with two different values is
    /// ambiguous.
    pub fn parse_release(&self, name: &str, kind: EntryKind) -> ReleaseFacts {
        let stem = lexical::stem(name, kind);
        let all_words = matched_words(self, stem);
        let resolutions: Vec<(String, Span)> = RESOLUTION_PATTERN
            .find_iter(stem)
            .map(|found| Span(found.start(), found.end()))
            .filter(|span| is_bounded(stem, *span))
            .filter(|span| {
                !all_words
                    .iter()
                    .any(|word| word.span.0 < span.1 && span.0 < word.span.1)
            })
            .filter_map(|span| {
                lexical::resolution(&stem[span.0..span.1])
                    .map(|(height, scan)| (format!("{height}{scan}"), span))
            })
            .collect();
        let region_start = release_region_start(self, stem, &all_words, &resolutions);
        let words: Vec<FoundWord> = all_words
            .into_iter()
            .filter(|word| word.span.0 >= region_start)
            .collect();
        let mut facts = Facts::default();
        for found in &words {
            // A channel layout read with an audio codec (`DDP5.1`) is not the codec's span.
            let span = match found.channels {
                Some(channels) => Span(
                    found.span.0,
                    stem[..channels.0]
                        .trim_end_matches([' ', '.', '-', '_'])
                        .len(),
                ),
                None => found.span,
            };
            facts.add(found.fact, found.value, span);
            // `BDRemux` states its source and that the release is a remux.
            let text = lexical::fold_case_text(&stem[found.span.0..found.span.1]);
            facts.remux |= text.contains("remux");
            facts.screener |= found.fact == ReleaseFact::Source
                && (text.contains("scr") || found.value == "screener");
            if let Some(span) = found.channels {
                facts.channels.push((stem[span.0..span.1].to_owned(), span));
            }
        }
        let is_free = |span: Span| {
            !words
                .iter()
                .any(|word| word.span.0 < span.1 && span.0 < word.span.1)
        };
        let in_region: Vec<(String, Span)> = resolutions
            .into_iter()
            .filter(|(_, span)| span.0 >= region_start)
            .collect();
        // An explicit height takes precedence over `4K`, which also names a restoration.
        let is_word = |span: &Span| {
            lexical::resolution_word(&lexical::fold_case_text(&stem[span.0..span.1])).is_some()
        };
        let has_height = in_region.iter().any(|(_, span)| !is_word(span));
        facts.resolution = in_region
            .into_iter()
            .filter(|(_, span)| !(has_height && is_word(span)))
            .collect();
        for found in self.release_patterns().audio_channels.find_iter(stem) {
            let span = Span(found.start(), found.end());
            let before = &stem[..span.0];
            let follows_number = before.ends_with(|c: char| c.is_ascii_digit())
                || (before.ends_with('.')
                    && before[..before.len() - 1].ends_with(|c: char| c.is_ascii_digit()));
            let is_number_bounded =
                !follows_number && !stem[span.1..].starts_with(|c: char| c.is_ascii_digit());
            if span.0 >= region_start
                && is_number_bounded
                && is_free(span)
                && !is_word_glued(stem, span)
            {
                facts
                    .channels
                    .push((channels_value(self, found.as_str()), span));
            }
        }
        for found in language_words(self, stem, region_start, &words) {
            facts.audio_languages.push(found);
        }
        let checksum = CHECKSUM_PATTERN
            .captures_iter(stem)
            .filter_map(|captures| captures.get(1))
            .map(|hex| {
                (
                    hex.as_str().to_ascii_uppercase(),
                    Span(hex.start(), hex.end()),
                )
            })
            .collect();
        ReleaseFacts {
            resolution: scalar(facts.resolution),
            source: scalar(facts.source),
            remux: facts.remux,
            screener: facts.screener,
            video_codec: scalar(facts.video_codec),
            bit_depth: match scalar(facts.bit_depth) {
                Finding::Resolved { value, evidence } => Finding::Resolved {
                    value: value.parse().unwrap_or_default(),
                    evidence,
                },
                Finding::Ambiguous { candidates, reason } => Finding::Ambiguous {
                    candidates: candidates
                        .iter()
                        .filter_map(|depth| depth.parse().ok())
                        .collect(),
                    reason,
                },
                Finding::Unknown => Finding::Unknown,
            },
            hdr: markers(facts.hdr),
            audio_codecs: markers(facts.audio_codecs),
            audio_channels: scalar(facts.channels),
            audio_languages: markers(facts.audio_languages),
            subtitle_languages: markers(facts.subtitle_languages),
            release_group: release_group(self, stem, &words),
            checksum: scalar(checksum),
            revision: markers(facts.revision),
            streaming_service: scalar(facts.streaming_service),
            container: lexical::extension(name, kind),
        }
    }
}

/// Returns where release words may start: the end of the title the name states, or an
/// earlier strong signal after the name's first word. A strong signal is a resolution, a
/// year, a technical word (`x264`, `BluRay`) or a release word with a digit (`Hi10p`).
/// Words that are also title words (`Cam`, `Real`, `DV`) count only after this position.
fn release_region_start(
    rules: &Rules,
    stem: &str,
    words: &[FoundWord],
    resolutions: &[(String, Span)],
) -> usize {
    let title_end = match rules.parse_movie_name(stem, EntryKind::Directory).title {
        Finding::Resolved {
            evidence: Some(evidence),
            ..
        } => evidence.span.1,
        _ => 0,
    };
    let first_word_end = stem
        .find(|c: char| !c.is_alphanumeric() && c != '[')
        .unwrap_or(stem.len());
    let strong_words = words.iter().map(|word| word.span).filter(|span| {
        let text: String = lexical::vocabulary_form(&stem[span.0..span.1])
            .chars()
            .filter(|c| !matches!(c, ' ' | '.' | '_'))
            .collect();
        text.contains(|c: char| c.is_ascii_digit()) || rules.technical_words.contains(&text)
    });
    let years = YEAR_PATTERN
        .find_iter(stem)
        .map(|found| Span(found.start(), found.end()))
        .filter(|span| is_bounded(stem, *span));
    let signal = strong_words
        .chain(resolutions.iter().map(|(_, span)| *span))
        .chain(years)
        .map(|span| span.0)
        .filter(|&start| start >= first_word_end)
        .min();
    signal.map_or(title_end, |signal| signal.min(title_end))
}

/// A vocabulary match in the name.
struct FoundWord<'a> {
    fact: ReleaseFact,
    value: &'a str,
    span: Span,
    /// A channel layout written right after an audio codec.
    channels: Option<Span>,
}

/// Returns the vocabulary words of the name, without overlaps: at each position
/// the longest match wins, and an earlier match comes before a later one.
fn matched_words<'a>(rules: &'a Rules, stem: &str) -> Vec<FoundWord<'a>> {
    let mut candidates: Vec<FoundWord> = Vec::new();
    for word in &rules.release_patterns().words {
        for captures in word.pattern.captures_iter(stem) {
            let whole = captures.get(0).expect("match");
            let span = Span(whole.start(), whole.end());
            if !is_bounded(stem, span) {
                continue;
            }
            candidates.push(FoundWord {
                fact: word.fact,
                value: &word.value,
                span,
                channels: captures
                    .get(1)
                    .map(|channels| Span(channels.start(), channels.end())),
            });
        }
    }
    candidates.sort_by(|left, right| {
        left.span
            .0
            .cmp(&right.span.0)
            .then((right.span.1 - right.span.0).cmp(&(left.span.1 - left.span.0)))
    });
    let mut chosen: Vec<FoundWord> = Vec::new();
    for candidate in candidates {
        if chosen.iter().all(|kept| kept.span.1 <= candidate.span.0) {
            chosen.push(candidate);
        }
    }
    chosen
}

/// Returns `true` if no letter or digit touches the span on either side.
fn is_bounded(stem: &str, span: Span) -> bool {
    lexical::is_boundary_before(stem, span.0) && lexical::is_boundary_after(stem, span.1)
}

/// Returns `true` if a letter directly follows the span, as in `5.1ch` read as `5.1c`.
fn is_word_glued(stem: &str, span: Span) -> bool {
    stem[span.1..].starts_with(char::is_alphabetic)
}

/// Returns the channel layout that `text` states: the layout of an audio channel word
/// (`6ch` is `5.1`), or the layout as written without `ch`.
fn channels_value(rules: &Rules, text: &str) -> String {
    let folded = lexical::vocabulary_form(text);
    rules
        .audio_channel_words
        .get(&folded)
        .cloned()
        .unwrap_or_else(|| folded.trim_end_matches("ch").to_owned())
}

/// Returns the language names after the title that no vocabulary word covers, as audio
/// language markers. Only full names count (`german`), never short codes, which collide
/// with release groups and words.
fn language_words(
    rules: &Rules,
    stem: &str,
    region_start: usize,
    words: &[FoundWord],
) -> Vec<(String, Span)> {
    let mut found = Vec::new();
    for token in lexical::tokenize(&stem[region_start..]) {
        if token.kind != lexical::TokenKind::Word {
            continue;
        }
        let span = Span(region_start + token.span.0, region_start + token.span.1);
        let text = lexical::vocabulary_form(&stem[span.0..span.1]);
        let covered = words
            .iter()
            .any(|word| word.span.0 < span.1 && span.0 < word.span.1);
        if text.len() > 3
            && !covered
            && let Some(tag) = rules.language_spellings.get(&text)
        {
            found.push((tag.clone(), span));
        }
    }
    found
}

/// Returns the release group: the part after the last `-` of the name's last token, once
/// trailing site tags and checksums are removed (`x264-GROUP`); a single-letter lead keeps
/// its dash (`D-Z0N3`). Brackets around the group are removed (`-[ZeaL]`). Without such a
/// group, a leading `[Group]` followed by more text is the group, unless it is a domain,
/// a year or a checksum. A group that is a release word, a number or a resolution is not a
/// group.
fn release_group(rules: &Rules, stem: &str, words: &[FoundWord]) -> Finding<String> {
    let resolved = |value: &str, start: usize| Finding::Resolved {
        value: value.to_owned(),
        evidence: Some(Evidence {
            rule: Rule::ReleaseWord,
            span: Span(start, start + value.len()),
        }),
    };
    let overlaps_word = |span: Span| {
        words
            .iter()
            .any(|word| word.span.0 < span.1 && span.0 < word.span.1)
    };
    let mut end = stem.len();
    loop {
        let text = stem[..end].trim_end_matches([' ', '.']);
        let Some(tag) = TRAILING_TAG_PATTERN.find(text) else {
            break;
        };
        let inner = &tag.as_str()[1..tag.as_str().len() - 1];
        if !(is_site(rules, inner) || is_checksum(inner)) {
            break;
        }
        end = tag.start();
    }
    let text = stem[..end].trim_end_matches([' ', '.']);
    let token_start = text
        .rfind(['.', ' ', '_'])
        .map_or(0, |separator| separator + 1);
    let token = &text[token_start..];
    if let Some(dash) = token.rfind('-').filter(|&dash| token_start > 0 || dash > 0) {
        let lead = &token[..dash];
        let keeps_lead = lead.chars().count() == 1
            && lead.chars().all(char::is_alphabetic)
            && token[..dash].len() < token.len();
        let (group_start, group) = if keeps_lead {
            (token_start, token)
        } else {
            (token_start + dash + 1, &token[dash + 1..])
        };
        let (group_start, group) = match group
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
        {
            Some(inner) => (group_start + 1, inner),
            None => (group_start, group),
        };
        let span = Span(group_start, group_start + group.len());
        let is_number = group.bytes().all(|byte| byte.is_ascii_digit());
        let is_resolution = RESOLUTION_PATTERN
            .find(group)
            .is_some_and(|found| found.len() == group.len());
        let has_markup = group.contains(['[', ']', '(', ')', '{', '}', '+']);
        if !group.is_empty() && !has_markup && !overlaps_word(span) && !is_number && !is_resolution
        {
            return resolved(group, group_start);
        }
    }
    if let Some(close) = stem.strip_prefix('[').and_then(|rest| rest.find(']')) {
        let inner = &stem[1..=close];
        let rest = stem[close + 2..].trim();
        if !inner.trim().is_empty()
            && !rest.is_empty()
            && !is_checksum(inner)
            && !is_site(rules, inner.trim_matches(['.', ' ']))
            && lexical::parse_year(rules, inner.trim()).is_none()
        {
            return resolved(inner, 1);
        }
    }
    Finding::Unknown
}

/// Returns `true` if a bracketed tag names an index site rather than a group.
fn is_site(rules: &Rules, tag: &str) -> bool {
    rules
        .release_site_tags
        .contains(&lexical::vocabulary_form(tag))
        || DOMAIN_PATTERN.is_match(tag)
}

fn is_checksum(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The facts collected while reading a name, each with the spans that stated it.
#[derive(Default)]
struct Facts {
    resolution: Vec<(String, Span)>,
    source: Vec<(String, Span)>,
    remux: bool,
    screener: bool,
    video_codec: Vec<(String, Span)>,
    bit_depth: Vec<(String, Span)>,
    hdr: Vec<(String, Span)>,
    audio_codecs: Vec<(String, Span)>,
    channels: Vec<(String, Span)>,
    audio_languages: Vec<(String, Span)>,
    subtitle_languages: Vec<(String, Span)>,
    revision: Vec<(String, Span)>,
    streaming_service: Vec<(String, Span)>,
}

impl Facts {
    fn add(&mut self, fact: ReleaseFact, value: &str, span: Span) {
        let list = match fact {
            ReleaseFact::Source => &mut self.source,
            ReleaseFact::Remux => {
                self.remux = true;
                return;
            }
            ReleaseFact::VideoCodec => &mut self.video_codec,
            ReleaseFact::BitDepth => &mut self.bit_depth,
            ReleaseFact::Hdr => &mut self.hdr,
            ReleaseFact::AudioCodec => &mut self.audio_codecs,
            ReleaseFact::AudioLanguage => &mut self.audio_languages,
            ReleaseFact::SubtitleLanguage => &mut self.subtitle_languages,
            ReleaseFact::Revision => &mut self.revision,
            ReleaseFact::StreamingService => &mut self.streaming_service,
        };
        list.push((value.to_owned(), span));
    }
}

/// Returns one value as resolved with the span that first stated it, several different
/// values as ambiguous, and none as unknown.
fn scalar(found: Vec<(String, Span)>) -> Finding<String> {
    let mut candidates: Vec<String> = Vec::new();
    for (value, _) in &found {
        if !candidates.contains(value) {
            candidates.push(value.clone());
        }
    }
    match (candidates.len(), found.first()) {
        (0, _) | (_, None) => Finding::Unknown,
        (1, Some((value, span))) => Finding::Resolved {
            value: value.clone(),
            evidence: Some(Evidence {
                rule: Rule::ReleaseWord,
                span: *span,
            }),
        },
        _ => Finding::Ambiguous {
            candidates,
            reason: "several_values",
        },
    }
}

/// Returns the markers in name order, each value once with the span that first stated it.
fn markers(found: Vec<(String, Span)>) -> Vec<ReleaseMarker> {
    let mut markers: Vec<ReleaseMarker> = Vec::new();
    for (value, span) in found {
        if !markers.iter().any(|marker| marker.value == value) {
            markers.push(ReleaseMarker { value, span });
        }
    }
    markers
}
