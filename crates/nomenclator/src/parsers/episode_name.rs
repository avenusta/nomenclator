use crate::model::AmbiguityReason;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::data;
use crate::lexical::{self, BracketClass, DateMatch, TokenKind, WordClass};
use crate::model::{EntryKind, Evidence, Finding, Rule, Span};
use crate::parsers::extras::extras_folder_kind;
use crate::parsers::series_name::{SeriesName, TitleParts, is_valid_season, title_parts};
use crate::rules::{BareNumbers, Rules};

/// The series, season and episode facts of one episode path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EpisodeName {
    /// The series title, from its folder or the name itself (T2).
    pub series_name: Finding<String>,
    /// The series year, from the same source as its title.
    pub series_year: Finding<u16>,
    /// The root-relative path of the series folder, or the path of the entry itself if its name
    /// alone gives the series.
    pub series_path: Option<String>,
    /// Another reading of a file-named series. For example, `Show.2020.S01E01` may be `Show`
    /// from 2020.
    pub alternate_identity: Option<AlternateIdentity>,
    /// The season, from the name or its season folder (T7, T9).
    pub season: Finding<u16>,
    /// Episode numbers within the season, with ranges expanded.
    pub episodes: Option<Vec<u32>>,
    /// The first episode that the name states.
    pub first_episode: Option<u32>,
    /// The last episode of a multi-episode name. `None` for a single episode.
    pub last_episode: Option<u32>,
    /// Absolute episode numbers, counted across seasons (T14).
    pub absolute: Option<Vec<u32>>,
    /// An air date (T13).
    pub date: Finding<String>,
    /// The cleaned text after the episode token or air date (T19).
    pub episode_title: Option<String>,
    /// Provider IDs from the bracketed tags, keyed by provider.
    pub provider_ids: BTreeMap<String, Finding<String>>,
    /// The `{key-value}` tags whose keys state no typed fact, keyed by tag key (X29).
    pub tags: BTreeMap<String, Finding<String>>,
    /// How the name refers to its episode. `None` if it does not refer to an episode.
    pub presentation_kind: Option<PresentationKind>,
    /// The special the name states instead of an episode number (X20): `OVA 2`, `12.5`.
    /// The season is 0 and `episodes` is `None`, because the name does not state the
    /// special's number in the season 0 order.
    pub special: Option<Special>,
    /// The release revision after the episode number (`12v2` is 2).
    pub release_version: Option<u8>,
    /// `true` if `END` follows the episode number: the last episode of a release run.
    pub is_final: bool,
    /// `true` if the name has an episode, absolute or date reference.
    pub is_episode: bool,
    /// `true` if any fact has competing interpretations.
    pub ambiguous: bool,
}

/// A series title and year read another way from the same name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlternateIdentity {
    /// The series title.
    pub title: String,
    /// The series year.
    pub year: Option<u16>,
}

/// A special that a name states instead of an episode number (X20).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Special {
    /// The kind of special.
    pub kind: SpecialKind,
    /// The number as written (`2`, `12.5`); `None` if the name states no number.
    pub number: Option<String>,
}

/// A kind of special.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpecialKind {
    /// An original video animation (`OVA`).
    Ova,
    /// An original animation DVD (`OAD`).
    Oad,
    /// An original net animation (`ONA`).
    Ona,
    /// A decimal episode such as `12.5`, usually a recap.
    Recap,
}

/// How a name refers to its episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationKind {
    /// One season and episode.
    Episode,
    /// A range or list of episodes (T15).
    MultiEpisode,
    /// An absolute number across seasons (T14).
    Absolute,
    /// An air date (T13).
    Dated,
    /// A special without an episode number (X20).
    Special,
}

impl Rules {
    /// Reads the series, season and episode facts from the root-relative path of an episode. The
    /// series folder is the parent of the nearest season folder. If there is no season folder,
    /// the series folder is the folder that contains the entry (T1).
    /// One name alone cannot distinguish a file-named series from an episode title before the
    /// code (`The X-Files/Pilot - S01E01`). The function reports the text as `series_name`, and
    /// the interpreter decides from the other files of the folder (T2).
    /// `season_folder_number` is the season that the caller already knows for the folder of the
    /// entry.
    pub fn parse_episode_name(
        &self,
        path: &str,
        kind: EntryKind,
        season_folder_number: Option<u16>,
    ) -> EpisodeName {
        let rules = self;
        read_episode(
            &KnownSeasons::new(),
            rules,
            path,
            kind,
            season_folder_number,
        )
        .name
    }
}

/// Folders that the caller declares as season folders, with their seasons (T20).
pub(crate) type KnownSeasons = std::collections::BTreeMap<String, u16>;

/// The facts of an episode name and the folders where they were found.
pub(crate) struct EpisodeReading {
    pub name: EpisodeName,
    pub layout: EpisodeLayout,
}

/// The folders that contain an episode, by their role (T1, T9, T18).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EpisodeLayout {
    /// The series folder, regardless of which name gives the series. `None` at the library root.
    pub series_folder: Option<String>,
    /// The season folder nearest the entry.
    pub season_folder: Option<String>,
    /// A disc or per-episode folder below the season or series folder (T18).
    pub episode_folder: Option<String>,
    /// The position in the stem of the entry where the release text after the episode reference
    /// and its title starts. `None` if the stem has no reference (T17).
    pub release_remainder_start_in_stem: Option<usize>,
    /// The series text of the file, if it differs from the trusted series folder that contains
    /// its season folder. This text is the title of the episode, unless several episodes repeat
    /// it (T2).
    pub set_aside_file_title: Option<TitleParts>,
}

/// Reads an episode as [`parse_episode_name`] does, and also returns the folders where the
/// series, season and episode were found.
pub(crate) fn read_episode(
    known: &KnownSeasons,
    rules: &Rules,
    path: &str,
    kind: EntryKind,
    season_folder_number: Option<u16>,
) -> EpisodeReading {
    let components: Vec<&str> = path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let Some((&name, folders)) = components.split_last() else {
        return EpisodeReading {
            name: EpisodeName::unresolved(),
            layout: EpisodeLayout::default(),
        };
    };
    // An extras folder at the library root has no owner, so it is an ordinary folder (E2).
    let in_extras_folder = folders.len() > 1
        && folders
            .last()
            .is_some_and(|folder| is_extras_folder(rules, folder));
    let folders = if in_extras_folder {
        &folders[..folders.len() - 1]
    } else {
        folders
    };
    let stem = lexical::stem(name, kind);
    let mut context = FolderContext::read(known, rules, folders);
    let mut reading = read_in_context(
        rules,
        stem,
        folders,
        &context,
        season_folder_number,
        in_extras_folder,
    );
    // `Drama/24/Extras/24 S00E02`: if the series title of a file matches a folder that was
    // read as a season, that folder is the series.
    if let Some(file_key) = reading.file_title_key(rules)
        && let Some(index) = context.season_like_folder_named(rules, folders, &file_key)
    {
        context = FolderContext::rooted_at(known, rules, folders, index);
        reading = read_in_context(
            rules,
            stem,
            folders,
            &context,
            season_folder_number,
            in_extras_folder,
        );
    }
    let folder_path = |index: usize| folders[..=index].join("/");
    let layout = EpisodeLayout {
        series_folder: context.series_folder_index.map(folder_path),
        season_folder: context.season_folder_index.map(folder_path),
        episode_folder: context.nested_episode_folder_index.map(folder_path),
        release_remainder_start_in_stem: (!reading.reference.is_empty()
            && reading.referenced_text_is_stem)
            .then(|| {
                let reference_end = reading.reference.span.1;
                episode_title(rules, reading.referenced_text, reference_end)
                    .map_or(reference_end, |(_, title_end)| title_end)
            }),
        set_aside_file_title: None,
    };
    let (name, set_aside_file_title) =
        reading.into_episode_name(rules, path, stem, folders, &context, season_folder_number);
    EpisodeReading {
        name,
        layout: EpisodeLayout {
            set_aside_file_title,
            ..layout
        },
    }
}

