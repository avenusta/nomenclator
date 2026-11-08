//! Derives the search queries that a caller sends to a provider (7.1, P7), builds match
//! facts from interpreted items, and ranks many items in one call (7.3, P9).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::inputs::EpisodeOrder;
use crate::interpret::Item;
use crate::lexical;
use crate::model::resolved_values;
use crate::rules::{Rules, RulesIdentity};

use super::{CandidateRanking, EntityKind, MatchCandidate, MatchFacts, MatchOutcome, ranking};

/// The separators that end a main title: the text before the last of them is the main
/// title of `Main: Subtitle` or `Main - Subtitle`.
const SUBTITLE_SEPARATORS: &[&str] = &[": ", " - "];

/// The search queries for one set of facts, most specific first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchQueries {
    /// The queries in the order to send them.
    pub queries: Vec<SearchQuery>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// One query: a lookup by provider ID, or a search by title and an optional year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchQuery {
    /// The reason for the query.
    pub reason: QueryReason,
    /// The entity kind to search for, from the facts.
    pub kind: Option<EntityKind>,
    /// The namespace of a lookup by provider ID.
    pub provider: Option<String>,
    /// The ID of a lookup by provider ID.
    pub id: Option<String>,
    /// The title of a search.
    pub title: Option<String>,
    /// The year of a search. Unset for a search without a year.
    pub year: Option<u16>,
}

/// The reason for a search query, in the order of the queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryReason {
    /// A lookup by one of the facts' provider IDs.
    ProviderId,
    /// The title with the year.
    TitleWithYear,
    /// An alternate title with the year.
    AlternateTitleWithYear,
    /// The title without the year, for a provider whose year differs.
    Title,
    /// An alternate title without the year.
    AlternateTitle,
    /// The text before the title's last subtitle separator, with the year if any.
    MainTitle,
}

/// Returns the search queries for `facts` under the built-in rules (P7).
///
/// [`Rules::search_queries`] runs under another rules value.
#[must_use]
pub fn search_queries(facts: &MatchFacts) -> SearchQueries {
    Rules::builtin().search_queries(facts)
}

/// Returns the match facts of each movie, series and episode of `items`, in item order
/// (P7). Seasons and orphan extras have no match facts.
pub fn match_facts(items: &[Item]) -> Vec<MatchFacts> {
    items.iter().filter_map(item_match_facts).collect()
}

/// One item to rank: the caller's handle, its facts and its candidates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankQuery {
    /// The caller's identifier for the item. It must be unique in the batch (P9).
    pub handle: String,
    /// The item's facts.
    pub facts: MatchFacts,
    /// The item's candidates.
    #[serde(default)]
    pub candidates: Vec<MatchCandidate>,
}

/// The rankings of a batch, sorted by handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchRanking {
    /// One ranking per query, sorted by handle.
    pub results: Vec<HandledRanking>,
    /// The rules this result was produced under.
    pub rules: RulesIdentity,
}

/// The ranking of one query of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandledRanking {
    /// The query's handle.
    pub handle: String,
    /// The query's ranking.
    pub ranking: CandidateRanking,
}

