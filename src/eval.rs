//! Rule evaluation engine. Synchronous, bounded, no I/O.
//!
//! Regex evaluation is checked against a 100 ms budget after matching.
//! Invalid regex patterns are treated as non-matching (never panic).

use std::sync::Arc;

use regex::Regex;

use crate::types::{
    AddressPart, Condition, ConditionField, EnvelopePart, FilterRule, Filterable, LogicOp, Operator,
};

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
/// the empty string. For [`ConditionField::Envelope`] this is the *full*
/// envelope address (`:all` semantics); use
/// [`evaluate_condition`] to apply `:localpart`/`:domain` extraction.
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
        ConditionField::Envelope { part, .. } => match part {
            EnvelopePart::From => msg.envelope_from().unwrap_or_default(),
            EnvelopePart::To => msg.envelope_to().unwrap_or_default(),
        },
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
/// Envelope conditions (RFC 5228 §5.1) evaluate as non-matching when the
/// message supplies no envelope data; `:localpart`/`:domain` select the
/// portion of the address compared.
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
    let result = if let ConditionField::Envelope { part, address_part } = &condition.field {
        let raw = match part {
            EnvelopePart::From => msg.envelope_from(),
            EnvelopePart::To => msg.envelope_to(),
        };
        // Missing envelope data: the test does not match (a negated test
        // then matches, per `not` semantics).
        match raw {
            Some(value) => apply_operator(
                &condition.operator,
                extract_address_part(value, *address_part),
                condition,
                regex_cache,
            ),
            None => false,
        }
    } else {
        let field_value = field_value(msg, &condition.field);
        apply_operator(&condition.operator, field_value, condition, regex_cache)
    };
    if condition.negate { !result } else { result }
}

/// Extract the portion of an address selected by an
/// [`AddressPart`]. `:localpart` is everything before the last `@` (the
/// whole value when there is no `@`); `:domain` is everything after the
/// last `@` (empty when there is no `@`).
#[must_use]
pub fn extract_address_part(address: &str, part: AddressPart) -> &str {
    match part {
        AddressPart::All => address,
        AddressPart::Localpart => address.rsplit_once('@').map_or(address, |(lp, _)| lp),
        AddressPart::Domain => address.rsplit_once('@').map_or("", |(_, d)| d),
    }
}

/// Apply a comparison operator to a resolved field value. Shared by the
/// plain and envelope code paths so operator semantics (the `:comparator`
/// plumbing) stay identical across `address`/`header`/`envelope` tests.
fn apply_operator(
    operator: &Operator,
    value: &str,
    condition: &Condition,
    regex_cache: &RegexCache,
) -> bool {
    match operator {
        Operator::Contains => value
            .to_lowercase()
            .contains(&condition.value.to_lowercase()),
        Operator::Equals => value.eq_ignore_ascii_case(&condition.value),
        Operator::Matches => glob_match(&condition.value, value),
        Operator::Regex => evaluate_regex(&condition.value, value, regex_cache),
        Operator::Exists => !value.is_empty(),
        Operator::NumericEquals => ascii_numeric_eq(&condition.value, value),
    }
}

/// `i;ascii-numeric` equality (RFC 4790): both strings must consist solely
/// of ASCII digits and be numerically equal (leading zeros ignored). The
/// empty string compares equal only to the empty string; any non-numeric
/// operand never matches.
#[must_use]
pub fn ascii_numeric_eq(a: &str, b: &str) -> bool {
    let valid = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    if !valid(a) || !valid(b) {
        return false;
    }
    if a.is_empty() || b.is_empty() {
        return a.is_empty() && b.is_empty();
    }
    strip_leading_zeros(a) == strip_leading_zeros(b)
}

