//! Checks the two-way property of templates (6.1, R14): reading a name that a template
//! rendered returns the facts that the template encodes, if no fact contains the template's
//! own separator text.

use std::collections::BTreeMap;

use nomenclator::model::Finding;
use nomenclator::{Template, TemplateFacts};

const TEMPLATES: &[&str] = &[
    "{title}[ ({year})]/{title}[ ({year})]",
    "{title}[ ({year})][ {{tmdb-{tmdb}}}]/{title}[ ({year})][ {{tmdb-{tmdb}}}][ - {version_label}][ - part{part}]",
    "{title}.{year}",
    "{series}[ ({series_year})]/Season {season:00}/{series} - S{season:00}E{episode:00}[ - {episode_title}]",
    "{series}/S{season}/{series} {season}x{episode:00}",
    "[{year}] {title}",
];

fn samples() -> Vec<TemplateFacts> {
    let titles = [
        "Northern Passage",
        "1917",
        "Mr. Holland's Opus",
        "Æon Flux",
        "A - B",
    ];
    let mut samples = Vec::new();
    for title in titles {
        for year in [None, Some("2021")] {
            for extra in [None, Some(("version_label", "2160p")), Some(("part", "2"))] {
                let mut facts = BTreeMap::new();
                facts.insert("title".to_owned(), title.to_owned());
                facts.insert("series".to_owned(), title.to_owned());
                facts.insert("season".to_owned(), "2".to_owned());
                facts.insert("episode".to_owned(), "5".to_owned());
                facts.insert("episode_title".to_owned(), "The Long Night".to_owned());
                facts.insert("tmdb".to_owned(), "329865".to_owned());
                if let Some(year) = year {
                    facts.insert("year".to_owned(), year.to_owned());
                    facts.insert("series_year".to_owned(), year.to_owned());
                }
                if let Some((field, value)) = extra {
                    facts.insert(field.to_owned(), value.to_owned());
                }
                samples.push(facts);
            }
        }
    }
    samples
}

/// The facts of `all` that `template` renders.
fn encoded(template: &Template, all: &TemplateFacts) -> TemplateFacts {
    all.iter()
        .filter(|(field, _)| {
            let without: TemplateFacts = all
                .iter()
                .filter(|(other, _)| other != field)
                .map(|(other, value)| (other.clone(), value.clone()))
                .collect();
            template.render(&without) != template.render(all)
        })
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect()
}

/// The literal text between the fields of a template, without the optional brackets.
fn separators(source: &str) -> Vec<String> {
    let mut separators = Vec::new();
    let mut current = String::new();
    let mut in_field = false;
    for character in source.chars() {
        match character {
            '{' if !in_field => {
                in_field = true;
                if !current.is_empty() {
                    separators.push(std::mem::take(&mut current));
                }
            }
            '}' if in_field => in_field = false,
            '[' | ']' if !in_field => {
                if !current.is_empty() {
                    separators.push(std::mem::take(&mut current));
                }
            }
            _ if !in_field => current.push(character),
            _ => {}
        }
    }
    if !current.is_empty() {
        separators.push(current);
    }
    separators
}

#[test]
fn template_read_returns_rendered_facts() {
    for source in TEMPLATES {
        let template = Template::parse(source).expect("the template compiles");
        for facts in samples() {
            let Some(name) = template.render(&facts).path else {
                continue;
            };
            let expected = encoded(&template, &facts);
            let collides = expected.values().any(|value| {
                separators(source).iter().any(|separator| {
                    value.contains(separator.trim()) && !separator.trim().is_empty()
                })
            });
            match template.read(&name) {
                Finding::Resolved { value, .. } if !collides => {
                    assert_eq!(value, expected, "`{source}` read `{name}`");
                }
                Finding::Ambiguous { candidates, .. } if collides => assert!(
                    candidates.contains(&expected),
                    "`{source}` read `{name}` as {candidates:?}, without {expected:?}"
                ),
                Finding::Resolved { .. } | Finding::Ambiguous { .. } if collides => {}
                other => panic!("`{source}` read `{name}` as {other:?}, expected {expected:?}"),
            }
        }
    }
}
