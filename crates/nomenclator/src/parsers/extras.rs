//! Classifies whether a file or folder is an extra, its kind, and the folder that owns it.
//!
//! An extras folder needs an owner. A vocabulary folder counts only if its parent contains
//! feature media among the supplied `context_paths` (E2). Suffixes and exact names need
//! no owner (E3-E5).

use serde::{Deserialize, Serialize};

use crate::lexical;
use crate::model::{EntryKind, Evidence, ExtraKind, Finding, Rule, Span};
use crate::parsers::episode_name::{is_season_folder, series_folder_of, states_explicit_episode};
use crate::parsers::file_types::{file_name, parent_folder};
use crate::rules::Rules;

/// The kind of library that a path belongs to. Each kind has its own pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum LibraryType {
    /// A movies library.
    Movies,
    /// A TV library.
    Tv,
}

/// The extra classification of an entry: whether it is an extra, its kind, and its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtraClassification {
    /// The kind of the extra. `Unknown` if the entry is not an extra.
    pub extra: Finding<ExtraKind>,
    /// The own title of the extra: its name without an extras suffix, with separators cleaned.
    pub title: Option<String>,
    /// The folder whose title the extra belongs to. `None` at the library root.
    pub owner: Option<String>,
    /// The series folder, in a TV library.
    pub series_path: Option<String>,
    /// The owned extras folder that contains the entry, if any.
    pub supplemental_dir: Option<String>,
}

impl Rules {
    /// Classifies one root-relative `path`. `context_paths` are the other files in scope;
    /// without them only the path itself is known, so no folder can be shown to own features.
    pub fn classify_extra(
        &self,
        path: &str,
        kind: EntryKind,
        library: Option<LibraryType>,
        context_paths: &[String],
    ) -> ExtraClassification {
        let rules = self;
        let own_path = path.trim_end_matches('/').to_owned();
        let context = ExtrasContext::new(library, context_paths.iter().chain([&own_path]));
        classify_extra_in(rules, path, kind, &context)
    }
}

/// Classifies one entry as `classify_extra` does, against a context that is built once for
/// many entries. The entry itself must be among the paths of the context.
pub(crate) fn classify_extra_in(
    rules: &Rules,
    path: &str,
    kind: EntryKind,
    context: &ExtrasContext,
) -> ExtraClassification {
    let path = path.trim_end_matches('/');
    let library = context.library;
    let folder_extra = extras_folder(rules, path, kind, context)
        .filter(|found| kind == EntryKind::Directory || has_extension_for(rules, path, found.kind));
    let (extra, owner, supplemental_dir, title) = match folder_extra {
        Some(found) => {
            let title = (kind == EntryKind::File).then(|| {
                let stem = lexical::stem(file_name(path), kind);
                let title = extras_suffix(rules, stem).map_or(stem, |suffix| suffix.title);
                lexical::display_title_among(rules, title, context.has_spaced_sibling(path))
            });
            (
                resolved(found.kind, Rule::ExtrasFolder, found.evidence_span),
                parent_folder(found.folder_path),
                Some(found.folder_path.to_owned()),
                title,
            )
        }
        None => match name_extra(rules, path, kind, library) {
            Some(found) => (
                resolved(found.kind, found.rule, found.evidence_span),
                parent_folder(path),
                None,
                found.title,
            ),
            None => (Finding::Unknown, None, None, None),
        },
    };

    let series_path = if library == Some(LibraryType::Tv) {
        let start = match extra {
            Finding::Resolved { .. } => owner,
            _ => parent_folder(path),
        };
        start.and_then(|folder| series_folder(rules, folder))
    } else {
        None
    };

    ExtraClassification {
        extra,
        title,
        owner: owner.map(str::to_owned),
        series_path,
        supplemental_dir,
    }
}

fn resolved(kind: ExtraKind, rule: Rule, span: Span) -> Finding<ExtraKind> {
    Finding::Resolved {
        value: kind,
        evidence: Some(Evidence { rule, span }),
    }
}

/// Returns `true` if `path` has an extension that fits `kind`. Theme songs are audio. Every
/// other extra is video.
fn has_extension_for(rules: &Rules, path: &str, kind: ExtraKind) -> bool {
    match kind {
        ExtraKind::ThemeSong => rules.is_audio_file(path).result,
        _ => rules.is_video_file(path).result,
    }
}

struct FolderExtra<'a> {
    kind: ExtraKind,
    folder_path: &'a str,
    evidence_span: Span,
}

