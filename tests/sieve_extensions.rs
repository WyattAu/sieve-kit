//! Integration tests for the 0.2.0 extensions: `vacation` (RFC 5230),
//! `notify` (RFC 5436), IMAP flag mutations (RFC 5232), the `envelope`
//! test (RFC 5228 §5.1), and the `i;ascii-numeric` comparator.
//!
//! Each test drives the full pipeline: parse/validate rules → evaluate
//! against a message → assert on the produced outcomes.

#![forbid(unsafe_code)]

use sieve_kit::actions::{VacationReply, VacationTracker, apply_flag_plan};
use sieve_kit::eval::{EvalContext, RegexCache, evaluate_plan, evaluate_rule};
use sieve_kit::types::{
    Action, AddressPart, Condition, ConditionField, EnvelopePart, FilterRule, Flag, LogicOp,
    MailEnvelope, Notify, Operator, Vacation,
};

fn cache() -> RegexCache {
    RegexCache::default()
}

fn rule(id: &str, conditions: Vec<Condition>, actions: Vec<Action>) -> FilterRule {
    FilterRule {
        id: id.to_string(),
        name: id.to_string(),
        enabled: true,
        priority: 0,
        conditions,
        condition_logic: LogicOp::And,
        actions,
    }
}

fn incoming(from: &str, subject: &str) -> MailEnvelope {
    MailEnvelope {
        from: from.to_string(),
        envelope_from: Some(from.to_string()),
        to: "you@corp.example".to_string(),
        envelope_to: Some("you@corp.example".to_string()),
        subject: subject.to_string(),
        ..MailEnvelope::default()
    }
}

/// Subject-contains condition — the shared "match me" clause.
fn about(needle: &str) -> Condition {
    Condition {
        field: ConditionField::Subject,
        operator: Operator::Contains,
        value: needle.to_string(),
        negate: false,
    }
}

// ---- vacation (RFC 5230) ----------------------------------------------------

#[test]
fn vacation_full_pipeline_reply_outcome() {
    let rules = vec![rule(
        "ooo",
        vec![about("invoice")],
        vec![Action::Vacation(
            Vacation::new("I am on leave; back Monday.")
                .with_days(3)
                .with_subject("Out of office")
                .with_from("assistant@corp.example"),
        )],
    )];

    assert!(rules.iter().try_for_each(FilterRule::validate).is_ok());

    let msg = incoming("client@example.com", "Invoice question");
    let outcome = evaluate_plan(&rules, &msg, &EvalContext::default(), &cache());

    assert_eq!(outcome.rule_id.as_deref(), Some("ooo"));
    assert!(outcome.warnings.is_empty());
    assert_eq!(
        outcome.plan,
        vec![sieve_kit::PlannedAction::Vacation(VacationReply {
            to: "client@example.com".to_string(),
            days: 3,
            subject: "Out of office".to_string(),
            from: Some("assistant@corp.example".to_string()),
            message: "I am on leave; back Monday.".to_string(),
        })]
    );
}

#[test]
fn vacation_responds_once_per_sender_per_period() {
    let rules = vec![rule(
        "ooo",
        vec![about("urgent")],
        vec![Action::Vacation(Vacation::new("Away.").with_days(7))],
    )];
    let tracker = VacationTracker::new();
    let seen = |sender: &str| tracker.seen_before(sender, 7);
    let ctx = EvalContext::default().with_seen_before(&seen);

    // First message from alice: reply planned, host records it.
    let first = evaluate_plan(
        &rules,
        &incoming("alice@example.com", "urgent"),
        &ctx,
        &cache(),
    );
    assert_eq!(first.plan.len(), 1);
    let sieve_kit::PlannedAction::Vacation(reply) = &first.plan[0] else {
        panic!("expected vacation outcome");
    };
    tracker.record(&reply.to);

    // Second message from alice the same day: suppressed.
    let second = evaluate_plan(
        &rules,
        &incoming("alice@example.com", "urgent again"),
        &ctx,
        &cache(),
    );
    assert!(second.plan.is_empty());
    assert!(second.warnings.is_empty());

    // A different sender the same day: still answered.
    let third = evaluate_plan(
        &rules,
        &incoming("bob@example.com", "urgent"),
        &ctx,
        &cache(),
    );
    assert_eq!(third.plan.len(), 1);

    // Without a dedup hook the engine hands the decision to the host: the
    // outcome carries the sender and period needed to dedup externally.
    let no_hook = evaluate_plan(
        &rules,
        &incoming("alice@example.com", "urgent"),
        &EvalContext::default(),
        &cache(),
    );
    let sieve_kit::PlannedAction::Vacation(reply) = &no_hook.plan[0] else {
        panic!("expected vacation outcome");
    };
    assert_eq!(reply.to, "alice@example.com");
    assert_eq!(reply.days, 7);
}

