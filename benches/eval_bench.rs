//! Criterion benchmarks for rule parsing (serde) and evaluation.
//!
//! sieve-kit made no numeric perf claims before 0.2.1; these benches
//! establish the regression trend (see `CLAIMS.md`). Shapes:
//!
//! - `eval/*` — `evaluate_rule` per condition operator on a realistic
//!   2 KiB message (match and no-match variants)
//! - `eval_plan/50_rules` — `evaluate_plan` first-match scan over a
//!   realistic 50-rule mailbox where the match is near the end
//! - `parse/rule_from_json` — `serde_json` deserialization of a rule
//!
//! Deterministic instruction-count gate: `benches/iai_eval.rs`.
//!
//! Run: `cargo bench --bench eval_bench`

// Test/bench code: unwrap/expect are the idiomatic way to assert outcomes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sieve_kit::eval::{EvalContext, RegexCache, evaluate_plan, evaluate_rule};
use sieve_kit::types::{
    Action, Condition, ConditionField, FilterRule, LogicOp, MailEnvelope, Operator,
};

/// A realistic message: ~2 KiB body, a few headers.
fn message() -> MailEnvelope {
    let body = "Hello, this is a sample newsletter body. ".repeat(50);
    MailEnvelope {
        from: "news@lists.example.com".to_string(),
        to: "you@corp.example".to_string(),
        cc: String::new(),
        subject: "Weekly digest #42".to_string(),
        body,
        has_attachment: false,
        envelope_from: Some("bounce@lists.example.com".to_string()),
        envelope_to: Some("you@corp.example".to_string()),
        headers: vec![
            (
                "List-Id".to_string(),
                "<news.lists.example.com>".to_string(),
            ),
            ("X-Spam-Score".to_string(), "3".to_string()),
        ],
    }
}

fn condition(field: ConditionField, operator: Operator, value: &str) -> Condition {
    Condition {
        field,
        operator,
        value: value.to_string(),
        negate: false,
    }
}

fn rule(id: &str, conditions: Vec<Condition>, actions: Vec<Action>) -> FilterRule {
    FilterRule {
        id: id.to_string(),
        name: format!("rule-{id}"),
        enabled: true,
        priority: 0,
        conditions,
        condition_logic: LogicOp::And,
        actions,
    }
}

/// A realistic mailbox: 49 rules that do not match (varied operators) and
/// a matching newsletters rule last.
fn fifty_rule_mailbox() -> Vec<FilterRule> {
    let mut rules: Vec<FilterRule> = (0..49u32)
        .map(|i| {
            let (field, operator, value) = match i % 4 {
                0 => (
                    ConditionField::Subject,
                    Operator::Contains,
                    format!("nominal-{i}"),
                ),
                1 => (
                    ConditionField::From,
                    Operator::Matches,
                    format!("sender-{i}@*.org"),
                ),
                2 => (
                    ConditionField::Header(format!("X-Bench-{i}")),
                    Operator::Exists,
                    String::new(),
                ),
                _ => (
                    ConditionField::Subject,
                    Operator::Regex,
                    r"(?i)unmatchable[- ]pattern[- ]\d+".to_string(),
                ),
            };
            rule(
                &format!("r{i}"),
                vec![condition(field, operator, &value)],
                vec![Action::MarkRead],
            )
        })
        .collect();
    rules.push(rule(
        "newsletters",
        vec![condition(
            ConditionField::Subject,
            Operator::Contains,
            "digest",
        )],
        vec![Action::MoveTo("Newsletters".to_string())],
    ));
    rules
}

fn bench_eval_operators(c: &mut Criterion) {
    let msg = message();
    let cache = RegexCache::default();

    let mut group = c.benchmark_group("eval");
    group.bench_function("contains_match", |b| {
        let r = rule(
            "r",
            vec![condition(
                ConditionField::Subject,
                Operator::Contains,
                "digest",
            )],
            vec![],
        );
        b.iter(|| assert!(evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.bench_function("contains_no_match", |b| {
        let r = rule(
            "r",
            vec![condition(
                ConditionField::Body,
                Operator::Contains,
                "totally-absent-token",
            )],
            vec![],
        );
        b.iter(|| assert!(!evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.bench_function("glob_match", |b| {
        let r = rule(
            "r",
            vec![condition(
                ConditionField::From,
                Operator::Matches,
                "news@*.example.com",
            )],
            vec![],
        );
        b.iter(|| assert!(evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.bench_function("regex_warm_cache", |b| {
        let r = rule(
            "r",
            vec![condition(
                ConditionField::Subject,
                Operator::Regex,
                r"(?i)weekly\s+digest\s+#\d+",
            )],
            vec![],
        );
        // Compile once so the measured path is the warm-cache lookup.
        assert!(evaluate_rule(&r, &msg, &cache));
        b.iter(|| assert!(evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.bench_function("numeric_equals", |b| {
        let r = rule(
            "r",
            vec![condition(
                ConditionField::Header("X-Spam-Score".to_string()),
                Operator::NumericEquals,
                "3",
            )],
            vec![],
        );
        b.iter(|| assert!(evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.bench_function("envelope_domain", |b| {
        let r = rule(
            "r",
            vec![Condition {
                field: ConditionField::Envelope {
                    part: sieve_kit::types::EnvelopePart::From,
                    address_part: sieve_kit::types::AddressPart::Domain,
                },
                operator: Operator::Equals,
                value: "lists.example.com".to_string(),
                negate: false,
            }],
            vec![],
        );
        b.iter(|| assert!(evaluate_rule(black_box(&r), black_box(&msg), &cache)))
    });
    group.finish();
}

fn bench_eval_plan(c: &mut Criterion) {
    let msg = message();
    let cache = RegexCache::default();
    let rules = fifty_rule_mailbox();
    let mut group = c.benchmark_group("eval_plan");
    group.bench_function("50_rules_last_match", |b| {
        b.iter(|| {
            let outcome = evaluate_plan(
                black_box(&rules),
                black_box(&msg),
                &EvalContext::default(),
                &cache,
            );
            assert_eq!(outcome.rule_id.as_deref(), Some("newsletters"));
        })
    });
    group.finish();
}

fn bench_parse(c: &mut Criterion) {
    let rule = fifty_rule_mailbox().pop().unwrap();
    let json = serde_json::to_string_pretty(&rule).unwrap();
    let mut group = c.benchmark_group("parse");
    group.bench_function("rule_from_json", |b| {
        b.iter(|| {
            let parsed: FilterRule = serde_json::from_str(black_box(&json)).unwrap();
            assert_eq!(parsed.id, "newsletters");
        })
    });
    group.finish();
}

criterion_group!(benches, bench_eval_operators, bench_eval_plan, bench_parse);
criterion_main!(benches);