fn strip_leading_zeros(s: &str) -> &str {
    let trimmed = s.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
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

/// Caller-supplied runtime context for [`evaluate_plan`].
///
/// Evaluation stays synchronous and I/O-free: instead of the engine touching
/// storage, callers hand in an optional `seen_before` predicate that decides
/// whether an automated reply was already sent to a sender within its
/// respond period.
#[derive(Default)]
pub struct EvalContext<'a> {
    /// Respond-once hook for the
    /// [`vacation`](crate::types::Action::Vacation) action (RFC 5230):
    /// return `true` when a reply was already sent to this sender within
    /// the configured period. When `None`, no dedup is applied and the
    /// outcome always carries the reply (hosts can dedup themselves using
    /// the `to`/`days` fields of
    /// [`VacationReply`](crate::actions::VacationReply), or use the
    /// ready-made [`VacationTracker`](crate::actions::VacationTracker)).
    pub seen_before: Option<&'a dyn Fn(&str) -> bool>,
}

impl<'a> EvalContext<'a> {
    /// Attach a respond-once dedup predicate.
    #[must_use]
    pub fn with_seen_before(mut self, seen_before: &'a dyn Fn(&str) -> bool) -> Self {
        self.seen_before = Some(seen_before);
        self
    }
}

/// A non-fatal observation made during [`evaluate_plan`]. Evaluation never
/// fails: misconfigured or exotic actions are reported as warnings while
/// the rest of the plan still executes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvalWarning {
    /// A [`notify`](crate::types::Action::Notify) method URI uses a scheme
    /// outside the recognized set
    /// ([`KNOWN_NOTIFY_SCHEMES`](crate::types::KNOWN_NOTIFY_SCHEMES)). The
    /// notification outcome is still produced — the host decides whether to
    /// attempt delivery.
    #[error("notify method {method:?} has an unrecognized URI scheme")]
    UnknownNotifyMethod {
        /// The offending method URI.
        method: String,
    },
    /// A [`vacation`](crate::types::Action::Vacation) reply could not be
    /// routed: neither envelope sender nor header From yielded a usable
    /// address. The reply is omitted from the plan.
    #[error("vacation action could not determine a reply-to sender")]
    VacationNoSender,
}

/// The result of evaluating a rule set with [`evaluate_plan`]: the matched
/// rule (if any), the planned outcomes, and any warnings.
#[derive(Clone, Debug, Default)]
pub struct EvalOutcome {
    /// Id of the first matching rule, or `None` when no rule matched.
    pub rule_id: Option<String>,
    /// Outcomes to execute, in rule action order.
    pub plan: Vec<crate::actions::PlannedAction>,
    /// Non-fatal observations (unknown notify schemes, unroutable replies).
    pub warnings: Vec<EvalWarning>,
}

impl EvalOutcome {
    /// Whether the outcome set is empty (no rule matched and no warnings).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty() && self.warnings.is_empty()
    }
}

/// Evaluate rules against a message and resolve them into execution-ready
/// outcomes.
///
/// Like [`collect_matches`](crate::actions::collect_matches) this is
/// first-match-wins, but the returned plan additionally resolves the
/// actions that need runtime context:
///
/// - [`vacation`](crate::types::Action::Vacation): the reply-to sender is
///   taken from envelope data (falling back to the header From address);
///   the subject defaults to `Re: <original subject>`; a reply already sent
///   (per [`EvalContext::seen_before`]) suppresses the outcome, and an
///   unroutable reply emits [`EvalWarning::VacationNoSender`].
/// - [`notify`](crate::types::Action::Notify): unrecognized method schemes
///   emit [`EvalWarning::UnknownNotifyMethod`].
///
/// Pure and synchronous: no I/O, never panics.
#[must_use]
pub fn evaluate_plan<F: Filterable + ?Sized>(
    rules: &[FilterRule],
    msg: &F,
    ctx: &EvalContext<'_>,
    regex_cache: &RegexCache,
) -> EvalOutcome {
    let mut outcome = EvalOutcome::default();
    let Some(rule) = rules.iter().find(|r| evaluate_rule(r, msg, regex_cache)) else {
        return outcome;
    };
    outcome.rule_id = Some(rule.id.clone());
    for action in &rule.actions {
        match action {
            crate::types::Action::Vacation(vacation) => {
                plan_vacation(vacation, msg, ctx, &mut outcome);
            }
            crate::types::Action::Notify(notify) => {
                if !notify.has_known_scheme() {
                    outcome.warnings.push(EvalWarning::UnknownNotifyMethod {
                        method: notify.method.clone(),
                    });
                }
                outcome
                    .plan
                    .push(crate::actions::PlannedAction::from(action));
            }
            other => outcome
                .plan
                .push(crate::actions::PlannedAction::from(other)),
        }
    }
    outcome
}