/// The reference read from a name in one folder context, before the series identity is
/// decided.
struct ContextReading<'a> {
    referenced_text: &'a str,
    referenced_text_is_stem: bool,
    reference: Reference,
    file_title: Option<TitleParts>,
}

impl ContextReading<'_> {
    fn file_title_key(&self, rules: &Rules) -> Option<String> {
        self.file_title
            .as_ref()
            .and_then(TitleParts::title_value)
            .map(|title| lexical::title_key(rules, title))
    }
}

fn read_in_context<'a>(
    rules: &Rules,
    stem: &'a str,
    folders: &[&'a str],
    context: &FolderContext,
    season_folder_number: Option<u16>,
    in_extras_folder: bool,
) -> ContextReading<'a> {
    let series_title_key = context.series_folder_index.and_then(|index| {
        rules
            .parse_series_name(folders[index])
            .title
            .resolved_value()
            .map(|title| lexical::title_key(rules, title))
    });
    let known_season = context
        .season
        .as_ref()
        .and_then(|season| season.resolved_value().copied())
        .or(season_folder_number);
    let reading_context = ReadingContext {
        series_title_key: series_title_key.as_deref(),
        known_season,
        in_extras_folder,
    };
    // The text that the reference was read from: the name, or a per-episode folder if the name
    // has no reference (T18).
    let mut referenced_text = stem;
    let mut reference = read_reference(rules, stem, &reading_context);
    if reference.is_empty()
        && let Some(index) = context.nested_episode_folder_index
    {
        referenced_text = folders[index];
        reference = read_reference(rules, referenced_text, &reading_context);
    }
    let file_title = (!reference.is_empty() && reference.span.0 > 0)
        .then(|| title_parts(rules, text_before_reference(referenced_text, &reference), 0))
        .filter(|parts| parts.title.is_some());
    ContextReading {
        referenced_text_is_stem: std::ptr::eq(referenced_text, stem),
        referenced_text,
        reference,
        file_title,
    }
}

impl ContextReading<'_> {
    fn into_episode_name(
        self,
        rules: &Rules,
        path: &str,
        stem: &str,
        folders: &[&str],
        context: &FolderContext,
        season_folder_number: Option<u16>,
    ) -> (EpisodeName, Option<TitleParts>) {
        let ContextReading {
            referenced_text,
            reference,
            file_title,
            ..
        } = self;
        let series_folder = context.series_folder_index.map(|index| {
            (
                folders[..=index].join("/"),
                rules.parse_series_name(folders[index]),
            )
        });
        let season_from_folders = context.season.clone().or_else(|| {
            season_folder_number.map(|season| resolved(season, Rule::SeasonFolder, None))
        });
        let identity = SeriesIdentity::decide(
            rules,
            series_folder.as_ref(),
            context.holds_season_folder(),
            file_title.as_ref(),
            path,
            reference.season_is_stated,
        );

        let season = match reference.season {
            Some((season, rule, span)) => resolved(season, rule, Some(span)),
            None => season_from_folders
                .or_else(|| {
                    identity
                        .from_folder
                        .then(|| {
                            series_folder
                                .as_ref()
                                .map(|(_, series)| series.season.clone())
                        })
                        .flatten()
                })
                .unwrap_or(Finding::Unknown),
        };
        let date = match &reference.date {
            Some(date) if date.candidates.len() == 1 => Finding::Resolved {
                value: date.candidates[0].clone(),
                evidence: Some(Evidence {
                    rule: Rule::Date,
                    span: date.span,
                }),
            },
            Some(date) => Finding::Ambiguous {
                candidates: date.candidates.clone(),
                reason: AmbiguityReason::DateOrderUnknown,
            },
            None => Finding::Unknown,
        };
        let title_after_reference = (!reference.is_empty())
            .then(|| episode_title(rules, referenced_text, reference.span.1))
            .flatten()
            .map(|(title, _)| title);
        // `END` after the number marks the last episode of a release run; it is no title.
        let is_final = title_after_reference.as_deref().is_some_and(|title| {
            rules
                .vocab
                .final_episode_words
                .contains(&lexical::vocabulary_form(title))
        });
        let episode_title = title_after_reference.filter(|_| !is_final).or_else(|| {
            identity
                .set_aside_file_title
                .as_ref()
                .and_then(|parts| parts.title_value().map(str::to_owned))
        });
        let episodes = (!reference.episodes.is_empty()).then(|| reference.episodes.clone());
        let absolute = (!reference.absolute.is_empty()).then(|| reference.absolute.clone());
        let presentation_kind = if reference.episodes.len() > 1 {
            Some(PresentationKind::MultiEpisode)
        } else if episodes.is_some() {
            Some(PresentationKind::Episode)
        } else if absolute.is_some() {
            Some(PresentationKind::Absolute)
        } else if reference.date.is_some() {
            Some(PresentationKind::Dated)
        } else if reference.special.is_some() {
            Some(PresentationKind::Special)
        } else {
            None
        };
        let release_version = (!reference.is_empty())
            .then(|| {
                RELEASE_VERSION_PATTERN
                    .captures(&referenced_text[reference.span.0.min(referenced_text.len())..])
                    .and_then(|captures| captures[1].parse().ok())
            })
            .flatten();
        let ambiguous = [&identity.name, &date]
            .iter()
            .any(|finding| matches!(finding, Finding::Ambiguous { .. }))
            || matches!(season, Finding::Ambiguous { .. });

        let name = EpisodeName {
            series_name: identity.name,
            series_year: identity.year,
            series_path: identity.path,
            alternate_identity: identity.alternate,
            season,
            first_episode: reference.episodes.first().copied(),
            last_episode: (reference.episodes.len() > 1)
                .then(|| reference.episodes.last().copied())
                .flatten(),
            episodes,
            absolute,
            date,
            episode_title,
            provider_ids: lexical::provider_ids(rules, stem, false),
            tags: lexical::custom_tags(rules, stem),
            special: reference.special.clone(),
            release_version,
            is_final,
            is_episode: presentation_kind.is_some(),
            presentation_kind,
            ambiguous,
        };
        (name, identity.set_aside_file_title)
    }
}

