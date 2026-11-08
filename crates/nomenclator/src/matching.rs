//! Ranks caller-supplied provider candidates and reports the components of each score.
//!
//! The caller fetches the candidates from any source. This module does not use any
//! provider's API. Contradictions are recorded separately from similarity points (P2). A
//! score is a sum of named points, not a probability (P1).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::inputs::EpisodeOrder;
use crate::lexical;
use crate::rules::{Rules, RulesIdentity};

mod queries;

pub use queries::{
    BatchRanking, HandledRanking, QueryReason, RankQuery, SearchQueries, SearchQuery, match_facts,
    search_queries,
};

/// The points for a provider ID that the facts and the candidate both have with the same
/// value (P3).
const SAME_PROVIDER_ID_POINTS: u32 = 100;
/// The points for titles whose comparison keys are equal (P4).
const EXACT_TITLE_POINTS: u32 = 60;
/// The points for titles whose word sets are equal (P4). Partial agreement gives
/// proportionally fewer points.
const FULL_WORD_AGREEMENT_POINTS: u32 = 50;
const SAME_YEAR_POINTS: u32 = 20;
const YEAR_WITHIN_TOLERANCE_POINTS: u32 = 10;
/// The largest year difference that is not a contradiction (P2).
const YEAR_TOLERANCE: u16 = 1;
const SAME_EPISODE_POINTS: u32 = 60;
const SAME_EPISODE_SEASON_UNKNOWN_POINTS: u32 = 30;
const SAME_DATE_POINTS: u32 = 60;
/// The minimum score for selecting a candidate without a provider ID (P6).
const MINIMUM_SELECTION_SCORE: u32 = 60;
/// The largest gap between the best score and another score that makes the result
/// ambiguous (P6).
const TIE_WINDOW: u32 = 10;

/// The kind of entity that a candidate or the facts describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A movie.
    Movie,
    /// A whole series.
    Series,
    /// One episode of a series.
    Episode,
}

/// The facts that the caller established about the media, from names or another source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchFacts {
    /// The entity kind. An unknown kind never causes a contradiction (P2).
    pub kind: Option<EntityKind>,
    /// The primary title.
    pub title: Option<String>,
    /// Additional titles. Only the best agreement of any title counts (P4).
    #[serde(default)]
    pub alternate_titles: Vec<String>,
    /// The release year.
    pub year: Option<u16>,
    /// The air date, as `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Provider IDs by namespace.
    #[serde(default)]
    pub provider_ids: BTreeMap<String, String>,
    /// The season that the media belongs to.
    pub season: Option<u16>,
    /// Every episode that the media covers.
    #[serde(default)]
    pub episodes: Vec<u32>,
    /// The order that `season` and `episodes` are numbered in (P8). Unset for the aired
    /// order.
    #[serde(default)]
    pub order: Option<EpisodeOrder>,
}

/// An entity that the caller found at a provider, with an opaque handle that identifies
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchCandidate {
    /// The caller's identifier for the candidate. It must be unique among the candidates
    /// (P1).
    pub handle: String,
    /// The candidate's entity kind.
    pub kind: EntityKind,
    /// The primary title.
    pub title: String,
    /// Additional titles that the provider lists for the entity.
    #[serde(default)]
    pub alternate_titles: Vec<String>,
    /// The release year.
    pub year: Option<u16>,
    /// The air date, as `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Provider IDs by namespace.
    #[serde(default)]
    pub provider_ids: BTreeMap<String, String>,
    /// The episode's season.
    pub season: Option<u16>,
    /// The episode's number.
    pub episode: Option<u32>,
    /// The title in the entity's original language (P8).
    #[serde(default)]
    pub original_title: Option<String>,
    /// Titles in other languages (P8).
    #[serde(default)]
    pub localized_titles: Vec<LocalizedTitle>,
    /// Release dates, such as one per country (P8). Each date's year counts as a year of
    /// the candidate.
    #[serde(default)]
    pub release_dates: Vec<ReleaseDate>,
    /// The episode's coordinates in other orders (P8). `season` and `episode` are the
    /// aired coordinates.
    #[serde(default)]
    pub coordinates: Vec<EpisodeCoordinates>,
}

