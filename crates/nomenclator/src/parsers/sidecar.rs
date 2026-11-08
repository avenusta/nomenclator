use serde::Serialize;

use crate::lexical;
use crate::model::{EntryKind, Evidence, Finding, Rule, SidecarFlag, SidecarKind, Span};
use crate::rules::Rules;

/// The result of a sidecar check: whether a sidecar belongs to a video, and the track facts
/// that its name gives (S3, S4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sidecar {
    /// `true` if the name belongs to the video: the file type is right, and the name starts with
    /// the full stem of the video followed by a delimiter or nothing.
    pub matches: bool,
    /// The BCP 47 tag. The evidence span is the raw token. Two different languages are ambiguous.
    pub language: Finding<String>,
    /// Track flags, in canonical order.
    pub flags: Vec<SidecarFlag>,
    /// The tokens after the video stem that are neither language nor flag, joined by `.`.
    pub title: Finding<String>,
}

/// One `.`-separated word after the video stem.
struct Segment<'a> {
    text: &'a str,
    span: Span,
}

impl Rules {
    /// Checks whether `name` is a sidecar of `video_name`, and reads the language, flags and
    /// title in the name.
    pub fn parse_sidecar(&self, name: &str, video_name: &str, kind: SidecarKind) -> Sidecar {
        let rules = self;
        let unmatched = Sidecar {
            matches: false,
            language: Finding::Unknown,
            flags: Vec::new(),
            title: Finding::Unknown,
        };
        let has_sidecar_extension = has_extension_for(rules, name, kind);
        let sidecar_stem = lexical::stem(name, EntryKind::File);
        let video_stem = lexical::stem(video_name, EntryKind::File);
        let Some(stem_end) = lexical::caseless_prefix_length(sidecar_stem, video_stem)
            .filter(|_| has_sidecar_extension && !video_stem.is_empty())
        else {
            return unmatched;
        };
        let rest = &sidecar_stem[stem_end..];

        if kind == SidecarKind::Image {
            let is_still = rest.is_empty()
                || rules
                    .vocab
                    .still_image_suffixes
                    .contains(&lexical::vocabulary_form(rest));
            return Sidecar {
                matches: is_still,
                ..unmatched
            };
        }
        if rest.is_empty() {
            return Sidecar {
                matches: true,
                ..unmatched
            };
        }
        if !rest.starts_with('.') {
            return unmatched;
        }

        let video_extension = lexical::extension(video_name, EntryKind::File);
        let mut segments = Vec::new();
        let mut start = stem_end + 1;
        for text in rest[1..].split('.') {
            let span = Span(start, start + text.len());
            start = span.1 + 1;
            let is_video_extension =
                video_extension.as_deref() == Some(&lexical::fold_case_text(text));
            if !text.is_empty() && !is_video_extension {
                segments.push(Segment { text, span });
            }
        }
        read_segments(rules, &segments)
    }
}

/// Reads the language, flags and title of a sidecar whose name has no video stem, as in
/// `Subs/English.srt` or `Subs/2_French.sdh.srt`. Reads every `.`-separated word after a
/// leading track number (`2_`). `matches` is `true` only if the file type is right. The caller
/// decides whether the name is descriptor-only, with no title words (S3).
pub(crate) fn parse_sidecar_descriptors(rules: &Rules, name: &str, kind: SidecarKind) -> Sidecar {
    let stem = lexical::stem(name, EntryKind::File);
    if !has_extension_for(rules, name, kind) {
        return Sidecar {
            matches: false,
            language: Finding::Unknown,
            flags: Vec::new(),
            title: Finding::Unknown,
        };
    }
    let track_number_length = stem
        .find(|character: char| !character.is_ascii_digit())
        .filter(|&digits| digits > 0 && stem[digits..].starts_with(['_', ' ', '-']))
        .map_or(0, |digits| digits + 1);
    let mut segments = Vec::new();
    let mut start = track_number_length;
    for text in stem[track_number_length..].split('.') {
        let span = Span(start, start + text.len());
        start = span.1 + 1;
        if !text.is_empty() {
            segments.push(Segment { text, span });
        }
    }
    read_segments(rules, &segments)
}

fn has_extension_for(rules: &Rules, name: &str, kind: SidecarKind) -> bool {
    let extensions = match kind {
        SidecarKind::Subtitle => &rules.vocab.subtitle_extensions,
        SidecarKind::Audio => &rules.vocab.audio_extensions,
        SidecarKind::Image => &rules.vocab.image_extensions,
    };
    lexical::extension(name, EntryKind::File)
        .is_some_and(|extension| extensions.contains(&extension))
}

/// Sorts segments into flags, languages and title words. A word such as `hi` is the
/// hearing-impaired flag if another language is present. If no other language is present,
/// the first such word is its language (Hindi for `hi`) and a later one is the
/// hearing-impaired flag.
fn read_segments(rules: &Rules, segments: &[Segment]) -> Sidecar {
    let mut flags = Vec::new();
    let mut languages: Vec<(String, Span)> = Vec::new();
    let mut title_segments: Vec<&Segment> = Vec::new();
    let mut hearing_impaired_or_language: Vec<(String, Span)> = Vec::new();
    for segment in segments {
        if let Some(flag) = lexical::sidecar_flag(rules, segment.text) {
            flags.push(flag);
        } else if let Some(tag) = rules
            .vocab
            .hearing_impaired_or_language_words
            .get(&lexical::vocabulary_form(segment.text))
        {
            hearing_impaired_or_language.push((tag.clone(), segment.span));
        } else if let Some(tag) = lexical::language_tag(rules, segment.text) {
            languages.push((tag, segment.span));
        } else {
            title_segments.push(segment);
        }
    }
    let mut hearing_impaired_words = hearing_impaired_or_language.into_iter();
    if languages.is_empty()
        && let Some(language) = hearing_impaired_words.next()
    {
        languages.push(language);
    }
    if hearing_impaired_words.next().is_some() {
        flags.push(SidecarFlag::HearingImpaired);
    }
    flags.sort();
    flags.dedup();

    Sidecar {
        matches: true,
        language: language_finding(languages),
        flags,
        title: title_finding(&title_segments),
    }
}

fn language_finding(languages: Vec<(String, Span)>) -> Finding<String> {
    let mut candidates: Vec<String> = Vec::new();
    for (tag, _) in &languages {
        if !candidates.contains(tag) {
            candidates.push(tag.clone());
        }
    }
    match (candidates.len(), languages.into_iter().next()) {
        (1, Some((value, span))) => Finding::Resolved {
            value,
            evidence: Some(Evidence {
                rule: Rule::LanguageToken,
                span,
            }),
        },
        (0, _) | (_, None) => Finding::Unknown,
        _ => Finding::Ambiguous {
            candidates,
            reason: "several_languages",
        },
    }
}

/// Returns the title words joined by `.`. The evidence has a span only if the words are
/// contiguous in the name.
/// Returns `Unknown` if there are no title words.
fn title_finding(title_segments: &[&Segment]) -> Finding<String> {
    let (Some(first), Some(last)) = (title_segments.first(), title_segments.last()) else {
        return Finding::Unknown;
    };
    let value = title_segments
        .iter()
        .map(|segment| segment.text)
        .collect::<Vec<_>>()
        .join(".");
    let span = Span(first.span.0, last.span.1);
    let is_contiguous = span.1 - span.0 == value.len();
    Finding::Resolved {
        value,
        evidence: is_contiguous.then_some(Evidence {
            rule: Rule::SidecarTitleTokens,
            span,
        }),
    }
}