impl Rules {
    /// Returns the search queries for `facts`, most specific first (P7): each provider ID,
    /// the title and then the alternate titles with the year, the same titles without the
    /// year, and the main title of a title with a subtitle. A query whose title has the
    /// comparison key and year of an earlier query is left out.
    pub fn search_queries(&self, facts: &MatchFacts) -> SearchQueries {
        let query = |reason, title: Option<String>, year| SearchQuery {
            reason,
            kind: facts.kind,
            provider: None,
            id: None,
            title,
            year,
        };
        let mut queries: Vec<SearchQuery> = facts
            .provider_ids
            .iter()
            .map(|(provider, id)| SearchQuery {
                provider: Some(provider.clone()),
                id: Some(id.clone()),
                ..query(QueryReason::ProviderId, None, None)
            })
            .collect();
        let year = facts
            .year
            .or_else(|| super::date_year(facts.date.as_deref()?));
        let title = facts
            .title
            .as_deref()
            .map(clean_title)
            .filter(|title| !title.is_empty());
        let alternates: Vec<String> = facts
            .alternate_titles
            .iter()
            .map(|title| clean_title(title))
            .filter(|title| !title.is_empty())
            .collect();
        let mut titled = Vec::new();
        if let Some(year) = year {
            titled.extend(
                title
                    .iter()
                    .map(|title| (QueryReason::TitleWithYear, title, Some(year))),
            );
            titled.extend(
                alternates
                    .iter()
                    .map(|title| (QueryReason::AlternateTitleWithYear, title, Some(year))),
            );
        }
        titled.extend(title.iter().map(|title| (QueryReason::Title, title, None)));
        titled.extend(
            alternates
                .iter()
                .map(|title| (QueryReason::AlternateTitle, title, None)),
        );
        let main_title = title.as_deref().and_then(main_title);
        titled.extend(
            main_title
                .iter()
                .map(|main| (QueryReason::MainTitle, main, year)),
        );
        let mut seen: BTreeSet<(String, Option<u16>)> = BTreeSet::new();
        for (reason, title, year) in titled {
            let key = lexical::title_comparison_key(self, title);
            if !key.is_empty() && seen.insert((key, year)) {
                queries.push(query(reason, Some(title.clone()), year));
            }
        }
        SearchQueries {
            queries,
            rules: self.identity().clone(),
        }
    }

    /// Ranks each query of a batch against its candidates (P9). The results are sorted by
    /// handle, so the order of the queries never affects them. Queries that share a handle
    /// are invalid input (`duplicate_query_handle`).
    pub fn rank_batch(&self, queries: &[RankQuery]) -> BatchRanking {
        let mut handle_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for query in queries {
            *handle_counts.entry(query.handle.as_str()).or_default() += 1;
        }
        let mut results: Vec<HandledRanking> = queries
            .iter()
            .map(|query| HandledRanking {
                handle: query.handle.clone(),
                ranking: if handle_counts[query.handle.as_str()] > 1 {
                    ranking(
                        self,
                        MatchOutcome::InvalidInput,
                        "duplicate_query_handle",
                        Vec::new(),
                    )
                } else {
                    self.rank_candidates(&query.facts, &query.candidates)
                },
            })
            .collect();
        results.sort_by(|left, right| left.handle.cmp(&right.handle));
        BatchRanking {
            results,
            rules: self.identity().clone(),
        }
    }
}

/// Returns `title` with its whitespace collapsed.
fn clean_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Returns the text before the last subtitle separator of `title`. Returns `None` if the
/// title has no separator, or if no word precedes it.
fn main_title(title: &str) -> Option<String> {
    let end = SUBTITLE_SEPARATORS
        .iter()
        .filter_map(|separator| title.rfind(separator))
        .max()?;
    let main = title[..end].trim();
    main.chars()
        .any(char::is_alphanumeric)
        .then(|| main.to_owned())
}

fn item_match_facts(item: &Item) -> Option<MatchFacts> {
    match item {
        Item::Movie(movie) => Some(MatchFacts {
            kind: Some(EntityKind::Movie),
            title: movie.title.clone(),
            alternate_titles: movie.alternate_titles.clone(),
            year: movie.year,
            provider_ids: resolved_values(&movie.provider_ids),
            ..MatchFacts::default()
        }),
        Item::Series(series) => Some(MatchFacts {
            kind: Some(EntityKind::Series),
            title: series.title.clone(),
            year: series.year,
            provider_ids: resolved_values(&series.provider_ids),
            ..MatchFacts::default()
        }),
        Item::Episode(episode) => Some(MatchFacts {
            kind: Some(EntityKind::Episode),
            title: episode.episode_title.clone(),
            date: episode.date.resolved_value().cloned(),
            provider_ids: resolved_values(&episode.provider_ids),
            season: episode.season.resolved_value().copied(),
            // An episode in the absolute order is numbered by its absolute numbers.
            episodes: match episode.order {
                Some(EpisodeOrder::Absolute) => episode.absolute.clone(),
                _ => episode.episodes.clone(),
            }
            .unwrap_or_default(),
            order: episode.order,
            ..MatchFacts::default()
        }),
        Item::Season(_) | Item::Extra(_) => None,
    }
}