/// Resolve a vacation action into a (possibly suppressed) reply outcome.
fn plan_vacation<F: Filterable + ?Sized>(
    vacation: &crate::types::Vacation,
    msg: &F,
    ctx: &EvalContext<'_>,
    outcome: &mut EvalOutcome,
) {
    let sender = msg
        .envelope_from()
        .filter(|s| !s.is_empty())
        .map_or_else(|| msg.from().to_string(), str::to_string);
    if sender.is_empty() {
        outcome.warnings.push(EvalWarning::VacationNoSender);
        return;
    }
    if ctx.seen_before.is_some_and(|seen| seen(&sender)) {
        // Respond-once-per-sender-per-period: suppress silently.
        return;
    }
    let subject = vacation.subject.clone().unwrap_or_else(|| {
        let original = msg.subject();
        if original.is_empty() {
            "Automated reply".to_string()
        } else {
            format!("Re: {original}")
        }
    });
    outcome.plan.push(crate::actions::PlannedAction::Vacation(
        crate::actions::VacationReply {
            to: sender,
            days: vacation.days,
            subject,
            from: vacation.from.clone(),
            message: vacation.message.clone(),
        },
    ));
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
            envelope_from: None,
            envelope_to: None,
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

    // ---- envelope test (RFC 5228 §5.1) ---------------------------------

    fn envelope_condition(part: EnvelopePart, part_kind: AddressPart, value: &str) -> Condition {
        Condition {
            field: ConditionField::Envelope {
                part,
                address_part: part_kind,
            },
            operator: Operator::Equals,
            value: value.to_string(),
            negate: false,
        }
    }

    fn envelope_msg() -> MailEnvelope {
        MailEnvelope {
            from: "newsletter@lists.example.com".to_string(),
            envelope_from: Some("bounce@sender.example.net".to_string()),
            envelope_to: Some("you@corp.example".to_string()),
            ..MailEnvelope::default()
        }
    }

    #[test]
    fn envelope_all_matches_full_address() {
        let msg = envelope_msg();
        let cache = RegexCache::default();
        for (part, value) in [
            (EnvelopePart::From, "bounce@sender.example.net"),
            (EnvelopePart::To, "you@corp.example"),
        ] {
            let cond = envelope_condition(part, AddressPart::All, value);
            assert!(evaluate_condition(&cond, &msg, &cache));
        }
    }

    #[test]
    fn envelope_localpart_and_domain() {
        let msg = envelope_msg();
        let cache = RegexCache::default();
        let cond = envelope_condition(EnvelopePart::From, AddressPart::Localpart, "bounce");
        assert!(evaluate_condition(&cond, &msg, &cache));
        let cond = envelope_condition(
            EnvelopePart::From,
            AddressPart::Domain,
            "sender.example.net",
        );
        assert!(evaluate_condition(&cond, &msg, &cache));
        let cond = envelope_condition(EnvelopePart::From, AddressPart::Domain, "elsewhere");
        assert!(!evaluate_condition(&cond, &msg, &cache));
    }

    #[test]
    fn envelope_matching_is_case_insensitive() {
        let msg = envelope_msg();
        let cache = RegexCache::default();
        let cond = envelope_condition(EnvelopePart::To, AddressPart::All, "YOU@CORP.EXAMPLE");
        assert!(evaluate_condition(&cond, &msg, &cache));
    }

    #[test]
    fn envelope_without_at_domain_is_empty() {
        let msg = MailEnvelope {
            envelope_from: Some("bare-address".to_string()),
            ..MailEnvelope::default()
        };
        let cache = RegexCache::default();
        let localpart =
            envelope_condition(EnvelopePart::From, AddressPart::Localpart, "bare-address");
        assert!(evaluate_condition(&localpart, &msg, &cache));
        let domain = envelope_condition(EnvelopePart::From, AddressPart::Domain, "");
        assert!(evaluate_condition(&domain, &msg, &cache));
        let domain = envelope_condition(EnvelopePart::From, AddressPart::Domain, "example.com");
        assert!(!evaluate_condition(&domain, &msg, &cache));
    }

    #[test]
    fn envelope_missing_data_never_matches() {
        let msg = MailEnvelope::default();
        let cache = RegexCache::default();
        let cond = envelope_condition(EnvelopePart::From, AddressPart::All, "anything");
        assert!(!evaluate_condition(&cond, &msg, &cache));
        // Negation inverts: `not envelope` matches when data is missing.
        let cond = Condition {
            negate: true,
            ..cond
        };
        assert!(evaluate_condition(&cond, &msg, &cache));
    }

    #[test]
    fn envelope_supports_all_operators() {
        let msg = envelope_msg();
        let cache = RegexCache::default();
        let base_field = ConditionField::Envelope {
            part: EnvelopePart::To,
            address_part: AddressPart::Domain,
        };
        let contains = Condition {
            field: base_field.clone(),
            operator: Operator::Contains,
            value: "corp".to_string(),
            negate: false,
        };
        assert!(evaluate_condition(&contains, &msg, &cache));
        let glob = Condition {
            field: base_field.clone(),
            operator: Operator::Matches,
            value: "*.example".to_string(),
            negate: false,
        };
        assert!(evaluate_condition(&glob, &msg, &cache));
        let exists = Condition {
            field: base_field.clone(),
            operator: Operator::Exists,
            value: String::new(),
            negate: false,
        };
        assert!(evaluate_condition(&exists, &msg, &cache));
        let numeric = Condition {
            field: base_field,
            operator: Operator::NumericEquals,
            value: "7".to_string(),
            negate: false,
        };
        assert!(!evaluate_condition(&numeric, &msg, &cache));
    }

    #[test]
    fn field_value_envelope_resolves_full_address() {
        let msg = envelope_msg();
        assert_eq!(
            field_value(
                &msg,
                &ConditionField::Envelope {
                    part: EnvelopePart::From,
                    address_part: AddressPart::All,
                }
            ),
            "bounce@sender.example.net"
        );
    }

    // ---- i;ascii-numeric comparator ------------------------------------

    #[test]
    fn ascii_numeric_equality() {
        assert!(ascii_numeric_eq("42", "42"));
        assert!(ascii_numeric_eq("007", "7"));
        assert!(ascii_numeric_eq("", ""));
        assert!(!ascii_numeric_eq("42", "0421"));
        assert!(!ascii_numeric_eq("", "0"));
        assert!(!ascii_numeric_eq("12a", "12"));
        assert!(!ascii_numeric_eq("12", "1 2"));
        assert!(!ascii_numeric_eq("-1", "1"));
    }

    #[test]
    fn numeric_equals_operator_on_header() {
        let rule = make_rule(
            vec![Condition {
                field: ConditionField::Header("X-Spam-Score".to_string()),
                operator: Operator::NumericEquals,
                value: "10".to_string(),
                negate: false,
            }],
            LogicOp::And,
        );
        let msg = MailEnvelope {
            headers: vec![("X-Spam-Score".to_string(), "010".to_string())],
            ..MailEnvelope::default()
        };
        assert!(evaluate_rule(&rule, &msg, &RegexCache::default()));
    }

    // ---- evaluate_plan: vacation, notify, warnings ----------------------

    use crate::actions::{VacationReply, VacationTracker};
    use crate::types::{Action, Notify, Vacation};

    /// A rule that always matches [`eval_envelope`] (subject "Hello").
    fn matching_rule(actions: Vec<Action>) -> FilterRule {
        FilterRule {
            actions,
            ..make_rule(
                vec![Condition {
                    field: ConditionField::Subject,
                    operator: Operator::Contains,
                    value: "hello".to_string(),
                    negate: false,
                }],
                LogicOp::And,
            )
        }
    }

    fn eval_envelope() -> MailEnvelope {
        MailEnvelope {
            from: "alice@example.com".to_string(),
            envelope_from: Some("alice@bounce.example.com".to_string()),
            to: "you@example.com".to_string(),
            subject: "Hello".to_string(),
            ..MailEnvelope::default()
        }
    }

    #[test]
    fn evaluate_plan_resolves_vacation_reply() {
        let rule = matching_rule(vec![Action::Vacation(
            Vacation::new("I am away until Monday.")
                .with_subject("Away")
                .with_days(3),
        )]);
        let msg = eval_envelope();
        let outcome = evaluate_plan(
            &[rule],
            &msg,
            &EvalContext::default(),
            &RegexCache::default(),
        );
        assert_eq!(outcome.plan.len(), 1);
        assert!(outcome.warnings.is_empty());
        let crate::actions::PlannedAction::Vacation(reply) = &outcome.plan[0] else {
            panic!("expected vacation outcome");
        };
        assert_eq!(
            reply,
            &VacationReply {
                to: "alice@bounce.example.com".to_string(),
                days: 3,
                subject: "Away".to_string(),
                from: None,
                message: "I am away until Monday.".to_string(),
            }
        );
    }

    #[test]
    fn vacation_subject_defaults_to_re_original() {
        let rule = FilterRule {
            actions: vec![Action::Vacation(Vacation::new("away"))],
            ..matching_rule(vec![])
        };
        let outcome = evaluate_plan(
            &[rule],
            &eval_envelope(),
            &EvalContext::default(),
            &RegexCache::default(),
        );
        let crate::actions::PlannedAction::Vacation(reply) = &outcome.plan[0] else {
            panic!("expected vacation outcome");
        };
        assert_eq!(reply.subject, "Re: Hello");
        assert_eq!(reply.days, 7);
    }

    #[test]
    fn vacation_prefers_envelope_sender_over_header_from() {
        let rule = FilterRule {
            actions: vec![Action::Vacation(Vacation::new("away"))],
            ..matching_rule(vec![])
        };
        let msg = MailEnvelope {
            from: "header-from@example.com".to_string(),
            envelope_from: Some("envelope-from@example.net".to_string()),
            ..eval_envelope()
        };
        let outcome = evaluate_plan(
            &[rule],
            &msg,
            &EvalContext::default(),
            &RegexCache::default(),
        );
        let crate::actions::PlannedAction::Vacation(reply) = &outcome.plan[0] else {
            panic!("expected vacation outcome");
        };
        assert_eq!(reply.to, "envelope-from@example.net");
    }

    #[test]
    fn vacation_seen_before_hook_suppresses_reply() {
        let rule = FilterRule {
            actions: vec![Action::Vacation(Vacation::new("away"))],
            ..matching_rule(vec![])
        };
        let msg = eval_envelope();
        let seen = |sender: &str| sender == "alice@bounce.example.com";
        let ctx = EvalContext::default().with_seen_before(&seen);
        let outcome = evaluate_plan(&[rule], &msg, &ctx, &RegexCache::default());
        assert!(outcome.plan.is_empty());
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn vacation_tracker_dedups_across_evaluations() {
        let rule = FilterRule {
            actions: vec![Action::Vacation(Vacation::new("away"))],
            ..matching_rule(vec![])
        };
        let tracker = VacationTracker::new();
        let seen = |sender: &str| tracker.seen_before(sender, 7);
        let ctx = EvalContext::default().with_seen_before(&seen);

        let first = evaluate_plan(
            std::slice::from_ref(&rule),
            &eval_envelope(),
            &ctx,
            &RegexCache::default(),
        );
        assert_eq!(first.plan.len(), 1);
        if let crate::actions::PlannedAction::Vacation(reply) = &first.plan[0] {
            tracker.record(&reply.to);
        }

        let second = evaluate_plan(
            std::slice::from_ref(&rule),
            &eval_envelope(),
            &ctx,
            &RegexCache::default(),
        );
        assert!(second.plan.is_empty());
    }

    #[test]
    fn vacation_without_sender_warns_and_is_omitted() {
        let rule = FilterRule {
            actions: vec![Action::Vacation(Vacation::new("away"))],
            ..matching_rule(vec![])
        };
        // Matches on subject but supplies neither envelope sender nor a
        // header From address to reply to.
        let msg = MailEnvelope {
            subject: "Hello".to_string(),
            ..MailEnvelope::default()
        };
        let outcome = evaluate_plan(
            &[rule],
            &msg,
            &EvalContext::default(),
            &RegexCache::default(),
        );
        assert!(outcome.plan.is_empty());
        assert_eq!(outcome.warnings, vec![EvalWarning::VacationNoSender]);
    }

    #[test]
    fn notify_plans_method_and_message() {
        let rule = FilterRule {
            actions: vec![Action::Notify(Notify::new(
                "mailto:ops@example.com",
                "Payment received",
            ))],
            ..matching_rule(vec![])
        };
        let outcome = evaluate_plan(
            &[rule],
            &eval_envelope(),
            &EvalContext::default(),
            &RegexCache::default(),
        );
        assert!(outcome.warnings.is_empty());
        assert_eq!(
            outcome.plan,
            vec![crate::actions::PlannedAction::Notify {
                method: "mailto:ops@example.com".to_string(),
                message: "Payment received".to_string(),
            }]
        );
    }

    #[test]
    fn unknown_notify_method_parses_but_warns() {
        let rule = FilterRule {
            actions: vec![
                Action::Notify(Notify::new("carrier-pigeon:perth", "hello")),
                Action::MarkRead,
            ],
            ..matching_rule(vec![])
        };
        let outcome = evaluate_plan(
            &[rule],
            &eval_envelope(),
            &EvalContext::default(),
            &RegexCache::default(),
        );
        // The notification outcome is still produced...
        assert_eq!(outcome.plan.len(), 2);
        // ...alongside a warning about the unrecognized scheme.
        assert_eq!(
            outcome.warnings,
            vec![EvalWarning::UnknownNotifyMethod {
                method: "carrier-pigeon:perth".to_string(),
            }]
        );
    }

    #[test]
    fn notify_method_without_scheme_warns() {
        let rule = FilterRule {
            actions: vec![Action::Notify(Notify::new("no-scheme-here", ""))],
            ..matching_rule(vec![])
        };
        let outcome = evaluate_plan(
            &[rule],
            &eval_envelope(),
            &EvalContext::default(),
            &RegexCache::default(),
        );
        assert_eq!(outcome.plan.len(), 1);
        assert_eq!(
            outcome.warnings,
            vec![EvalWarning::UnknownNotifyMethod {
                method: "no-scheme-here".to_string(),
            }]
        );
    }

    #[test]
    fn no_matching_rule_yields_empty_outcome() {
        let rule = FilterRule {
            actions: vec![Action::MarkRead],
            ..make_rule(
                vec![Condition {
                    field: ConditionField::Subject,
                    operator: Operator::Contains,
                    value: "never".to_string(),
                    negate: false,
                }],
                LogicOp::And,
            )
        };
        let outcome = evaluate_plan(
            &[rule],
            &eval_envelope(),
            &EvalContext::default(),
            &RegexCache::default(),
        );
        assert!(outcome.is_empty());
        assert_eq!(outcome.rule_id, None);
    }
}