impl EpisodeName {
    fn unresolved() -> Self {
        EpisodeName {
            series_name: Finding::Unknown,
            series_year: Finding::Unknown,
            series_path: None,
            alternate_identity: None,
            season: Finding::Unknown,
            episodes: None,
            first_episode: None,
            last_episode: None,
            absolute: None,
            date: Finding::Unknown,
            episode_title: None,
            provider_ids: BTreeMap::new(),
            tags: BTreeMap::new(),
            presentation_kind: None,
            special: None,
            release_version: None,
            is_final: false,
            is_episode: false,
            ambiguous: false,
        }
    }
}

/// Returns the text before the reference that a series title is read from. A run of words
/// joined by underscores that leads into a date is the channel tag of a recording. In that
/// case the title ends at the first underscore after the last space
/// (`Show Name_KTLADT_2018_05_25` → `Show Name`, T21).
fn text_before_reference<'a>(text: &'a str, reference: &Reference) -> &'a str {
    let before = &text[..reference.span.0];
    let starts_with_date = reference
        .date
        .as_ref()
        .is_some_and(|date| date.span.0 == reference.span.0);
    if starts_with_date
        && before.ends_with('_')
        && let Some(last_space) = before.rfind(' ')
        && let Some(underscore) = before[last_space..].find('_')
    {
        return &before[..last_space + underscore];
    }
    before
}

/// Returns the cleaned text after the episode reference (T19), with the position in `stem`
/// where the text ends, after any year that it contains.
/// Returns `None` if release material follows the reference.
fn episode_title(rules: &Rules, stem: &str, reference_end: usize) -> Option<(String, usize)> {
    let text = &stem[reference_end..];
    let starts_with_release_material = lexical::tokenize(rules, text)
        .into_iter()
        .find(|token| token.kind != TokenKind::Dash)
        .is_some_and(|token| match token.kind {
            TokenKind::Bracket { inner } => !matches!(
                lexical::classify_bracket(rules, text, token.span, inner),
                BracketClass::Other | BracketClass::Year(_)
            ),
            _ => matches!(
                lexical::classify_word(rules, &text[token.span.0..token.span.1]),
                WordClass::Technical
            ),
        });
    if starts_with_release_material {
        return None;
    }
    let parts = title_parts(rules, text, reference_end);
    let (mut title, title_span, _) = parts.title?;
    let mut end = parts.year.map_or(title_span.1, |(_, year_span, _)| {
        year_span.1.max(title_span.1)
    });
    // A part number in parentheses directly after the title belongs to the title (T19):
    // `Live Together, Die Alone (1)`.
    if parts.year.is_none()
        && let Some(number) = TITLE_PART_NUMBER_PATTERN.captures(&stem[end..])
    {
        title = format!("{title} ({})", &number[1]);
        end += number[1].len() + 3;
    }
    Some((title, end))
}

static TITLE_PART_NUMBER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^ \((\d{1,2})\)(?:$|[ ._\-\[(])").expect("valid title part pattern")
});

fn resolved(value: u16, rule: Rule, span: Option<Span>) -> Finding<u16> {
    Finding::Resolved {
        value,
        evidence: span.map(|span| Evidence { rule, span }),
    }
}

fn is_extras_folder(rules: &Rules, name: &str) -> bool {
    extras_folder_kind(rules, name).is_some()
}

static EPISODE_FOLDER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])s\d{1,4}e\d{1,5}(?:$|[^a-z0-9])").expect("valid pattern")
});

/// The positions of the season and series folders among the ancestors of an entry.
#[derive(Debug, Default)]
struct FolderContext {
    season_folder_index: Option<usize>,
    season: Option<Finding<u16>>,
    /// A season folder that contains the nearest season folder (`Season 01/Specials/`).
    outer_season_folder_index: Option<usize>,
    series_folder_index: Option<usize>,
    /// A disc or per-episode folder below the season or series folder (T18).
    nested_episode_folder_index: Option<usize>,
}

impl FolderContext {
    /// Reads the folder context of an entry from its ancestor `folders`. The nearest season
    /// folder is the folder of the entry, or the folder above a disc or per-episode folder. It
    /// may be inside one other season folder (`Season 01/Specials/`). The series folder is the
    /// parent of the outermost season folder. If there is no season folder, the series folder is
    /// the folder that contains the entry or its per-episode folder. A folder at the library root
    /// is never a season.
    fn read(known: &KnownSeasons, rules: &Rules, folders: &[&str]) -> Self {
        let mut context = FolderContext::default();
        let Some(last) = folders.len().checked_sub(1) else {
            return context;
        };
        let is_nested_episode_folder = |name: &str| {
            rules.patterns.disc_folder.is_match(name.trim())
                || EPISODE_FOLDER_PATTERN.is_match(name)
        };
        if is_season_like(known, rules, folders, last) {
            context.season_folder_index = Some(last);
        } else if last > 0 && is_nested_episode_folder(folders[last]) {
            if is_season_like(known, rules, folders, last - 1) {
                context.season_folder_index = Some(last - 1);
                context.nested_episode_folder_index = Some(last);
            } else if names_same_series(rules, folders[last], folders[last - 1]) {
                context.nested_episode_folder_index = Some(last);
            }
        }
        context.outer_season_folder_index = context
            .season_folder_index
            .and_then(|index| index.checked_sub(1))
            .filter(|&index| is_season_like(known, rules, folders, index));
        context.season = [
            context.season_folder_index,
            context.outer_season_folder_index,
        ]
        .into_iter()
        .flatten()
        .find_map(|index| season_of(known, rules, folders, index));
        // A season folder at the library root is its own series folder, because no folder
        // above it exists (X1).
        context.series_folder_index = match (
            context
                .outer_season_folder_index
                .or(context.season_folder_index),
            context.nested_episode_folder_index,
        ) {
            (Some(index), _) => Some(index.saturating_sub(1)),
            (None, Some(index)) => Some(index - 1),
            (None, None) => Some(last),
        };
        context
    }

    /// Returns `true` if the series folder contains a season folder below it (T2).
    fn holds_season_folder(&self) -> bool {
        matches!(
            (self.series_folder_index, self.season_folder_index),
            (Some(series), Some(season)) if series < season
        )
    }

    /// Returns the context with the folder at `index` as the series folder, and the season
    /// folder below it, if any, as the season.
    fn rooted_at(known: &KnownSeasons, rules: &Rules, folders: &[&str], index: usize) -> Self {
        let below = index + 1;
        let season_folder_index = (below < folders.len()
            && is_season_like(known, rules, folders, below))
        .then_some(below);
        FolderContext {
            season_folder_index,
            season: season_folder_index.and_then(|season| season_of(known, rules, folders, season)),
            outer_season_folder_index: None,
            series_folder_index: Some(index),
            nested_episode_folder_index: FolderContext::read(known, rules, folders)
                .nested_episode_folder_index
                .filter(|&nested| nested > index),
        }
    }