/// A title in one language.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedTitle {
    /// The title.
    pub title: String,
    /// The language, as a BCP 47 tag.
    pub language: Option<String>,
}

/// A release date, such as one country's.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseDate {
    /// The date, as `YYYY-MM-DD`.
    pub date: String,
    /// The country, as an ISO 3166-1 code.
    pub country: Option<String>,
}

/// An episode's coordinates in one order.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeCoordinates {
    /// The order.
    pub order: EpisodeOrder,
    /// The season. Unset in the absolute order.
    pub season: Option<u16>,
    /// The episode's number.
    pub episode: u32,
}

/// The outcome of a ranking (P6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    /// One candidate is selected.
    Selected,
    /// Several candidates are equally plausible.
    Ambiguous,
    /// No candidate has enough evidence without a contradiction.
    NoMatch,
    /// The candidates cannot be ranked, for example because two candidates have the same
    /// handle (P1).
    InvalidInput,
}

/// The fact that a score component or a contradiction compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchComponent {
    /// A provider ID in a shared namespace (P3).
    ProviderId,
    /// The entity kind (P2).
    Kind,
    /// The titles that agree best (P4).
    Title,
    /// The release year (P5).
    Year,
    /// Season and episode coordinates (P5).
    Episode,
    /// An episode's air date (P5).
    Date,
}

/// The points that one comparison gave a candidate, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoreComponent {
    /// The compared fact.
    pub component: MatchComponent,
    /// The points given. The value can be zero.
    pub points: u32,
    /// The reason, such as `exact_title` or `similar_title`.
    pub reason: &'static str,
    /// The compared namespace, for a provider ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The candidate's title that agreed best, for a title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_title: Option<String>,
    /// The language of the localized title that agreed best, for a title (P8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_language: Option<String>,
    /// The release date whose year agreed best, for a year (P8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_date: Option<String>,
    /// The country of `matched_date`, for a year (P8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_country: Option<String>,
    /// The order of the compared coordinates, for an episode, if the facts state one (P8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<EpisodeOrder>,
}

/// A fact that the candidate contradicts. A candidate with a contradiction is never
/// selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contradiction {
    /// The contradicted fact.
    pub component: MatchComponent,
    /// The reason, such as `different_id` or `year_beyond_tolerance` (P2).
    pub reason: &'static str,
    /// The compared namespace, for a provider ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// A candidate's score and the evidence for it (P1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoredCandidate {
    /// The candidate's handle.
    pub handle: String,
    /// The sum of the components' points.
    pub score: u32,
    /// Each comparison that gave points.
    pub components: Vec<ScoreComponent>,
    /// Each fact that the candidate contradicts.
    pub contradictions: Vec<Contradiction>,
}

/// The result of ranking candidates: the outcome, its reason and the score of every
/// candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateRanking {
    /// The outcome of the ranking.
    pub outcome: MatchOutcome,
    /// The selected candidate's handle.
    pub selected: Option<String>,
    /// The reason for the outcome: `provider_id`, `best_score`, `tied_scores`,
    /// `several_provider_id_matches`,
    /// `no_candidates`, `all_contradicted`, `provider_id_match_contradicted`,
    /// `insufficient_evidence` or `duplicate_handle`.
    pub reason: &'static str,
    /// The candidates that an ambiguous result could not choose between.
    pub tied: Vec<String>,
    /// Every candidate. Candidates without contradictions come first, then the order is by
    /// score (highest first), then by handle.
    pub candidates: Vec<ScoredCandidate>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// Ranks caller-supplied candidates against the facts (P1-P6).
///
/// Uses the built-in rules. [`Rules::rank_candidates`] runs under another rules value.
#[must_use]
pub fn rank_candidates(facts: &MatchFacts, candidates: &[MatchCandidate]) -> CandidateRanking {
    Rules::builtin().rank_candidates(facts, candidates)
}