#[test]
fn vacation_invalid_arguments_are_rejected_at_construction() {
    // Empty message body.
    let bad_message = rule(
        "bad",
        vec![about("x")],
        vec![Action::Vacation(Vacation::new(""))],
    );
    assert!(matches!(
        bad_message.validate(),
        Err(sieve_kit::FilterError::InvalidVacation { .. })
    ));
    // Zero-day respond period.
    let bad_days = rule(
        "bad",
        vec![about("x")],
        vec![Action::Vacation(Vacation::new("m").with_days(0))],
    );
    assert!(matches!(
        bad_days.validate(),
        Err(sieve_kit::FilterError::InvalidVacation { .. })
    ));
}

#[test]
fn vacation_unroutable_reply_warns_and_rest_of_plan_still_runs() {
    let rules = vec![rule(
        "ooo",
        vec![about("hello")],
        vec![
            Action::Vacation(Vacation::new("Away.")),
            Action::Flag(vec![Flag::Seen]),
        ],
    )];
    // Message matches but has neither envelope sender nor a From header.
    let msg = MailEnvelope {
        subject: "hello".to_string(),
        ..MailEnvelope::default()
    };
    let outcome = evaluate_plan(&rules, &msg, &EvalContext::default(), &cache());
    assert_eq!(
        outcome.plan,
        vec![sieve_kit::PlannedAction::AddFlags {
            flags: vec![Flag::Seen]
        }]
    );
    assert_eq!(
        outcome.warnings,
        vec![sieve_kit::EvalWarning::VacationNoSender]
    );
}

// ---- notify (RFC 5436) ------------------------------------------------------

#[test]
fn notify_full_pipeline_outcome_with_method_and_message() {
    let rules = vec![rule(
        "payments",
        vec![about("receipt")],
        vec![Action::Notify(Notify::new(
            "mailto:finance@corp.example",
            "A receipt arrived",
        ))],
    )];
    let outcome = evaluate_plan(
        &rules,
        &incoming("billing@payments.example.com", "Your receipt"),
        &EvalContext::default(),
        &cache(),
    );
    assert!(outcome.warnings.is_empty());
    assert_eq!(
        outcome.plan,
        vec![sieve_kit::PlannedAction::Notify {
            method: "mailto:finance@corp.example".to_string(),
            message: "A receipt arrived".to_string(),
        }]
    );
}

#[test]
fn notify_unknown_scheme_evaluates_with_warning() {
    let rules = vec![rule(
        "pings",
        vec![about("alert")],
        vec![
            Action::Notify(Notify::new("pagerduty:service-7", "page on call")),
            Action::MoveTo("Alerts".to_string()),
        ],
    )];
    // Parses and validates fine — the URI scheme list is open-ended.
    assert!(rules.iter().try_for_each(FilterRule::validate).is_ok());

    let outcome = evaluate_plan(
        &rules,
        &incoming("monitor@example.com", "alert: disk"),
        &EvalContext::default(),
        &cache(),
    );
    // The notification is still planned; the host decides what to do with it.
    assert_eq!(outcome.plan.len(), 2);
    assert_eq!(
        outcome.warnings,
        vec![sieve_kit::EvalWarning::UnknownNotifyMethod {
            method: "pagerduty:service-7".to_string(),
        }]
    );
}

// ---- IMAP flags (RFC 5232) --------------------------------------------------

#[test]
fn flag_mutations_evaluate_to_outcomes() {
    let rules = vec![rule(
        "triage",
        vec![about("report")],
        vec![
            Action::Flag(vec![Flag::Flagged, Flag::Keyword("quarterly".into())]),
            Action::Unflag(vec![Flag::Seen]),
            Action::SetFlags(vec![Flag::Answered]),
        ],
    )];
    let outcome = evaluate_plan(
        &rules,
        &incoming("boss@corp.example", "Q3 report"),
        &EvalContext::default(),
        &cache(),
    );
    assert_eq!(
        outcome.plan,
        vec![
            sieve_kit::PlannedAction::AddFlags {
                flags: vec![Flag::Flagged, Flag::Keyword("quarterly".into())]
            },
            sieve_kit::PlannedAction::RemoveFlags {
                flags: vec![Flag::Seen]
            },
            sieve_kit::PlannedAction::SetFlags {
                flags: vec![Flag::Answered]
            },
        ]
    );

    // Folding the plan onto a current flag set follows RFC 5232 semantics:
    // add (dedup), remove, replace.
    let flags = apply_flag_plan(&[Flag::Seen, Flag::Keyword("stale".into())], &outcome.plan);
    assert_eq!(flags, vec![Flag::Answered]);
}

