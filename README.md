# sieve-kit

Rule-based mail filtering engine for Rust: conditions → actions
(move / copy / flag / mark-read / delete / forward), plus evaluated
`vacation` and `notify` outcomes, IMAP flag mutations, and SMTP `envelope`
tests.

- Synchronous, I/O-free, `#![forbid(unsafe_code)]`
- Generic over any message type via the `Filterable` trait
  (or use the provided `MailEnvelope` struct)
- Operators: contains, equals, glob (`*`/`?`), bounded regex, exists,
  numeric (`i;ascii-numeric`)
- AND/OR condition logic with per-condition negation
- Action plans as plain values — execution is the host's responsibility
- Invalid regex patterns never panic; they evaluate as non-matching
- Rules are `serde`-serializable data — load them from JSON, TOML, or a DB

Extracted from the [kestrel](https://github.com/WyattAu/kestrel) mail client.

## Example

[`examples/mail_pipeline.rs`](examples/mail_pipeline.rs) implements the full
pipeline — receive → evaluate → dispatch — over an in-memory mailbox with
four realistic rules (newsletters, payments, junk, attachments from
strangers) and five incoming messages.

[`examples/vacation_filter.rs`](examples/vacation_filter.rs) shows the 0.2.0
extensions end-to-end: an away-mode rule set with vacation replies deduped
through `VacationTracker`, an on-call notify, envelope-based list
exclusion, and flag mutations. Run either with:

```text
cargo run --example mail_pipeline
cargo run --example vacation_filter
```

A minimal evaluation looks like this:

```rust
use sieve_kit::eval::{evaluate_rule, RegexCache};
use sieve_kit::types::{
    Condition, ConditionField, FilterRule, LogicOp, MailEnvelope, Operator,
};

let rule = FilterRule {
    id: "r1".into(),
    name: "Newsletters".into(),
    enabled: true,
    priority: 0,
    conditions: vec![Condition {
        field: ConditionField::Subject,
        operator: Operator::Contains,
        value: "digest".into(),
        negate: false,
    }],
    condition_logic: LogicOp::And,
    actions: vec![sieve_kit::types::Action::MoveTo("Newsletters".into())],
};
let msg = MailEnvelope {
    subject: "Weekly digest".into(),
    ..MailEnvelope::default()
};
assert!(evaluate_rule(&rule, &msg, &RegexCache::default()));
```

## RFC coverage / feature table

**sieve-kit is not a Sieve-language interpreter.** It implements a typed,
serde-friendly rule model that covers a practical subset of RFC 5228
*semantics* plus its most-used extensions, for applications that own their
rule representation (config files, database rows, a rules-builder UI)
rather than parsing user-written Sieve scripts.

| Feature | RFC | sieve-kit API |
|---|---|---|
| `header` / `address` tests | 5228 | `ConditionField::{From, To, Cc, Header, Subject, Body}` |
| `exists` test | 5228 | `Operator::Exists` |
| `allof` / `anyof` / `not` | 5228 | `LogicOp::{And, Or}` + per-condition `negate` |
| `is` (comparator `i;ascii-casemap`) | 5228 | `Operator::Equals` |
| `contains` | 5228 §2.7.1 | `Operator::Contains` |
| `matches` (glob wildcards) | 5228 §2.7.1 | `Operator::Matches` |
| `size`-style regex extension | regex-0.8.2 | `Operator::Regex` (100 ms bounded) |
| **`envelope` test** (`:all`/`:localpart`/`:domain`) | **5228 §5.1** | `ConditionField::Envelope` + `Filterable::envelope_from/to` |
| `fileinto` | 5228 | `Action::MoveTo` / `Action::CopyTo` → `PlannedAction::{Move, Copy}` |
| `redirect` | 5228 | `Action::Forward` → `PlannedAction::Forward` |
| `discard` | 5228 | `Action::Delete` (to trash — host decides) |
| implicit `keep` | 5228 | empty plan when no rule matches |
| **`addflag` / `removeflag` / `setflag`** | **5232** | `Action::{Flag, Unflag, SetFlags}` + `apply_flag_plan` |
| **`vacation`** (`:days`/`:subject`/`:from`) | **5230** | `Action::Vacation` → `PlannedAction::Vacation` + `VacationTracker` dedup hook |
| **`notify`** (`:method`/`:message`) | **5436** | `Action::Notify` → `PlannedAction::Notify` + scheme warnings |
| **comparator `i;ascii-numeric`** | **4790** | `Operator::NumericEquals` |
| has-attachment (sieve-kit-specific) | — | `ConditionField::HasAttachment` |

Vacation and notify are **evaluated, not executed**: the engine resolves
routing, defaults, and respond-once-per-sender-per-period dedup
(via the caller-supplied `seen_before` hook or the built-in
`VacationTracker`), and returns inert outcome values — the host owns
actual SMTP/notification delivery. See
[`examples/vacation_filter.rs`](examples/vacation_filter.rs).

### Evaluation entry points

- `evaluate_rule` / `evaluate_condition` — boolean matching
- `collect_matches` — first-match actions
- `evaluate_plan` — first-match **outcomes** with vacation dedup, notify
  scheme warnings, and envelope/flag resolution

sieve-kit-specific (no RFC counterpart): `ConditionField::HasAttachment`.

Not implemented (future work):

- **The RFC 5228 grammar itself** — there is no parser or serializer for
  Sieve script text: no `require`, no string lists, no multiline `text:`
  blocks, no comments, no string interpolation. Rules are Rust structs
  (or their serde form).
- **Multiple rule firing** — evaluation returns the *first* match's
  outcomes (an implicit `stop`). Cross-rule action accumulation and
  conflict resolution are host concerns today.
- Vacation `:addresses` / `:mime` / `:handle`, `reject`, `relational`
  (`:count` / `:value`), comparator *selection* syntax, `variables`,
  `mime`, and a `size` test (messages have no size field yet).

If you need full Sieve *script* compatibility today, see
[COMPARISON.md](COMPARISON.md) for the alternatives and where sieve-kit fits.

## Benchmarks

`benches/eval_bench.rs` (criterion) measures per-rule evaluation cost per
condition operator, a 50-rule `evaluate_plan` first-match scan, and per-rule
serde JSON parsing, on a realistic ~2 KiB message. `benches/iai_eval.rs`
([iai-callgrind](https://github.com/iai-callgrind/iai-callgrind)) is the
deterministic instruction-count regression gate for the per-condition and
per-rule eval hot paths (CI-only; requires valgrind).

Indicative numbers from a development machine (x86-64, idle):

| Bench | Result |
|---|---|
| `eval/contains_match` | ~0.1 µs/rule |
| `eval/contains_no_match` (2 KiB body scan) | ~0.6 µs/rule |
| `eval/glob_match` | ~0.3 µs/rule |
| `eval/regex_warm_cache` (2 KiB body) | ~33 µs/rule |
| `eval/numeric_equals` | ~0.08 µs/rule |
| `eval/envelope_domain` | ~0.14 µs/rule |
| `eval_plan/50_rules_last_match` | ~10 µs/plan |
| `parse/rule_from_json` | ~1.3 µs/rule |

Every claim in this README is mapped to its proof artifact in
[CLAIMS.md](CLAIMS.md).

```text
cargo bench --bench eval_bench
```

## License

MIT OR Apache-2.0