    /// Returns the index of a folder that was read as a season and whose name or title is the
    /// series title of the file.
    /// Returns `None` if no such folder exists.
    fn season_like_folder_named(
        &self,
        rules: &Rules,
        folders: &[&str],
        file_title_key: &str,
    ) -> Option<usize> {
        [self.outer_season_folder_index, self.season_folder_index]
            .into_iter()
            .flatten()
            .find(|&index| {
                let folder_title = title_parts(rules, folders[index], 0);
                lexical::title_key(rules, folders[index]) == file_title_key
                    || folder_title
                        .title_value()
                        .is_some_and(|title| lexical::title_key(rules, title) == file_title_key)
            })
    }
}

/// Returns `true` if `text` contains an explicit episode token (`S01E02`, `E02`, `1x02`,
/// `SP02`).
pub(crate) fn states_explicit_episode(rules: &Rules, text: &str) -> bool {
    !explicit_candidates(rules, text).0.is_empty()
}

/// Returns `true` if the folder at `folder_path` is a season folder (T9). The folder name is
/// read with its parent as context.
pub(crate) fn is_season_folder(rules: &Rules, folder_path: &str) -> bool {
    let known = &KnownSeasons::new();
    let folders: Vec<&str> = folder_path.split('/').collect();
    is_season_like(known, rules, &folders, folders.len() - 1)
}

/// Returns the series folder of an entry in `folder_path` (T1, T18): the parent of its season
/// folders. Falls back to the folder itself, or to the folder above a per-episode folder.
/// Returns `None` if no folder qualifies.
pub(crate) fn series_folder_of(rules: &Rules, folder_path: &str) -> Option<String> {
    let known = &KnownSeasons::new();
    let folders: Vec<&str> = folder_path.split('/').collect();
    FolderContext::read(known, rules, &folders)
        .series_folder_index
        .map(|index| folders[..=index].join("/"))
}

/// Returns the season that the folder at `index` states, with its parent as context (T9).
/// Returns `None` if the folder states no season.
fn season_of(
    known: &KnownSeasons,
    rules: &Rules,
    folders: &[&str],
    index: usize,
) -> Option<Finding<u16>> {
    if let Some(&season) = known.get(&folders[..=index].join("/")) {
        return Some(Finding::Resolved {
            value: season,
            evidence: None,
        });
    }
    let parent = folders[..index].last().copied();
    let folder = rules.parse_season_folder(folders[index], parent, parent.is_some());
    folder.season.is_resolved().then_some(folder.season)
}

/// Returns `true` if `episode_folder` and `parent` give the same series title, or if
/// `episode_folder` has no title. `Dexter - S01E01` in `Dexter/` is a folder per episode of
/// that series. A release folder with another title is a series folder of its own.
fn names_same_series(rules: &Rules, episode_folder: &str, parent: &str) -> bool {
    let title_key = |name: &str| {
        rules
            .parse_series_name(name)
            .title
            .resolved_value()
            .map(|title| lexical::title_key(rules, title))
    };
    match title_key(episode_folder) {
        Some(key) => title_key(parent).is_some_and(|parent_key| parent_key == key),
        None => true,
    }
}

/// Returns `true` if the folder at `index` is a season folder, or a season word alone
/// (`Season`, `Staffel`) below a series folder.
fn is_season_like(known: &KnownSeasons, rules: &Rules, folders: &[&str], index: usize) -> bool {
    season_of(known, rules, folders, index).is_some()
        || (index > 0
            && rules
                .parse_season_folder(folders[index], Some(folders[index - 1]), true)
                .is_bare_season_folder)
}

/// The series that an episode belongs to, and the source of its name.
struct SeriesIdentity {
    name: Finding<String>,
    year: Finding<u16>,
    path: Option<String>,
    alternate: Option<AlternateIdentity>,
    from_folder: bool,
    /// The series text of the file, if it differs. It is set aside because a trusted series
    /// folder that contains a season folder gives the series (T2).
    set_aside_file_title: Option<TitleParts>,
}

impl SeriesIdentity {
    /// Decides the series identity. The series folder gives the series if it is trusted, if it
    /// agrees with the series title of the file, or if the file has no series title. A trusted
    /// series folder contains a season folder, or has a year, a provider tag or a season.
    /// Otherwise the series title of the file is used (T2).
    fn decide(
        rules: &Rules,
        series_folder: Option<&(String, SeriesName)>,
        has_season_folder: bool,
        file_title: Option<&TitleParts>,
        path: &str,
        reference_states_season: bool,
    ) -> Self {
        let file_key = file_title
            .and_then(TitleParts::title_value)
            .map(|title| lexical::title_key(rules, title));
        let folder_name_of = |path: &'_ str| path.rsplit('/').next().unwrap_or(path).to_owned();
        // If a folder contains a season folder and its whole name reads as a season
        // (`Season 01/Specials/` at the library root), that name is its title.
        let folder_title = series_folder.map(|(folder_path, series)| {
            if series.title.is_resolved() || !has_season_folder {
                series.title.clone()
            } else {
                title_parts(rules, &folder_name_of(folder_path), 0).title_finding()
            }
        });
        if let Some((folder_path, series)) = series_folder
            && let Some(folder_title_finding) = folder_title.as_ref()
            && let Some(folder_title) = folder_title_finding.resolved_value()
        {
            let folder_key = lexical::title_key(rules, folder_title);
            let is_trusted = has_season_folder
                || series.year.is_resolved()
                || !series.provider_ids.is_empty()
                || series.season.is_resolved();
            // `La Série 2/La Série 2 S01E03`: the file repeats the whole folder name, so a
            // season word in the name is part of the title.
            let folder_name = folder_name_of(folder_path);
            let whole_name = title_parts(rules, &folder_name, 0);
            if series.season.is_resolved()
                && let Some(whole_title) = whole_name.title_value()
                && file_key.as_deref() == Some(lexical::title_key(rules, whole_title).as_str())
            {
                return SeriesIdentity {
                    name: whole_name.title_finding(),
                    year: whole_name.year_finding(),
                    path: Some(folder_path.clone()),
                    alternate: None,
                    from_folder: false,
                    set_aside_file_title: None,
                };
            }
            let title_matches = file_key.as_deref() == Some(folder_key.as_str());
            let title_before_year_matches = file_title
                .and_then(|parts| parts.title_before_trailing_year.as_ref())
                .filter(|(title, _)| lexical::title_key(rules, title) == folder_key);
            if is_trusted
                || file_key.is_none()
                || title_matches
                || title_before_year_matches.is_some()
            {
                let year = if series.year.is_resolved() {
                    series.year.clone()
                } else if title_matches {
                    file_title.map_or(Finding::Unknown, TitleParts::year_finding)
                } else if let Some((_, year)) = title_before_year_matches {
                    Finding::Resolved {
                        value: *year,
                        evidence: None,
                    }
                } else {
                    Finding::Unknown
                };
                // Compared in the folded form of title agreement (P4): `Office` agrees with
                // `The Office`.
                let file_title_differs =
                    file_title
                        .and_then(TitleParts::title_value)
                        .is_some_and(|title| {
                            lexical::title_comparison_key(rules, title)
                                != lexical::title_comparison_key(rules, folder_title)
                        })
                        && title_before_year_matches.is_none();
                return SeriesIdentity {
                    name: folder_title_finding.clone(),
                    year,
                    path: Some(folder_path.clone()),
                    alternate: None,
                    from_folder: true,
                    set_aside_file_title: (has_season_folder && file_title_differs)
                        .then(|| file_title.cloned())
                        .flatten(),
                };
            }
        }
        match file_title {
            Some(parts) => SeriesIdentity {
                name: parts.title_finding(),
                year: parts.year_finding(),
                path: Some(path.to_owned()),
                alternate: parts
                    .title_before_trailing_year
                    .as_ref()
                    .filter(|_| reference_states_season)
                    .map(|(title, year)| AlternateIdentity {
                        title: title.clone(),
                        year: Some(*year),
                    }),
                from_folder: false,
                set_aside_file_title: None,
            },
            None => SeriesIdentity {
                name: Finding::Unknown,
                year: Finding::Unknown,
                path: None,
                alternate: None,
                from_folder: false,
                set_aside_file_title: None,
            },
        }
    }
}

