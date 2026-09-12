# sieve-kit — claims inventory

Every verifiable numeric / behavioral performance claim in README.md and the
crate docs, mapped to its proof artifact. Generated as part of the
perf-claims proof-back pass (0.2.1). sieve-kit had **no benchmarks and no
numeric perf claims** before this pass; the benches added here establish
the measured baseline the docs now quote.

Status legend:

- **backed** — an existing bench/test asserts the claim; linked below.
- **code-backed** — enforced structurally in the code (not cheaply
  wall-clock-testable in CI).

## Claims

| Claim | Proof artifact | Status |
|---|---|---|
| Regex evaluation bounded: 100 ms post-check (`Operator::Regex`) | `src/eval.rs` (`REGEX_TIMEOUT_MS`, `evaluate_regex` post-hoc elapsed check) | code-backed |
| Invalid regex patterns never panic; evaluate as non-matching | `src/eval.rs::tests::regex_invalid_pattern_returns_false`, `::rule_validation_rejects_invalid_regex`, `tests/properties.rs` | backed |
| Evaluation is synchronous and I/O-free | structural: dependencies are `regex`, `serde`, `thiserror` only; no I/O in `src/` | code-backed |
| `evaluate_rule` / `evaluate_condition` / `evaluate_plan` never panic | doc contracts + unit tests + `tests/properties.rs` (arbitrary rules/messages) | backed |
| Envelope tests evaluate as non-matching when envelope data is missing | `src/eval.rs::tests::envelope_missing_data_never_matches` | backed |
| `VacationTracker` dedups respond-once-per-sender-per-period | `src/eval.rs::tests::vacation_tracker_dedups_across_evaluations` | backed |

## Measured baseline (new in 0.2.1)

No numeric perf claims existed in the docs before 0.2.1. The benches below
establish the baseline (criterion, development machine, x86-64, ~2 KiB
message, subject "Weekly digest #42"):

| Bench | Result (indicative) |
|---|---|
| `eval/contains_match` | ~0.1 µs/rule |
| `eval/contains_no_match` (2 KiB body scan) | ~0.6 µs/rule |
| `eval/glob_match` | ~0.3 µs/rule |
| `eval/regex_warm_cache` (2 KiB body) | ~33 µs/rule |
| `eval/numeric_equals` | ~0.08 µs/rule |
| `eval/envelope_domain` | ~0.14 µs/rule |
| `eval_plan/50_rules_last_match` | ~10 µs/plan |
| `parse/rule_from_json` | ~1.3 µs/rule |

Proof artifacts added in 0.2.1:

- `benches/eval_bench.rs` — criterion: per-operator `evaluate_rule`,
  50-rule `evaluate_plan` first-match scan, per-rule serde JSON parse.
- `benches/iai_eval.rs` — iai-callgrind instruction-count gate for the
  per-condition and per-rule eval hot paths (CI-only; requires valgrind).

## Summary

- Backed by existing artifacts: 4 (2 test-backed, 2 code-backed)
- Proven by artifacts added in this pass: 0 (no prior perf claims to
  prove; measured baseline established instead)
- Reworded or deleted: 0
