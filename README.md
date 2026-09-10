# sieve-kit

Rule-based mail filtering engine for Rust: conditions → actions
(move / copy / flag / mark-read / delete / forward).

- Synchronous, I/O-free, `#![forbid(unsafe_code)]`
- Generic over any message type via the `Filterable` trait
  (or use the provided `MailEnvelope` struct)
- Operators: contains, equals, glob (`*`/`?`), bounded regex, exists
- AND/OR condition logic with per-condition negation
- Action plans as plain values — execution is the host's responsibility
- Invalid regex patterns never panic; they evaluate as non-matching
- Rules are `serde`-serializable data — load them from JSON, TOML, or a DB

Extracted from the [kestrel](https://github.com/WyattAu/kestrel) mail client.

## Example

[`examples/mail_pipeline.rs`](examples/mail_pipeline.rs) implements the full
pipeline — receive → evaluate → dispatch — over an in-memory mailbox with
four realistic rules (newsletters, payments, junk, attachments from
strangers) and five incoming messages. Run it with:

```text
cargo run --example mail_pipeline
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

## Relationship to RFC 5228 (Sieve)

**sieve-kit is not a Sieve-language interpreter.** It implements a typed,
serde-friendly rule model that covers a practical subset of RFC 5228
*semantics*, for applications that own their rule representation (config
files, database rows, a rules-builder UI) rather than parsing user-written
Sieve scripts.

Implemented, mapped to RFC 5228 concepts:

| RFC 5228 | sieve-kit |
|---|---|
| `address`, `header` tests | `ConditionField::{From, To, Cc, Header}` |
| `subject` / body matching | `ConditionField::{Subject, Body}` |
| `exists` test | `Operator::Exists` |
| `contains` (comparator `i;ascii-casemap`) | `Operator::Contains` |
| `is` | `Operator::Equals` |
| `matches` (glob wildcards) | `Operator::Matches` |
| regex extension (`regex-0.8.2`-style) | `Operator::Regex` (100 ms bounded) |
| `allof` / `anyof` | `LogicOp::{And, Or}` |
| `not` | per-condition `negate` flag |
| `fileinto` | `Action::MoveTo` / `Action::CopyTo` |
| `imap4flags` (system flags) | `Action::Flag` |
| `mark` / `\Seen` | `Action::MarkRead` |
| `discard` | `Action::Delete` (to trash — host decides) |
| `redirect` | `Action::Forward` |

sieve-kit-specific (no RFC counterpart): `ConditionField::HasAttachment`.

Not implemented (future work):

- **The RFC 5228 grammar itself** — there is no parser or serializer for
  Sieve script text: no `require`, no string lists, no multiline `text:`
  blocks, no comments, no string interpolation. Rules are Rust structs
  (or their serde form).
- **Multiple rule firing** — evaluation returns the *first* match's actions
  (an implicit `stop`). RFC `keep` bookkeeping, cross-rule action
  accumulation, and conflict resolution are host concerns today.
- **Extensions** — `vacation`, `envelope`, `relational` (`:count` /
  `:value`), comparator selection, `variables`, `mime`, `reject`.
- A `size` test (messages have no size field yet).

If you need full Sieve *script* compatibility today, see
[COMPARISON.md](COMPARISON.md) for the alternatives and where sieve-kit fits.

## License

MIT OR Apache-2.0
