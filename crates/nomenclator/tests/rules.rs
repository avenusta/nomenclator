//! Tests the rules value: results contain the identity of the rules that they were
//! produced under.

use nomenclator::{LibraryType, MatchFacts, OutputProfile, RULES_VERSION, Rules};

#[test]
fn rules_builtin_identity_has_rules_version() {
    assert_eq!(Rules::builtin().identity().version, RULES_VERSION);
}

#[test]
fn rules_results_contain_rules_identity() {
    let rules = Rules::builtin();
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