impl Rules {
    /// Ranks caller-supplied candidates against the facts (P1-P6).
    pub fn rank_candidates(
        &self,
        facts: &MatchFacts,
        candidates: &[MatchCandidate],
    ) -> CandidateRanking {
        let rules = self;
        let handles: BTreeSet<&str> = candidates
            .iter()
            .map(|candidate| candidate.handle.as_str())
            .collect();
        if handles.len() != candidates.len() {
            return ranking(
                rules,
                MatchOutcome::InvalidInput,
                "duplicate_handle",
                Vec::new(),
            );
        }
        if candidates.is_empty() {
            return ranking(rules, MatchOutcome::NoMatch, "no_candidates", Vec::new());
        }
        let mut scored: Vec<(ScoredCandidate, bool)> = candidates
            .iter()
            .map(|candidate| score_candidate(rules, facts, candidate))
            .collect();
        scored.sort_by(|(left, _), (right, _)| {
            let order_key = |candidate: &ScoredCandidate| {
                (
                    !candidate.contradictions.is_empty(),
                    std::cmp::Reverse(candidate.score),
                    candidate.handle.clone(),
                )
            };
            order_key(left).cmp(&order_key(right))
        });
        let any_candidate_has_same_id = scored.iter().any(|(_, has_same_id)| *has_same_id);
        let eligible_handles = |require_same_id: bool| -> Vec<(String, u32)> {
            scored
                .iter()
                .filter(|(candidate, has_same_id)| {
                    candidate.contradictions.is_empty() && (*has_same_id || !require_same_id)
                })
                .map(|(candidate, _)| (candidate.handle.clone(), candidate.score))
                .collect()
        };

        let (outcome, reason, selected, tied) = if any_candidate_has_same_id {
            // P3: an explicit ID outranks every title, so only candidates with that ID can
            // be selected.
            match eligible_handles(true).as_slice() {
                [] => (
                    MatchOutcome::NoMatch,
                    "provider_id_match_contradicted",
                    None,
                    Vec::new(),
                ),
                [(only, _)] => (
                    MatchOutcome::Selected,
                    "provider_id",
                    Some(only.clone()),
                    Vec::new(),
                ),
                several => (
                    MatchOutcome::Ambiguous,
                    "several_provider_id_matches",
                    None,
                    several.iter().map(|(handle, _)| handle.clone()).collect(),
                ),
            }
        } else {
            let eligible = eligible_handles(false);
            match eligible.first() {
                None => (MatchOutcome::NoMatch, "all_contradicted", None, Vec::new()),
                Some((_, best_score)) if *best_score < MINIMUM_SELECTION_SCORE => (
                    MatchOutcome::NoMatch,
                    "insufficient_evidence",
                    None,
                    Vec::new(),
                ),
                Some((best_handle, best_score)) => {
                    let tied: Vec<String> = eligible
                        .iter()
                        .filter(|(_, score)| best_score - score <= TIE_WINDOW)
                        .map(|(handle, _)| handle.clone())
                        .collect();
                    if tied.len() > 1 {
                        (MatchOutcome::Ambiguous, "tied_scores", None, tied)
                    } else {
                        (
                            MatchOutcome::Selected,
                            "best_score",
                            Some(best_handle.clone()),
                            Vec::new(),
                        )
                    }
                }
            }
        };
        CandidateRanking {
            selected,
            tied,
            ..ranking(
                rules,
                outcome,
                reason,
                scored.into_iter().map(|(candidate, _)| candidate).collect(),
            )
        }
    }
}

fn ranking(
    rules: &Rules,
    outcome: MatchOutcome,
    reason: &'static str,
    candidates: Vec<ScoredCandidate>,
) -> CandidateRanking {
    CandidateRanking {
        outcome,
        selected: None,
        reason,
        tied: Vec::new(),
        candidates,
        rules: rules.identity().clone(),
    }
}