/// Finds the shallowest owned vocabulary folder that contains the entry. A directory entry
/// may itself be that folder. The deepest vocabulary folder at or below it gives the kind.
fn extras_folder<'a>(
    rules: &Rules,
    path: &'a str,
    kind: EntryKind,
    context: &ExtrasContext,
) -> Option<FolderExtra<'a>> {
    let mut folders = component_spans(path);
    if kind == EntryKind::File {
        folders.pop();
    }
    let owned_index = folders.iter().enumerate().position(|(index, span)| {
        let folder_path = &path[..span.1];
        index > 0
            && extras_folder_kind(rules, &path[span.0..span.1]).is_some()
            && context.owns_feature_media(rules, &path[..folders[index - 1].1])
            && !context.holds_episode_seasons(rules, folder_path)
    })?;
    let (kind, evidence_span) = folders[owned_index..].iter().rev().find_map(|span| {
        extras_folder_kind(rules, &path[span.0..span.1]).map(|kind| (kind, *span))
    })?;
    Some(FolderExtra {
        kind,
        folder_path: &path[..folders[owned_index].1],
        evidence_span,
    })
}

struct NameExtra {
    kind: ExtraKind,
    rule: Rule,
    evidence_span: Span,
    title: Option<String>,
}

/// Classifies an extra by its own name: an exact name (E4) or an extras suffix (E3, E5).
/// In a TV library, a spaced suffix after an explicit episode token is the title of the
/// episode, not an extra (T19).
fn name_extra(
    rules: &Rules,
    path: &str,
    kind: EntryKind,
    library: Option<LibraryType>,
) -> Option<NameExtra> {
    if kind == EntryKind::Directory {
        return None;
    }
    let name = file_name(path);
    let name_start = path.len() - name.len();
    let stem = lexical::stem(name, kind);
    // The names are stored in the folded form (X25), so the stem is looked up in it.
    let folded_stem = folded_vocabulary_form(stem);
    let exact_names = if rules.is_video_file(path).result {
        &rules.vocab.extras_video_names
    } else if rules.is_audio_file(path).result {
        &rules.vocab.extras_audio_names
    } else {
        return None;
    };
    if let Some(kind) = exact_names.get(&folded_stem) {
        return Some(NameExtra {
            kind: *kind,
            rule: Rule::ExtrasName,
            evidence_span: Span(name_start, name_start + stem.len()),
            title: None,
        });
    }
    if !rules.is_video_file(path).result {
        return None;
    }
    let suffix = extras_suffix(rules, stem)?;
    if library == Some(LibraryType::Tv)
        && suffix.is_spaced(stem)
        && states_explicit_episode(rules, suffix.title)
    {
        return None;
    }
    Some(NameExtra {
        kind: suffix.kind,
        rule: Rule::ExtrasSuffix,
        evidence_span: Span(name_start + suffix.separator_index, name_start + stem.len()),
        title: Some(lexical::display_title(rules, suffix.title)),
    })
}

struct ExtrasSuffix<'a> {
    kind: ExtraKind,
    title: &'a str,
    separator_index: usize,
}

impl ExtrasSuffix<'_> {
    /// Returns `true` if the separator has a space beside it (`Show S01E01 - Trailer`), rather
    /// than directly touching the name (`Show S01E01-trailer`).
    fn is_spaced(&self, stem: &str) -> bool {
        stem[..self.separator_index].ends_with(char::is_whitespace)
            || stem[self.separator_index + 1..].starts_with(char::is_whitespace)
    }
}

/// Finds an extras suffix at the end of `stem`: `-`, `.`, `_` or ` - `, then an extras word
/// and an optional index, with a title before it.
/// Returns `None` if `stem` does not end in an extras suffix.
fn extras_suffix<'a>(rules: &Rules, stem: &'a str) -> Option<ExtrasSuffix<'a>> {
    let separator_index = stem.rfind(['-', '.', '_'])?;
    let title = stem[..separator_index].trim_end();
    let word = folded_vocabulary_form(&stem[separator_index + 1..]);
    let word = word.trim_end_matches(|character: char| character.is_ascii_digit());
    if title.is_empty() {
        return None;
    }
    let kind = rules.vocab.extras_suffixes.get(word)?;
    Some(ExtrasSuffix {
        kind: *kind,
        title,
        separator_index,
    })
}

/// Returns the kind of extra that an extras folder name stands for (E1).
/// Returns `None` if `folder_name` is not an extras folder name.
pub(crate) fn extras_folder_kind(rules: &Rules, folder_name: &str) -> Option<ExtraKind> {
    let folded = folded_vocabulary_form(folder_name);
    rules.vocab.extras_folders.get(&folded).copied()
}

/// Returns `text` in lowercase with spaces, dashes, dots and underscores removed:
/// `Theme-Music` → `thememusic`.
pub(crate) fn folded_vocabulary_form(text: &str) -> String {
    lexical::vocabulary_form(text)
        .chars()
        .filter(|character| !matches!(character, ' ' | '-' | '.' | '_'))
        .collect()
}