/// The context of the entry that reading a reference needs: the series title key, the known
/// season, and whether the entry is in an extras folder.
struct ReadingContext<'a> {
    series_title_key: Option<&'a str>,
    known_season: Option<u16>,
    in_extras_folder: bool,
}

/// The episode reference in a name, and its span.
#[derive(Debug, Default)]
struct Reference {
    span: Span,
    season: Option<(u16, Rule, Span)>,
    /// `true` if the name states the season next to the episode (`S01E02`, `1x02`).
    season_is_stated: bool,
    episodes: Vec<u32>,
    absolute: Vec<u32>,
    date: Option<DateMatch>,
    special: Option<Special>,
}

impl Reference {
    fn is_empty(&self) -> bool {
        self.episodes.is_empty()
            && self.absolute.is_empty()
            && self.date.is_none()
            && self.special.is_none()
    }
}

static DECIMAL_EPISODE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[ ._]-[ ._]*(\d{1,4}\.\d)(?:v\d{1,2})?").expect("valid decimal episode pattern")
});
static SEASON_DASH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)s(\d{1,2})[ ._]*-[ ._]*(\d{1,4})(?:v\d{1,2})?")
        .expect("valid season dash pattern")
});
static RELEASE_VERSION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\dv(\d{1,2})").expect("valid release version pattern"));

/// Returns the special a name states after its first word (X20): an `OVA`, `OAD` or `ONA`
/// marker with an optional number, or a decimal episode number after a dash (`- 12.5`).
/// Returns `None` if the name states none.
fn special_reference(rules: &Rules, text: &str) -> Option<Reference> {
    let first_word_end = text.find([' ', '.', '_']).unwrap_or(text.len());
    let bounded = |start: usize, end: usize| {
        start >= first_word_end
            && lexical::is_boundary_before(text, start)
            && lexical::is_boundary_after(text, end)
    };
    let found = rules
        .patterns
        .animation_special
        .captures_iter(text)
        .find(|captures| {
            let whole = captures.get(0).expect("match");
            bounded(whole.start(), whole.end())
        })
        .map(|captures| {
            let whole = captures.get(0).expect("match");
            let kind = rules
                .vocab
                .animation_special_words
                .get(&lexical::vocabulary_form(&captures[1]))
                .copied()
                .unwrap_or(SpecialKind::Ova);
            let number = captures
                .get(2)
                .map(|number| number.as_str().trim_start_matches('0').to_owned())
                .map(|number| {
                    if number.is_empty() {
                        "0".to_owned()
                    } else {
                        number
                    }
                });
            (Span(whole.start(), whole.end()), Special { kind, number })
        })
        .or_else(|| {
            DECIMAL_EPISODE_PATTERN
                .captures_iter(text)
                .find(|captures| {
                    let whole = captures.get(0).expect("match");
                    lexical::is_boundary_after(text, whole.end())
                })
                .map(|captures| {
                    let number = captures.get(1).expect("number");
                    (
                        Span(number.start(), captures.get(0).expect("match").end()),
                        Special {
                            kind: SpecialKind::Recap,
                            number: Some(number.as_str().trim_start_matches('0').to_owned()),
                        },
                    )
                })
        })?;
    let (span, special) = found;
    Some(Reference {
        span,
        season: Some((0, Rule::SpecialMarker, span)),
        season_is_stated: true,
        special: Some(special),
        ..Reference::default()
    })
}

/// Returns a season and episode written as `S2 - 03` after the series title.
fn season_dash_candidate(text: &str) -> Option<Candidate> {
    SEASON_DASH_PATTERN
        .captures_iter(text)
        .find_map(|captures| {
            let whole = captures.get(0).expect("match");
            if !(lexical::is_boundary_before(text, whole.start())
                && lexical::is_boundary_after(text, whole.end()))
            {
                return None;
            }
            let season: u16 = captures[1].parse().ok()?;
            let digits = &captures[2];
            Some(Candidate {
                tier: Tier::SeasonAndEpisode,
                span: Span(whole.start(), whole.end()),
                season: Some(season),
                numbers: vec![(digits.parse().ok()?, false)],
                first_digits: digits.len(),
                first_has_leading_zero: digits.starts_with('0') && digits.len() > 1,
            })
        })
}

/// The strength of the evidence that a token is an episode reference, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    SeasonAndEpisode,
    EpisodeOnly,
    Cross,
    /// An episode word of a language pack or an overlay, which can also be a title word
    /// (`3x08 Episodio 2`).
    LocalizedEpisodeWord,
    StrongDashNumber,
    BracketNumber,
    LeadingNumber,
    WeakDashNumber,
    NumberAfterTitle,
    BracketedCross,
}

#[derive(Debug, Clone)]
struct Candidate {
    tier: Tier,
    span: Span,
    season: Option<u16>,
    /// The numbers as written. The flag of each number is `true` if a dash or tilde joins it to
    /// the previous number.
    numbers: Vec<(u32, bool)>,
    /// The digit count of the first number as written, for zero-padding.
    first_digits: usize,
    first_has_leading_zero: bool,
}

fn read_reference(rules: &Rules, stem: &str, context: &ReadingContext) -> Reference {
    let masked = lexical::mask_technical(rules, stem);
    let text = masked.as_str();
    let date = lexical::find_date(rules, text);
    let (mut candidates, blocked) = explicit_candidates(rules, text);
    if candidates.is_empty()
        && date.is_none()
        && let Some(special) = special_reference(rules, text)
    {
        return special;
    }
    if candidates.is_empty() {
        candidates.extend(season_dash_candidate(text));
    }
    if context.in_extras_folder {
        candidates.retain(|candidate| candidate.season.is_some());
    }
    if let Some(date) = date {
        // Next to a date, only an explicit token counts. Elsewhere such text is part of the title
        // (`2x4`).
        let adjacent = candidates.into_iter().find(|candidate| {
            let between = if candidate.span.0 >= date.span.1 {
                &text[date.span.1..candidate.span.0]
            } else {
                &text[candidate.span.1.min(date.span.0)..date.span.0]
            };
            between.chars().all(|c| " -._()[]".contains(c))
        });
        let mut reference = Reference {
            span: widen_to_enclosing_bracket(text, date.span),
            date: Some(date.clone()),
            ..Reference::default()
        };
        if let Some(candidate) = adjacent {
            let span = Span(
                date.span.0.min(candidate.span.0),
                date.span.1.max(candidate.span.1),
            );
            reference = reference_from(text, candidate, context);
            reference.span = span;
            reference.date = Some(date);
        }
        return reference;
    }
    if !context.in_extras_folder {
        candidates.extend(
            bare_number_candidates(rules, text, context)
                .into_iter()
                .filter(|candidate| !overlaps_any(&blocked, candidate.span)),
        );
    }
    let Some(chosen) = choose(candidates, text, context) else {
        return Reference::default();
    };
    reference_from(text, chosen, context)
}