// ---- envelope test (RFC 5228 §5.1) ------------------------------------------

#[test]
fn envelope_test_matches_caller_supplied_values() {
    let rules = vec![rule(
        "bounces",
        vec![Condition {
            field: ConditionField::Envelope {
                part: EnvelopePart::From,
                address_part: AddressPart::Domain,
            },
            operator: Operator::Equals,
            value: "bounce.lists.example.net".to_string(),
            negate: false,
        }],
        vec![Action::MoveTo("Lists".to_string())],
    )];

    // Envelope domain matches even though the header From differs.
    let msg = MailEnvelope {
        envelope_from: Some("bounce@bounce.lists.example.net".to_string()),
        ..incoming("news@lists.example.net", "Your digest")
    };
    assert!(evaluate_rule(&rules[0], &msg, &cache()));
    let outcome = evaluate_plan(&rules, &msg, &EvalContext::default(), &cache());
    assert_eq!(outcome.rule_id.as_deref(), Some("bounces"));

    // Different envelope domain: no match.
    let other = incoming("news@lists.example.net", "Your digest");
    let other = MailEnvelope {
        envelope_from: Some("news@elsewhere.example".to_string()),
        ..other
    };
    assert!(!evaluate_rule(&rules[0], &other, &cache()));

    // No envelope data at all: the test cannot match.
    let bare = MailEnvelope {
        from: "news@bounce.lists.example.net".to_string(),
        subject: "Your digest".to_string(),
        ..MailEnvelope::default()
    };
    assert!(!evaluate_rule(&rules[0], &bare, &cache()));
}

#[test]
fn envelope_test_localpart_and_to_part() {
    let localpart_rule = rule(
        "postmaster",
        vec![Condition {
            field: ConditionField::Envelope {
                part: EnvelopePart::To,
                address_part: AddressPart::Localpart,
            },
            operator: Operator::Equals,
            value: "postmaster".to_string(),
            negate: false,
        }],
        vec![Action::Forward("abuse@corp.example".to_string())],
    );

    for addr in ["postmaster@corp.example", "POSTMASTER@other.example"] {
        let msg = MailEnvelope {
            envelope_to: Some(addr.to_string()),
            ..MailEnvelope::default()
        };
        assert!(
            evaluate_rule(&localpart_rule, &msg, &cache()),
            "should match {addr}"
        );
    }

    let msg = MailEnvelope {
        envelope_to: Some("admin@corp.example".to_string()),
        ..MailEnvelope::default()
    };
    assert!(!evaluate_rule(&localpart_rule, &msg, &cache()));
}

// ---- i;ascii-numeric comparator ---------------------------------------------

#[test]
fn ascii_numeric_comparator_on_header_and_address_tests() {
    // The numeric comparator is ordinary operator plumbing: it applies to
    // every string test, including headers and addresses.
    let rules = [rule(
        "sasl",
        vec![
            Condition {
                field: ConditionField::Header("X-Spam-Level".to_string()),
                operator: Operator::NumericEquals,
                value: "05".to_string(),
                negate: false,
            },
            Condition {
                field: ConditionField::From,
                operator: Operator::NumericEquals,
                value: "42".to_string(),
                negate: false,
            },
        ],
        vec![Action::MoveTo("Numbers".to_string())],
    )];

    let msg = MailEnvelope {
        headers: vec![("X-Spam-Level".to_string(), "5".to_string())],
        ..incoming("42", "hi")
    };
    assert!(evaluate_rule(&rules[0], &msg, &cache()));

    // Non-numeric operands never match, per RFC 4790.
    let msg = MailEnvelope {
        headers: vec![("X-Spam-Level".to_string(), "five".to_string())],
        ..incoming("42", "hi")
    };
    assert!(!evaluate_rule(&rules[0], &msg, &cache()));
}