/// The files in scope, sorted so the files below a folder are one contiguous range.
pub(crate) struct ExtrasContext {
    library: Option<LibraryType>,
    paths: Vec<String>,
}

impl ExtrasContext {
    pub(crate) fn new<'a>(
        library: Option<LibraryType>,
        paths: impl IntoIterator<Item = &'a String>,
    ) -> Self {
        let mut paths: Vec<String> = paths.into_iter().cloned().collect();
        paths.sort();
        paths.dedup();
        ExtrasContext { library, paths }
    }

    /// Returns `true` if any file in the same folder as `path`, including `path`, has a space
    /// in its name. The separators of all extras in one folder are read the same way (M6).
    fn has_spaced_sibling(&self, path: &str) -> bool {
        let folder = parent_folder(path).map_or(String::new(), |folder| format!("{folder}/"));
        let start = self.paths.partition_point(|other| *other < folder);
        self.paths[start..]
            .iter()
            .take_while(|other| other.starts_with(&folder))
            .filter(|other| !other[folder.len()..].contains('/'))
            .any(|other| lexical::stem(file_name(other), EntryKind::File).contains(' '))
    }

    /// Returns the videos below `folder` whose names make them features rather than extras.
    fn feature_videos(&self, rules: &Rules, folder: &str) -> impl Iterator<Item = &str> {
        let prefix = format!("{folder}/");
        let start = self.paths.partition_point(|path| *path < prefix);
        self.paths[start..]
            .iter()
            .take_while(move |path| path.starts_with(&prefix))
            .map(String::as_str)
            .filter(|path| {
                rules.is_video_file(path).result
                    && name_extra(rules, path, EntryKind::File, self.library).is_none()
            })
    }

    /// Returns `true` if `folder` owns feature media. A folder owns feature media if a feature
    /// video is in it, in a season folder directly below it, or one folder deeper than that
    /// (T18). Extras folders on the path do not count. A wrapper folder with the same name
    /// (`Show/Show/`) owns what its child owns.
    fn owns_feature_media(&self, rules: &Rules, folder: &str) -> bool {
        let is_plain = |name: &str| extras_folder_kind(rules, name).is_none();
        self.feature_videos(rules, folder).any(|video| {
            match folders_between(folder, video).as_deref() {
                Some([]) => true,
                Some([child, ..]) if is_same_name(child, file_name(folder)) => {
                    self.owns_feature_media(rules, &join(folder, child))
                }
                Some([only]) => {
                    is_plain(only)
                        && (is_season_folder(rules, folder)
                            || is_season_folder(rules, &join(folder, only)))
                }
                Some([season, nested]) => {
                    is_plain(season)
                        && is_plain(nested)
                        && is_season_folder(rules, &join(folder, season))
                }
                _ => false,
            }
        })
    }

    /// Returns `true` if `folder` contains season folders of videos with episode codes. Such a
    /// vocabulary folder is a series of its own (T1), not an extras folder.
    fn holds_episode_seasons(&self, rules: &Rules, folder: &str) -> bool {
        self.feature_videos(rules, folder).any(|video| {
            let in_season_folder = matches!(
                folders_between(folder, video).as_deref(),
                Some([season]) if is_season_folder(rules, &join(folder, season))
            );
            in_season_folder
                && lexical::has_episode_marker(
                    rules,
                    lexical::stem(file_name(video), EntryKind::File),
                )
        })
    }
}

/// Returns the series folder for a folder that contains an episode or owns an extra (T1,
/// T18). Returns the wrapper if the series folder is inside a wrapper folder with the same
/// name (`Show/Show/`).
/// Returns `None` if no series folder is found.
fn series_folder(rules: &Rules, folder: &str) -> Option<String> {
    let series = series_folder_of(rules, folder)?;
    match parent_folder(&series) {
        Some(wrapper) if is_same_name(file_name(wrapper), file_name(&series)) => {
            Some(wrapper.to_owned())
        }
        _ => Some(series),
    }
}

fn is_same_name(first: &str, second: &str) -> bool {
    lexical::vocabulary_form(first) == lexical::vocabulary_form(second)
}

/// Returns the folders between `folder` and `file`.
/// Returns `None` if `file` is not below `folder`.
fn folders_between<'a>(folder: &str, file: &'a str) -> Option<Vec<&'a str>> {
    let relative = file.strip_prefix(folder)?.strip_prefix('/')?;
    let mut folders: Vec<&str> = relative.split('/').collect();
    folders.pop();
    Some(folders)
}

fn join(folder: &str, child: &str) -> String {
    format!("{folder}/{child}")
}

/// Returns the byte span of each `/`-separated component of `path`.
fn component_spans(path: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = 0;
    for component in path.split('/') {
        spans.push(Span(start, start + component.len()));
        start += component.len() + 1;
    }
    spans
}