fn choose(
    mut candidates: Vec<Candidate>,
    text: &str,
    context: &ReadingContext,
) -> Option<Candidate> {
    candidates.sort_by_key(|candidate| (candidate.tier, candidate.span.0));
    let first = candidates.first()?.clone();
    // ` - 2 - 05` under season 2: the first number is the season, the second the episode.
    if first.tier == Tier::StrongDashNumber
        && first.numbers.len() == 1
        && context.known_season == Some(first.numbers[0].0 as u16)
        && let Some(next) = candidates.iter().find(|candidate| {
            candidate.tier == Tier::StrongDashNumber
                && candidate.span.0 > first.span.1
                && text[first.span.1..candidate.span.0].trim() == "-"
        })
    {
        return Some(next.clone());
    }
    Some(first)
}

/// Converts a chosen token into a season, episodes or absolute numbers. A number of 100 or
/// more is season-coded (`301` → S03E01) only if the known season matches its leading
/// digits. Otherwise it is absolute (T6). A leading `1-05` outside a season folder is S01E05.
fn reference_from(text: &str, candidate: Candidate, context: &ReadingContext) -> Reference {
    let states_episode_only = matches!(
        candidate.tier,
        Tier::EpisodeOnly | Tier::LocalizedEpisodeWord
    );
    let span = widen_to_enclosing_bracket(text, candidate.span);
    let mut reference = Reference {
        span,
        ..Reference::default()
    };
    if let Some(season) = candidate.season {
        reference.season = Some((season, Rule::EpisodeToken, candidate.span));
        reference.season_is_stated = !states_episode_only;
        reference.episodes = expand(&candidate.numbers);
        return reference;
    }
    if states_episode_only {
        reference.episodes = expand(&candidate.numbers);
        return reference;
    }
    let first = candidate.numbers[0].0;
    if candidate.tier == Tier::LeadingNumber
        && context.known_season.is_none()
        && candidate.numbers.len() == 2
        && candidate.numbers[1].1
        && first <= 99
    {
        reference.season = Some((first as u16, Rule::EpisodeToken, candidate.span));
        reference.season_is_stated = true;
        reference.episodes = vec![candidate.numbers[1].0];
        return reference;
    }
    let numbers = within_one_hundred(&candidate.numbers);
    if first < 100 {
        reference.episodes = expand(&numbers);
        return reference;
    }
    let is_season_coded = !candidate.first_has_leading_zero
        && candidate.first_digits <= 4
        && context.known_season == Some((first / 100) as u16);
    if is_season_coded {
        let coded: Vec<(u32, bool)> = numbers
            .iter()
            .map(|(number, joined)| (number % 100, *joined))
            .collect();
        reference.season = Some(((first / 100) as u16, Rule::CompactNumber, candidate.span));
        reference.episodes = expand(&coded);
    } else {
        reference.absolute = expand(&numbers);
    }
    reference
}

/// Returns only the first number of a pair of numbers of 100 or more whose hundreds differ.
/// Such a pair is a range only within one hundred (`301-305`). Returns other lists unchanged.
fn within_one_hundred(numbers: &[(u32, bool)]) -> Vec<(u32, bool)> {
    match numbers {
        [(first, _), (second, true)] if *first >= 100 && first / 100 != second / 100 => {
            vec![(*first, false)]
        }
        _ => numbers.to_vec(),
    }
}

/// Expands a pair joined by a dash or tilde into an inclusive range. Keeps any other list as
/// written, up to the first number that does not increase (T15).
fn expand(numbers: &[(u32, bool)]) -> Vec<u32> {
    match numbers {
        [(first, _), (last, true)] if last > first => (*first..=*last).collect(),
        _ => {
            let mut episodes: Vec<u32> = Vec::new();
            for (number, _) in numbers {
                if episodes.last().is_some_and(|previous| number <= previous) {
                    break;
                }
                episodes.push(*number);
            }
            episodes
        }
    }
}

/// Widens `span` to include the enclosing brackets, so `[1x02]` and `( 1x02 )` cover their
/// brackets. Returns `span` unchanged if no brackets enclose it.
fn widen_to_enclosing_bracket(text: &str, span: Span) -> Span {
    let before = text[..span.0].trim_end_matches(' ');
    let after = text[span.1..].trim_start_matches(' ');
    if (before.ends_with('[') && after.starts_with(']'))
        || (before.ends_with('(') && after.starts_with(')'))
    {
        Span(before.len() - 1, text.len() - after.len() + 1)
    } else {
        span
    }
}

static SEASON_EPISODE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)s ?(\d{1,4})(?:[ ._]?(?:x[ ._]?)?e ?|x)(\d{1,6})")
        .expect("valid season episode pattern")
});
static CROSS_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d{1,4})x(\d{1,6})").expect("valid cross pattern"));

