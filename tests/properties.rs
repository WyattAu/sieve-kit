//! Property tests for condition evaluation.

#![forbid(unsafe_code)]

use proptest::prelude::*;

use sieve_kit::eval::{
    RegexCache, evaluate_condition, evaluate_rule, field_value, glob_match, sort_rules_by_priority,
};
use sieve_kit::types::{Condition, ConditionField, FilterRule, LogicOp, MailEnvelope, Operator};

fn envelope(subject: String) -> MailEnvelope {
    MailEnvelope {
        from: "alice@example.com".to_string(),
        to: "bob@example.com".to_string(),
        cc: "carol@example.com".to_string(),
        subject,
        body: "this is a test body".to_string(),
        has_attachment: false,
        headers: vec![("X-Priority".to_string(), "high".to_string())],
    }
}

fn contains_condition(field: ConditionField, value: String, negate: bool) -> Condition {
    Condition {
        field,
        operator: Operator::Contains,
        value,
        negate,
    }
}

/// Safe, deterministic operators (no regex compilation involved).
fn arb_simple_condition() -> impl Strategy<Value = Condition> {
    (
        prop_oneof![
            Just(ConditionField::From),
            Just(ConditionField::To),
            Just(ConditionField::Cc),
            Just(ConditionField::Subject),
            Just(ConditionField::Body),
            Just(ConditionField::Header("X-Priority".to_string())),
            Just(ConditionField::Header("X-Missing".to_string())),
        ],
        prop_oneof![
            Just(Operator::Contains),
            Just(Operator::Equals),
            Just(Operator::Matches),
            Just(Operator::Exists),
        ],
        "[a-zA-Z]{0,12}",
        any::<bool>(),
    )
        .prop_map(|(field, operator, value, negate)| Condition {
            field,
            operator,
            value,
            negate,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `Contains` matches iff the lowercased field contains the lowercased value.
    #[test]
    fn contains_iff_case_insensitive_substring(
        needle in "[a-zA-Z]{0,10}",
        haystack in "[a-zA-Z0-9 ]{0,40}",
    ) {
        let env = envelope(haystack.clone());
        let cond = contains_condition(ConditionField::Subject, needle.clone(), false);
        let expected = haystack.to_lowercase().contains(&needle.to_lowercase());
        prop_assert_eq!(evaluate_condition(&cond, &env, &RegexCache::default()), expected);
    }

    /// `Equals` matches iff the values are equal ignoring ASCII case.
    #[test]
    fn equals_ignores_ascii_case(
        a in "[a-zA-Z0-9]{0,16}",
        b in "[a-zA-Z0-9]{0,16}",
    ) {
        let env = envelope(a.clone());
        let cond = Condition {
            field: ConditionField::Subject,
            operator: Operator::Equals,
            value: b.clone(),
            negate: false,
        };
        prop_assert_eq!(evaluate_condition(&cond, &env, &RegexCache::default()), a.eq_ignore_ascii_case(&b));
    }

    /// Negation inverts the result of the same condition.
    #[test]
    fn negation_inverts_result(cond in arb_simple_condition()) {
        let env = envelope("Hello World".to_string());
        let cache = RegexCache::default();
        let plain_cond = Condition { negate: false, ..cond };
        let plain = evaluate_condition(&plain_cond, &env, &cache);
        let negated = Condition { negate: true, ..plain_cond };
        prop_assert_eq!(evaluate_condition(&negated, &env, &cache), !plain);
    }

    /// AND/OR rule evaluation agrees with per-condition evaluation.
    #[test]
    fn rule_logic_matches_condition_combination(
        conds in prop::collection::vec(arb_simple_condition(), 1..8),
        logic in prop_oneof![Just(LogicOp::And), Just(LogicOp::Or)],
    ) {
        let env = envelope("Hello World".to_string());
        let cache = RegexCache::default();
        let results: Vec<bool> = conds.iter().map(|c| evaluate_condition(c, &env, &cache)).collect();
        let rule = FilterRule {
            id: "prop".to_string(),
            name: "prop".to_string(),
            enabled: true,
            priority: 0,
            conditions: conds,
            condition_logic: logic,
            actions: vec![],
        };
        let expected = match logic {
            LogicOp::And => results.iter().all(|&r| r),
            LogicOp::Or => results.iter().any(|&r| r),
        };
        prop_assert_eq!(evaluate_rule(&rule, &env, &cache), expected);
    }

    /// Disabled rules and rules without conditions never match.
    #[test]
    fn disabled_or_empty_rules_never_match(
        conds in prop::collection::vec(arb_simple_condition(), 0..5),
        logic in prop_oneof![Just(LogicOp::And), Just(LogicOp::Or)],
    ) {
        let env = envelope("Hello World".to_string());
        let cache = RegexCache::default();
        for enabled in [false, true] {
            let rule = FilterRule {
                id: "prop".to_string(),
                name: "prop".to_string(),
                enabled,
                priority: 0,
                conditions: conds.clone(),
                condition_logic: logic,
                actions: vec![],
            };
            if !enabled || rule.conditions.is_empty() {
                prop_assert!(!evaluate_rule(&rule, &env, &cache));
            }
        }
    }

    /// `*` matches any input.
    #[test]
    fn star_glob_matches_anything(input in ".*") {
        prop_assert!(glob_match("*", &input));
    }

    /// A glob without wildcards is a case-insensitive equality check.
    #[test]
    fn literal_glob_is_case_insensitive_equality(
        pattern in "[a-zA-Z0-9 ]{0,20}",
        input in "[a-zA-Z0-9 ]{0,20}",
    ) {
        prop_assert_eq!(glob_match(&pattern, &input), pattern.eq_ignore_ascii_case(&input));
    }

    /// `?` matches exactly one character.
    #[test]
    fn question_mark_matches_exactly_one_char(
        one in "[a-z]",
        none in "",
        two in "[a-z]{2}",
    ) {
        prop_assert!(glob_match("?", &one));
        prop_assert!(!glob_match("?", &none));
        prop_assert!(!glob_match("?", &two));
    }

    /// `Exists` is true iff the field value is non-empty.
    #[test]
    fn exists_iff_non_empty(subject in "[a-zA-Z0-9 ]{0,20}") {
        let env = envelope(subject.clone());
        let cond = Condition {
            field: ConditionField::Subject,
            operator: Operator::Exists,
            value: String::new(),
            negate: false,
        };
        prop_assert_eq!(evaluate_condition(&cond, &env, &RegexCache::default()), !subject.is_empty());
    }

    /// Sorting by priority yields a non-decreasing permutation.
    #[test]
    fn sort_by_priority_is_stable_ordering(priorities in prop::collection::vec(any::<i32>(), 0..50)) {
        let mut rules: Vec<FilterRule> = priorities
            .iter()
            .map(|&p| FilterRule {
                id: format!("r{p}"),
                name: "prop".to_string(),
                enabled: true,
                priority: p,
                conditions: vec![],
                condition_logic: LogicOp::And,
                actions: vec![],
            })
            .collect();
        let mut expected: Vec<i32> = priorities.clone();
        expected.sort_unstable();
        sort_rules_by_priority(&mut rules);
        let got: Vec<i32> = rules.iter().map(|r| r.priority).collect();
        prop_assert_eq!(got, expected);
    }
}

/// `field_value` resolves `HasAttachment` to the literal strings the
/// `Equals` operator compares against.
#[test]
fn has_attachment_field_resolves_to_true_false() {
    let mut env = envelope("s".to_string());
    env.has_attachment = true;
    assert_eq!(field_value(&env, &ConditionField::HasAttachment), "true");
    env.has_attachment = false;
    assert_eq!(field_value(&env, &ConditionField::HasAttachment), "false");
}

/// Missing headers resolve to the empty string.
#[test]
fn missing_header_resolves_to_empty() {
    let env = envelope("s".to_string());
    assert_eq!(
        field_value(&env, &ConditionField::Header("X-Missing".to_string())),
        ""
    );
}
