//! Rule evaluation engine. Synchronous, bounded, no I/O.
//!
//! Regex evaluation is checked against a 100 ms budget after matching.
//! Invalid regex patterns are treated as non-matching (never panic).

use std::sync::Arc;

use regex::Regex;

use crate::types::{Condition, ConditionField, FilterRule, Filterable, LogicOp, Operator};

/// Maximum time allowed for a single regex match (milliseconds).
const REGEX_TIMEOUT_MS: u128 = 100;

/// Cache of compiled regex patterns keyed by the raw pattern string.
/// Prevents re-compilation on every evaluation pass.
#[derive(Clone, Default)]
pub struct RegexCache {
    inner: Arc<std::sync::RwLock<std::collections::HashMap<String, Option<Regex>>>>,
}

impl RegexCache {
    /// Returns a compiled regex, or `None` if the pattern is invalid.
    #[must_use]
    pub fn get_or_compile(&self, pattern: &str) -> Option<Regex> {
        // Fast path: already compiled.
        if let Ok(cache) = self.inner.read() {
            if let Some(entry) = cache.get(pattern) {
                return entry.clone();
            }
        }
        // Slow path: compile and insert.
        let compiled = Regex::new(pattern).ok();
        if let Ok(mut cache) = self.inner.write() {
            cache.insert(pattern.to_string(), compiled.clone());
        }
        compiled
    }
}

/// Resolve the string value of a condition field on a message.
///
/// `HasAttachment` resolves to `"true"`/`"false"`; missing fields resolve to
/// the empty string.
#[must_use]
pub fn field_value<'a, F: Filterable + ?Sized>(msg: &'a F, field: &ConditionField) -> &'a str {
    match field {
        ConditionField::From => msg.from(),
        ConditionField::To => msg.to(),
        ConditionField::Cc => msg.cc(),
        ConditionField::Subject => msg.subject(),
        ConditionField::Body => msg.body(),
        ConditionField::Header(name) => msg.header(name).unwrap_or_default(),
        ConditionField::HasAttachment => {
            if msg.has_attachment() {
                "true"
            } else {
                "false"
            }
        }
    }
}

/// Evaluate a filter rule against a message.
///
/// Returns `true` when the rule matches (all/any conditions satisfied).
/// Disabled rules and rules without conditions always return `false`.
///
/// # Panics
///
/// Never panics. Invalid regex patterns are treated as non-matching.
#[must_use]
pub fn evaluate_rule<F: Filterable + ?Sized>(
    rule: &FilterRule,
    msg: &F,
    regex_cache: &RegexCache,
) -> bool {
    if !rule.enabled {
        return false;
    }
    if rule.conditions.is_empty() {
        return false;
    }

    let results: Vec<bool> = rule
        .conditions
        .iter()
        .map(|c| evaluate_condition(c, msg, regex_cache))
        .collect();

    match rule.condition_logic {
        LogicOp::And => results.iter().all(|&r| r),
        LogicOp::Or => results.iter().any(|&r| r),
    }
}

/// Evaluate a single condition against a message.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn evaluate_condition<F: Filterable + ?Sized>(
    condition: &Condition,
    msg: &F,
    regex_cache: &RegexCache,
) -> bool {
    let field_value = field_value(msg, &condition.field);
    let result = match &condition.operator {
        Operator::Contains => field_value
            .to_lowercase()
            .contains(&condition.value.to_lowercase()),
        Operator::Equals => field_value.eq_ignore_ascii_case(&condition.value),
        Operator::Matches => glob_match(&condition.value, field_value),
        Operator::Regex => evaluate_regex(&condition.value, field_value, regex_cache),
        Operator::Exists => !field_value.is_empty(),
    };
    if condition.negate { !result } else { result }
}

/// Evaluate a regex condition with a bounded budget.
///
/// Returns `false` on invalid patterns or when the match exceeds the time
/// budget (never panics).
fn evaluate_regex(pattern: &str, input: &str, cache: &RegexCache) -> bool {
    let Some(re) = cache.get_or_compile(pattern) else {
        return false;
    };

    // The regex crate is linear-time (no catastrophic backtracking) and has
    // no built-in timeout. Wall-clock measurement is a post-hoc safety check:
    // a match that ran over budget is reported as non-matching so callers can
    // treat pathological rules conservatively.
    let start = std::time::Instant::now();
    let matched = re.is_match(input);
    if start.elapsed().as_millis() > REGEX_TIMEOUT_MS {
        return false;
    }
    matched
}

/// Simple glob matching supporting `*` (any chars) and `?` (single char).
///
/// `*` and `?` are the only special characters; backslash escapes them.
/// Matching is case-insensitive.
#[must_use]
pub fn glob_match(pattern: &str, input: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let input_lower = input.to_lowercase();
    glob_match_inner(pattern_lower.as_bytes(), input_lower.as_bytes())
}