/// Finds the tokens that state an episode: `S01E02`, `Season 1 Episode 2`, `SP02`, `E02`,
/// `1x02`. Also returns the spans of such tokens whose season is invalid (`S999E01`). No text
/// inside those spans is read as an episode.
fn explicit_candidates(rules: &Rules, text: &str) -> (Vec<Candidate>, Vec<Span>) {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut blocked: Vec<Span> = Vec::new();
    let overlaps = |candidates: &[Candidate], start: usize, end: usize| {
        candidates
            .iter()
            .any(|candidate| start < candidate.span.1 && candidate.span.0 < end)
    };
    for token in &rules.patterns.episode_tokens {
        for captures in token.pattern.captures_iter(text) {
            let whole = captures.get(0).expect("match");
            if !lexical::is_boundary_before(text, whole.start())
                || !lexical::is_boundary_after(text, whole.end())
                || overlaps(&candidates, whole.start(), whole.end())
            {
                continue;
            }
            let season = if token.states_season {
                match valid_season(rules, &captures["season"]) {
                    Some(season) => Some(season),
                    None => continue,
                }
            } else {
                None
            };
            let tier = if token.states_season {
                Tier::SeasonAndEpisode
            } else {
                Tier::EpisodeOnly
            };
            let episode = captures.name("episode").expect("episode group");
            candidates.extend(run_candidate(
                rules,
                text,
                tier,
                whole.start(),
                episode,
                Style::Episode,
                season,
            ));
        }
    }
    for pattern in [
        &rules.patterns.season_episode_words,
        &*SEASON_EPISODE_PATTERN,
    ] {
        for captures in pattern.captures_iter(text) {
            let whole = captures.get(0).expect("match");
            if !lexical::is_boundary_before(text, whole.start())
                || overlaps(&candidates, whole.start(), whole.end())
            {
                continue;
            }
            let Some(season) = valid_season(rules, &captures[1]) else {
                blocked.push(Span(whole.start(), whole.end()));
                continue;
            };
            let episode = captures.get(2).expect("episode group");
            if let Some(candidate) = run_candidate(
                rules,
                text,
                Tier::SeasonAndEpisode,
                whole.start(),
                episode,
                Style::Episode,
                Some(season),
            ) {
                candidates.push(candidate);
            }
        }
    }
    for captures in rules.patterns.special_prefix.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        if lexical::is_boundary_before(text, whole.start())
            && lexical::is_boundary_after(text, whole.end())
            && !overlaps(&candidates, whole.start(), whole.end())
        {
            let number = captures.get(1).expect("number");
            candidates.extend(run_candidate(
                rules,
                text,
                Tier::SeasonAndEpisode,
                whole.start(),
                number,
                Style::Episode,
                Some(0),
            ));
        }
    }
    for captures in rules.patterns.episode_word.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        let word = &captures[1];
        let separated = whole.len() > word.len() + captures[2].len();
        let starts_episode_word = is_episode_word_boundary(text, whole.start());
        if !starts_episode_word
            || (word.len() == 1 && separated)
            || overlaps(&candidates, whole.start(), whole.end())
        {
            continue;
        }
        let number = captures.get(2).expect("number");
        let folded_word = lexical::vocabulary_form(word);
        let tier = if folded_word == data::EPISODE_OUTPUT_LETTER
            || data::EPISODE_BASE_WORDS.contains(&folded_word.as_str())
        {
            Tier::EpisodeOnly
        } else {
            Tier::LocalizedEpisodeWord
        };
        candidates.extend(run_candidate(
            rules,
            text,
            tier,
            whole.start(),
            number,
            Style::Episode,
            None,
        ));
    }
    let brackets = bracket_spans(rules, text);
    for captures in CROSS_PATTERN.captures_iter(text) {
        let whole = captures.get(0).expect("match");
        if !lexical::is_boundary_before(text, whole.start())
            || overlaps(&candidates, whole.start(), whole.end())
        {
            continue;
        }
        let Some(season) = valid_season(rules, &captures[1]) else {
            continue;
        };
        let is_bracketed = brackets
            .iter()
            .any(|bracket| bracket.0 <= whole.start() && whole.end() <= bracket.1);
        let tier = if is_bracketed {
            Tier::BracketedCross
        } else {
            Tier::Cross
        };
        let episode = captures.get(2).expect("episode group");
        candidates.extend(run_candidate(
            rules,
            text,
            tier,
            whole.start(),
            episode,
            Style::Cross,
            Some(season),
        ));
    }
    candidates.retain(|candidate| !overlaps_any(&blocked, candidate.span));
    (candidates, blocked)
}

fn overlaps_any(spans: &[Span], span: Span) -> bool {
    spans
        .iter()
        .any(|other| span.0 < other.1 && other.0 < span.1)
}

/// Returns `true` if an episode word such as `E02` can start a token at `position`: after a
/// separator or an opening bracket, or after a dash that a space precedes. In `1080p-E02`
/// the dash directly follows `1080p`, so `E02` does not start a token.
fn is_episode_word_boundary(text: &str, position: usize) -> bool {
    let mut before = text[..position].chars().rev();
    match before.next() {
        None => true,
        Some(' ' | '.' | '_' | '[' | '(') => true,
        Some(dash) if lexical::DASHES.contains(&dash) => before.next().is_none_or(|c| c == ' '),
        _ => false,
    }
}

fn valid_season(rules: &Rules, digits: &str) -> Option<u16> {
    let season: u32 = digits.parse().ok()?;
    is_valid_season(rules, season).then_some(season as u16)
}

/// The continuation grammar of a multi-episode name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `S01E02E03`, `S01E02-E03`, `S01E02-03`, `E01~E16`.
    Episode,
    /// `1x02x03`, `1x02-03`, `1x02-x03`, `1x02-1x03`, `1x02 - 1x03`, `1x02-E03`.
    Cross,
}

static EPISODE_CONTINUATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:([-~]?)(?:s(\d{1,4}))?e(\d{1,6})|([-~])(\d{1,6}))")
        .expect("valid continuation pattern")
});
static CROSS_CONTINUATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:x(\d{1,6})|( ?[-~] ?)(?:(\d{1,4})x|x|e)?(\d{1,6}))")
        .expect("valid continuation pattern")
});
static PART_LETTER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[ab]").expect("valid part letter pattern"));
static CODEC_GLUE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^x26[45]").expect("valid codec pattern"));

/// Reads one continuation at `position`. Returns whether a dash or tilde joins it, its season
/// if restated, its number, and the position where it ends.
/// Returns `None` if no valid continuation starts at `position`.
fn continuation(
    rules: &Rules,
    text: &str,
    position: usize,
    style: Style,
) -> Option<(bool, Option<u16>, u32, usize)> {
    let rest = &text[position..];
    match style {
        Style::Episode => {
            let captures = EPISODE_CONTINUATION_PATTERN.captures(rest)?;
            let end = position + captures.get(0)?.end();
            if let Some(number) = captures.get(3) {
                let joined = captures.get(1).is_some_and(|joiner| !joiner.is_empty());
                let season = captures
                    .get(2)
                    .map(|season| valid_season(rules, season.as_str()));
                let season = match season {
                    Some(None) => return None,
                    Some(Some(season)) => Some(season),
                    None => None,
                };
                Some((joined, season, episode_number(number.as_str())?, end))
            } else {
                Some((true, None, episode_number(&captures[5])?, end))
            }
        }
        Style::Cross => {
            let captures = CROSS_CONTINUATION_PATTERN.captures(rest)?;
            let end = position + captures.get(0)?.end();
            if let Some(number) = captures.get(1) {
                return Some((false, None, episode_number(number.as_str())?, end));
            }
            let joiner = captures.get(2)?.as_str();
            let number = captures.get(4)?.as_str();
            let after_joiner = &rest[joiner.len()..];
            let is_bare_x = after_joiner.starts_with(['x', 'X']);
            if is_bare_x && (number == "264" || number == "265") {
                return None;
            }
            // A spaced dash continues only with a cross (`- 02x04`, `- x04`), never `- 04`.
            let has_season = captures.get(3).is_some();
            if joiner.contains(' ') && !(has_season || is_bare_x) {
                return None;
            }
            let season = match captures.get(3) {
                Some(season) => Some(valid_season(rules, season.as_str())?),
                None => None,
            };
            Some((true, season, episode_number(number)?, end))
        }
    }
}

