// iai-callgrind benchmarks run once under Valgrind on fixed inputs; the
// harness measures instruction counts, so there is no "expected failure"
// recovery path — a panic aborts the run visibly, which is what we want.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Deterministic regression gate for rule parsing and evaluation.
//!
//! Unlike criterion (wall-clock trend — `benches/eval_bench.rs`),
//! iai-callgrind counts CPU instructions under Valgrind and is
//! reproducible for a given binary — fit for a CI gate.
//!
//! Paths pinned:
//!
//! - `eval_condition_contains` — the per-condition hot path
//! - `eval_rule_3_conditions` — per-rule evaluation (AND of 3)
//!
//! Parsing (`serde_json` deserialization) is trended in
//! `benches/eval_bench.rs::parse::rule_from_json`.

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use sieve_kit::eval::{RegexCache, evaluate_condition, evaluate_rule};
use sieve_kit::types::{Condition, ConditionField, FilterRule, LogicOp, MailEnvelope, Operator};

fn message() -> MailEnvelope {
    MailEnvelope {
        from: "news@lists.example.com".to_string(),
        to: "you@corp.example".to_string(),
        cc: String::new(),
        subject: "Weekly digest #42".to_string(),
        body: "Sample body for the benchmark. ".repeat(32),
        has_attachment: false,
        envelope_from: None,
        envelope_to: None,
        headers: vec![],
    }
}

fn contains_condition(value: &str) -> Condition {
    Condition {
        field: ConditionField::Subject,
        operator: Operator::Contains,
        value: value.to_string(),
        negate: false,
    }
}

fn setup_eval() -> (MailEnvelope, Condition, RegexCache) {
    (
        message(),
        contains_condition("digest"),
        RegexCache::default(),
    )
}

fn setup_eval_rule() -> (MailEnvelope, FilterRule, RegexCache) {
    let msg = message();
    let rule = FilterRule {
        id: "r".to_string(),
        name: "bench".to_string(),
        enabled: true,
        priority: 0,
        conditions: vec![
            contains_condition("digest"),
            Condition {
                field: ConditionField::From,
                operator: Operator::Matches,
                value: "news@*.example.com".to_string(),
                negate: false,
            },
            Condition {
                field: ConditionField::Header("X-Spam-Score".to_string()),
                operator: Operator::NumericEquals,
                value: "3".to_string(),
                negate: false,
            },
        ],
        condition_logic: LogicOp::And,
        actions: vec![],
    };
    (msg, rule, RegexCache::default())
}

// One condition against one message: the innermost eval loop.
#[library_benchmark]
#[bench::contains(setup = setup_eval)]
fn eval_condition_contains(env: (MailEnvelope, Condition, RegexCache)) -> bool {
    let (msg, cond, cache) = env;
    evaluate_condition(&cond, &msg, &cache)
}

// One rule, three ANDed conditions: the per-rule hot path.
#[library_benchmark]
#[bench::three_conditions(setup = setup_eval_rule)]
fn eval_rule_3_conditions(env: (MailEnvelope, FilterRule, RegexCache)) -> bool {
    let (msg, rule, cache) = env;
    evaluate_rule(&rule, &msg, &cache)
}

library_benchmark_group!(
    name = iai_eval_hot_path;
    benchmarks = eval_condition_contains, eval_rule_3_conditions
);

main!(library_benchmark_groups = iai_eval_hot_path);