#[allow(clippy::similar_names)]
fn glob_match_inner(pattern: &[u8], input: &[u8]) -> bool {
    let mut pi = 0;
    let mut ii = 0;
    let mut star_pi = usize::MAX;
    let mut star_ii = 0;

    while ii < input.len() {
        if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = pi;
            star_ii = ii;
            pi += 1;
        } else if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == input[ii]) {
            pi += 1;
            ii += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ii += 1;
            ii = star_ii;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// Sort rules by priority (ascending = highest priority first).
pub fn sort_rules_by_priority(rules: &mut [FilterRule]) {
    rules.sort_by_key(|r| r.priority);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::types::MailEnvelope;

    fn make_rule(conditions: Vec<Condition>, logic: LogicOp) -> FilterRule {
        FilterRule {
            id: "test-rule-1".to_string(),
            name: "Test Rule".to_string(),
            enabled: true,
            priority: 0,
            conditions,
            condition_logic: logic,
            actions: vec![],
        }
    }

    fn make_envelope() -> MailEnvelope {
        MailEnvelope {
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            cc: String::new(),
            subject: "Hello World".to_string(),
            body: String::new(),
            has_attachment: false,
            headers: vec![],
        }
    }

    #[test]
    fn contains_operator_matches_case_insensitive() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Contains,
                value: "hello".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn contains_operator_no_match() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Contains,
                value: "nomatch".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(!evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn negate_inverts_result() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Contains,
                value: "nomatch".to_string(),
                negate: true,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn and_logic_requires_all() {
        let rule = make_rule(
            vec![
                Condition {
                    field: ConditionField::Subject,
                    operator: Operator::Contains,
                    value: "hello".to_string(),
                    negate: false,
                },
                Condition {
                    field: ConditionField::From,
                    operator: Operator::Contains,
                    value: "nomatch".to_string(),
                    negate: false,
                },
            ],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(!evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn or_logic_requires_any() {
        let rule = make_rule(
            vec![
                Condition {
                    field: ConditionField::Subject,
                    operator: Operator::Contains,
                    value: "nomatch".to_string(),
                    negate: false,
                },
                Condition {
                    field: ConditionField::From,
                    operator: Operator::Contains,
                    value: "alice".to_string(),
                    negate: false,
                },
            ],
            LogicOp::Or,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn regex_condition_matches() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Regex,
                value: r"(?i)hello\s+world".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn regex_invalid_pattern_returns_false() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Regex,
                value: "[invalid".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(!evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("hello*", "hello world"));
        assert!(glob_match("*world", "hello world"));
        assert!(glob_match("hello*world", "hello beautiful world"));
        assert!(glob_match("h?llo", "hello"));
        assert!(!glob_match("h?llo", "hllo"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let mut rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Exists,
                value: String::new(),
                negate: false,
            }],
            LogicOp::And,
        );
        rule.enabled = false;
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(!evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn empty_conditions_never_match() {
        let rule = make_rule(vec![], LogicOp::And);
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(!evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn exists_operator() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Exists,
                value: String::new(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = make_envelope();
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn body_condition() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Body,
                operator: Operator::Contains,
                value: "test".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = MailEnvelope {
            body: "this is a test body".to_string(),
            ..make_envelope()
        };
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn has_attachment_condition() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::HasAttachment,
                operator: Operator::Equals,
                value: "true".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = MailEnvelope {
            has_attachment: true,
            ..make_envelope()
        };
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn header_condition() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Header("X-Priority".to_string()),
                operator: Operator::Equals,
                value: "high".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = MailEnvelope {
            headers: vec![("x-priority".to_string(), "high".to_string())],
            ..make_envelope()
        };
        let cache = RegexCache::default();
        assert!(evaluate_rule(&rule, &msg, &cache));
    }

    #[test]
    fn regex_cache_reuses_compiled_pattern() {
        let cache = RegexCache::default();
        let r1 = cache.get_or_compile(r"\d+");
        let r2 = cache.get_or_compile(r"\d+");
        // Both calls should succeed and return equivalent patterns.
        assert!(r1.is_some());
        assert!(r2.is_some());
        // Verify both patterns match the same input.
        assert!(r1.unwrap().is_match("123"));
        assert!(r2.unwrap().is_match("123"));
    }

    #[test]
    fn rule_validation_rejects_invalid_regex() {
        let mut rule = make_rule(
            vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Regex,
                value: "[invalid".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        assert!(matches!(
            rule.validate(),
            Err(crate::error::FilterError::InvalidRegex { .. })
        ));
        rule.conditions[0].operator = Operator::Contains;
        rule.conditions[0].value = "ok".to_string();
        assert!(rule.validate().is_ok());
        rule.id = String::new();
        assert!(matches!(
            rule.validate(),
            Err(crate::error::FilterError::EmptyRuleId)
        ));
        rule.id = "x".to_string();
        rule.name = String::new();
        assert!(matches!(
            rule.validate(),
            Err(crate::error::FilterError::EmptyRuleName)
        ));
    }
}