/// Parses `digits` as an episode number. Episodes have at most five digits (T16).
/// Returns `None` if `digits` has more than five digits.
fn episode_number(digits: &str) -> Option<u32> {
    (digits.len() <= 5).then(|| digits.parse().ok()).flatten()
}

/// Returns the end of the suffix after a number: an optional `v2`, `END` or part letter that
/// a word boundary follows.
/// Returns `position` if the suffix is not followed by a word boundary.
fn after_suffix(rules: &Rules, text: &str, position: usize) -> usize {
    let mut end = position
        + rules
            .patterns
            .episode_suffix
            .find(&text[position..])
            .map_or(0, |found| found.end());
    if !lexical::is_boundary_after(text, end)
        && let Some(letter) = PART_LETTER_PATTERN.find(&text[end..])
        && lexical::is_boundary_after(text, end + letter.end())
    {
        end += letter.end();
    }
    if lexical::is_boundary_after(text, end) {
        end
    } else {
        position
    }
}

fn is_terminated(rules: &Rules, text: &str, position: usize, style: Style) -> bool {
    lexical::is_boundary_after(text, position)
        || CODEC_GLUE_PATTERN.is_match(&text[position..])
        || continuation(rules, text, position, style).is_some()
}

fn run_candidate(
    rules: &Rules,
    text: &str,
    tier: Tier,
    start: usize,
    first_number: regex::Match<'_>,
    style: Style,
    season: Option<u16>,
) -> Option<Candidate> {
    let first_digits = first_number.as_str();
    let first = episode_number(first_digits)?;
    let mut end = after_suffix(rules, text, first_number.end());
    if !is_terminated(rules, text, end, style) {
        return None;
    }
    let mut numbers = vec![(first, false)];
    while let Some((joined, restated_season, number, continuation_end)) =
        continuation(rules, text, end, style)
    {
        let next_end = after_suffix(rules, text, continuation_end);
        let crosses_season = restated_season.is_some_and(|restated| Some(restated) != season);
        if crosses_season || !is_terminated(rules, text, next_end, style) {
            break;
        }
        numbers.push((number, joined));
        end = next_end;
    }
    Some(Candidate {
        tier,
        span: Span(start, end),
        season,
        numbers,
        first_digits: first_digits.len(),
        first_has_leading_zero: first_digits.starts_with('0') && first_digits.len() > 1,
    })
}

fn bracket_spans(rules: &Rules, text: &str) -> Vec<Span> {
    lexical::tokenize(rules, text)
        .into_iter()
        .filter(|token| matches!(token.kind, TokenKind::Bracket { .. }))
        .map(|token| token.span)
        .collect()
}

static NUMBER_RUN_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+(?:-\d+)*").expect("valid number run pattern"));
static VERSION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^v\d{1,2}").expect("valid version pattern"));

/// Finds bare episode numbers (T6): a leading number, ` - NN`, `[NN]`, and a number after the
/// title. Years, numbers inside the series title and number runs that are not ranges (`7-11`)
/// are never episodes.
fn bare_number_candidates(rules: &Rules, text: &str, context: &ReadingContext) -> Vec<Candidate> {
    // Under the season-folders-only preset a bare number needs a season folder (T6, X12).
    if rules.vocab.presets.bare_numbers == BareNumbers::SeasonFoldersOnly
        && context.known_season.is_none()
    {
        return Vec::new();
    }
    let brackets = bracket_spans(rules, text);
    let title_start = text.len()
        - lexical::tokenize(rules, text)
            .into_iter()
            .find(|token| !matches!(token.kind, TokenKind::Bracket { .. }))
            .map_or(0, |token| text.len() - token.span.0);
    let series_title_end = context
        .series_title_key
        .and_then(|key| lexical::prefix_with_title_key(rules, &text[title_start..], key))
        .map(|length| title_start + length)
        .filter(|&end| lexical::is_boundary_after(text, end));
    let mut candidates = Vec::new();
    let mut skip_until = 0;
    for run in NUMBER_RUN_PATTERN.find_iter(text) {
        if run.start() < skip_until {
            continue;
        }
        let end = run.end()
            + VERSION_PATTERN
                .find(&text[run.end()..])
                .map_or(0, |version| version.end());
        if !lexical::is_boundary_before(text, run.start())
            || !lexical::is_boundary_after(text, end)
            || series_title_end.is_some_and(|title_end| run.start() < title_end)
        {
            continue;
        }
        let parts: Vec<&str> = run.as_str().split('-').collect();
        if parts.len() == 1 && lexical::parse_year(rules, parts[0]).is_some() {
            continue;
        }
        let Some(values) = parts
            .iter()
            .map(|part| episode_number(part))
            .collect::<Option<Vec<u32>>>()
        else {
            continue;
        };
        let is_increasing = values.windows(2).all(|pair| pair[1] > pair[0]);
        if !is_increasing || (values.len() == 2 && parts[0].len() > 4) {
            continue;
        }
        let numbers: Vec<(u32, bool)> = values
            .iter()
            .enumerate()
            .map(|(index, value)| (*value, index > 0))
            .collect();
        let is_whole_bracket_group =
            run.start() > 0 && brackets.contains(&Span(run.start() - 1, end + 1));
        let tier = if is_whole_bracket_group {
            Tier::BracketNumber
        } else if run.start() == title_start && series_title_end.is_none() {
            Tier::LeadingNumber
        } else if follows_spaced_dash(text, run.start()) {
            if is_strong_dash_number_end(rules, text, end) {
                Tier::StrongDashNumber
            } else if text[end..]
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
            {
                // ` - 12 00 AM`: a number followed by another number is a time, not episodes.
                let next_start = end + (text[end..].len() - text[end..].trim_start().len());
                skip_until = next_start
                    + text[next_start..]
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(text.len() - next_start);
                continue;
            } else {
                Tier::WeakDashNumber
            }
        } else if brackets
            .iter()
            .any(|bracket| bracket.0 < run.start() && end < bracket.1)
        {
            continue;
        } else {
            Tier::NumberAfterTitle
        };
        candidates.push(Candidate {
            tier,
            span: Span(run.start(), end),
            season: None,
            numbers,
            first_digits: parts[0].len(),
            first_has_leading_zero: parts[0].starts_with('0') && parts[0].len() > 1,
        });
    }
    candidates
}

fn follows_spaced_dash(text: &str, position: usize) -> bool {
    let before = text[..position].trim_end_matches(' ');
    before.len() < position && before.ends_with(lexical::DASHES)
}

/// Returns `true` if the number in ` - 12` ends the name, or if another dash, a bracket group
/// or release words follow it.
fn is_strong_dash_number_end(rules: &Rules, text: &str, position: usize) -> bool {
    let rest = text[position..].trim_start_matches([' ', '.', '_']);
    if rest.is_empty()
        || rest.starts_with(lexical::DASHES)
        || rest.starts_with(['[', '(', lexical::MASK as char])
    {
        return true;
    }
    let next_word = rest
        .split([' ', '.', '_', '-', '['])
        .next()
        .unwrap_or_default();
    matches!(
        lexical::classify_word(rules, next_word),
        WordClass::Technical | WordClass::TitleLike
    )
}
