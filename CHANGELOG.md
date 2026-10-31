Changelog](https://keepachangelog.com/); versions follow [semver](https://semver.org).
- First benchmark suite: `benches/eval_bench.rs` (criterion); per-operator
  serde JSON parse; and `benches/iai_eval.rs` (iai-callgrind,
- **comparator `i;ascii-numeric` (RFC 4790)**: `Operator::NumericEquals` -
  `EvalContext`/`EvalOutcome`/`EvalWarning`; first-match evaluation that
  pipeline over an in-memory mailbox; four rules (newsletters, payments,