/// Returns the candidate's score components, and `true` if the candidate has one of the
/// facts' provider IDs.
fn score_candidate(
    rules: &Rules,
    facts: &MatchFacts,
    candidate: &MatchCandidate,
) -> (ScoredCandidate, bool) {
    let mut components = Vec::new();
    let mut contradictions = Vec::new();
    let mut has_same_id = false;

    for (provider, fact_id) in &facts.provider_ids {
        let Some(candidate_id) = candidate.provider_ids.get(provider) else {
            continue;
        };
        let is_same_id = candidate_id == fact_id;
        has_same_id |= is_same_id;
        let (points, reason) = if is_same_id {
            (SAME_PROVIDER_ID_POINTS, "same_id")
        } else {
            contradictions.push(Contradiction {
                provider: Some(provider.clone()),
                ..contradiction(MatchComponent::ProviderId, "different_id")
            });
            (0, "different_id")
        };
        components.push(ScoreComponent {
            provider: Some(provider.clone()),
            ..component(MatchComponent::ProviderId, points, reason)
        });
    }

    if facts.kind.is_some_and(|kind| kind != candidate.kind) {
        contradictions.push(contradiction(MatchComponent::Kind, "different_kind"));
    }

    components.push(title_component(rules, facts, candidate));

    components.push(year_component(facts, candidate, &mut contradictions));

    if let Some((candidate_season, candidate_episode)) = candidate_coordinates(facts, candidate)
        && !facts.episodes.is_empty()
    {
        let seasons_differ = matches!(
            (facts.season, candidate_season),
            (Some(fact_season), Some(candidate_season)) if fact_season != candidate_season
        );
        // An absolute number is a whole coordinate, so it needs no season.
        let seasons_known = (facts.season.is_some() && candidate_season.is_some())
            || facts.order == Some(EpisodeOrder::Absolute);
        let episode_component = if seasons_differ || !facts.episodes.contains(&candidate_episode) {
            contradictions.push(contradiction(MatchComponent::Episode, "different_episode"));
            component(MatchComponent::Episode, 0, "different_episode")
        } else if seasons_known {
            component(MatchComponent::Episode, SAME_EPISODE_POINTS, "same_episode")
        } else {
            component(
                MatchComponent::Episode,
                SAME_EPISODE_SEASON_UNKNOWN_POINTS,
                "same_episode_season_unknown",
            )
        };
        components.push(ScoreComponent {
            order: facts.order,
            ..episode_component
        });
    }

    if let (EntityKind::Episode, Some(fact_date), Some(candidate_date)) =
        (candidate.kind, &facts.date, &candidate.date)
    {
        components.push(if fact_date == candidate_date {
            component(MatchComponent::Date, SAME_DATE_POINTS, "same_date")
        } else {
            contradictions.push(contradiction(MatchComponent::Date, "different_date"));
            component(MatchComponent::Date, 0, "different_date")
        });
    }

    let scored = ScoredCandidate {
        handle: candidate.handle.clone(),
        score: components.iter().map(|component| component.points).sum(),
        components,
        contradictions,
    };
    (scored, has_same_id)
}

