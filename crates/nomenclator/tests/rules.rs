//! Tests the rules value: results contain the identity of the rules that they were
//! produced under.

use nomenclator::model::EntryKind;
use nomenclator::{LibraryType, MatchFacts, OutputProfile, RULES_VERSION, Rules};

#[test]
fn rules_builtin_identity_has_rules_version() {
    assert_eq!(Rules::builtin().identity().version, RULES_VERSION);
    assert_eq!(Rules::default().identity(), Rules::builtin().identity());
}

#[test]
fn rules_results_contain_rules_identity() {
    let rules = Rules::default();
    let paths = vec!["Northern Passage (2021)/Northern Passage (2021).mkv".to_owned()];
    let interpretation = rules.interpret(LibraryType::Movies, &paths, &[]);
    let plan = rules.plan_renames(&interpretation, &paths, &OutputProfile::default());
    let validation = rules.validate(&interpretation, &OutputProfile::default());
    let ranking = rules.rank_candidates(&MatchFacts::default(), &[]);
    for identity in [
        &interpretation.rules,
        &plan.rules,
        &validation.rules,
        &ranking.rules,
    ] {
        assert_eq!(identity, rules.identity());
    }
}

#[test]
fn free_functions_use_builtin_rules() {
    let name = "Northern.Passage.2021.1080p.BluRay.x264.mkv";
    assert_eq!(
        nomenclator::parse_movie_name(name, EntryKind::File),
        Rules::builtin().parse_movie_name(name, EntryKind::File),
    );
}