/// Returns the best agreement between any title of the facts and any title of the
/// candidate. On equal points, the candidate's primary title wins, then its original
/// title, its alternate titles and its localized titles, in order (P4, P8).
fn title_component(
    rules: &Rules,
    facts: &MatchFacts,
    candidate: &MatchCandidate,
) -> ScoreComponent {
    let fact_titles: Vec<&String> = facts.title.iter().chain(&facts.alternate_titles).collect();
    if fact_titles.is_empty() {
        return component(MatchComponent::Title, 0, "title_unknown");
    }
    let candidate_titles = std::iter::once((&candidate.title, None))
        .chain(candidate.original_title.iter().map(|title| (title, None)))
        .chain(candidate.alternate_titles.iter().map(|title| (title, None)))
        .chain(
            candidate
                .localized_titles
                .iter()
                .map(|localized| (&localized.title, localized.language.as_ref())),
        );
    let mut best: Option<(u32, &'static str, &String, Option<&String>)> = None;
    for (candidate_title, language) in candidate_titles {
        for fact_title in &fact_titles {
            let (points, reason) = title_agreement(rules, fact_title, candidate_title);
            if points > 0 && best.is_none_or(|(best_points, ..)| points > best_points) {
                best = Some((points, reason, candidate_title, language));
            }
        }
    }
    match best {
        Some((points, reason, matched_title, language)) => ScoreComponent {
            matched_title: Some(matched_title.clone()),
            matched_language: language.cloned(),
            ..component(MatchComponent::Title, points, reason)
        },
        None => component(MatchComponent::Title, 0, "dissimilar_title"),
    }
}

/// Returns the agreement of two titles. Returns exact points if the title keys are equal.
/// Otherwise returns points from the Dice coefficient of the two word sets (twice the
/// shared words divided by all words). Returns no points if the coefficient is below 0.5.
fn title_agreement(rules: &Rules, fact_title: &str, candidate_title: &str) -> (u32, &'static str) {
    let fact_key = lexical::title_comparison_key(rules, fact_title);
    if !fact_key.is_empty() && fact_key == lexical::title_comparison_key(rules, candidate_title) {
        return (EXACT_TITLE_POINTS, "exact_title");
    }
    let fact_words: BTreeSet<String> = lexical::title_comparison_words(rules, fact_title)
        .into_iter()
        .collect();
    let candidate_words: BTreeSet<String> = lexical::title_comparison_words(rules, candidate_title)
        .into_iter()
        .collect();
    let shared_words = fact_words.intersection(&candidate_words).count() as u32;
    let all_words = (fact_words.len() + candidate_words.len()) as u32;
    if all_words == 0 || 4 * shared_words < all_words {
        return (0, "dissimilar_title");
    }
    // Computes round(FULL_WORD_AGREEMENT_POINTS * 2 * shared / all) in integer arithmetic.
    let points = (4 * FULL_WORD_AGREEMENT_POINTS * shared_words + all_words) / (2 * all_words);
    (points, "similar_title")
}

fn fact_year(facts: &MatchFacts) -> Option<u16> {
    facts.year.or_else(|| date_year(facts.date.as_deref()?))
}

/// Returns the year component (P5, P8). Each year of the candidate counts: its year, else
/// its date's year, and each release date's year. The year closest to the facts' year
/// agrees; on equal distance, the earlier one in that list. The candidate contradicts the
/// facts only if every year is beyond the tolerance.
fn year_component(
    facts: &MatchFacts,
    candidate: &MatchCandidate,
    contradictions: &mut Vec<Contradiction>,
) -> ScoreComponent {
    let own_year = candidate
        .year
        .or_else(|| date_year(candidate.date.as_deref()?))
        .map(|year| (year, None));
    let release_years = candidate
        .release_dates
        .iter()
        .filter_map(|release| Some((date_year(&release.date)?, Some(release))));
    let years: Vec<(u16, Option<&ReleaseDate>)> =
        own_year.into_iter().chain(release_years).collect();
    let Some(fact_year) = fact_year(facts) else {
        return component(MatchComponent::Year, 0, "year_unknown");
    };
    let Some(&(year, release)) = years
        .iter()
        .min_by_key(|(year, _)| fact_year.abs_diff(*year))
    else {
        return component(MatchComponent::Year, 0, "year_unknown");
    };
    let year_component = match fact_year.abs_diff(year) {
        0 => component(MatchComponent::Year, SAME_YEAR_POINTS, "same_year"),
        difference if difference <= YEAR_TOLERANCE => component(
            MatchComponent::Year,
            YEAR_WITHIN_TOLERANCE_POINTS,
            "year_within_tolerance",
        ),
        _ => {
            contradictions.push(contradiction(MatchComponent::Year, "year_beyond_tolerance"));
            component(MatchComponent::Year, 0, "year_beyond_tolerance")
        }
    };
    ScoreComponent {
        matched_date: release.map(|release| release.date.clone()),
        matched_country: release.and_then(|release| release.country.clone()),
        ..year_component
    }
}

/// Returns the candidate's season and episode in the facts' order (P8): the aired
/// coordinates if the facts state no order or the aired order, else the coordinates that
/// the candidate lists for that order. Returns `None` if the candidate has none.
fn candidate_coordinates(
    facts: &MatchFacts,
    candidate: &MatchCandidate,
) -> Option<(Option<u16>, u32)> {
    let order = facts.order.unwrap_or(EpisodeOrder::Aired);
    if order == EpisodeOrder::Aired
        && let Some(episode) = candidate.episode
    {
        return Some((candidate.season, episode));
    }
    candidate
        .coordinates
        .iter()
        .find(|coordinates| coordinates.order == order)
        .map(|coordinates| (coordinates.season, coordinates.episode))
}

fn date_year(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

fn component(kind: MatchComponent, points: u32, reason: &'static str) -> ScoreComponent {
    ScoreComponent {
        component: kind,
        points,
        reason,
        provider: None,
        matched_title: None,
        matched_language: None,
        matched_date: None,
        matched_country: None,
        order: None,
    }
}

fn contradiction(kind: MatchComponent, reason: &'static str) -> Contradiction {
    Contradiction {
        component: kind,
        reason,
        provider: None,
    }
}
