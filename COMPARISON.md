# sieve-kit vs. the alternatives

Status of the Rust mail-filtering ecosystem as of **September 2026**.
Numbers are crates.io figures at the time of writing; check the linked
crates for current data.

| | **sieve-kit** | `mailrs-sieve` | `sieve` (crates.io) |
|---|---|---|---|
| What it is | typed rule engine (conditions → action plan) | RFC 5228 Sieve **script** interpreter | **prime-number sieve** — not mail |
| Status | maintained | active (2026) | dead since Aug 2018 |
| Version | 0.2.0 | 2.0.1 | 0.1.0 |
| Downloads | 8 | ~105 total, ~37 recent | ~2k total, ~14 recent |
| Input format | `FilterRule` structs (serde: JSON/TOML/DB rows) | Sieve script text (RFC 5228 grammar) | — |
| Semantics | first-match-wins, priority-ordered | full RFC 5228 action stream (Keep, FileInto, Discard, Redirect, Reject, Vacation) | — |
| Tests/operators | 9 fields incl. `envelope`, 6 operators incl. bounded regex + `i;ascii-numeric` | full RFC 5228 + extensions | — |
| Extensions | `vacation` (RFC 5230, evaluated + dedup hook), `notify` (RFC 5436), IMAP flags (RFC 5232) | engine-dependent | — |
| Dependencies | `regex`, `serde`, `thiserror` | engine + parser stack (wraps a full Sieve engine) | — |
| Execution model | pure, sync, I/O-free; host executes the plan | delivery-loop integration | — |
| `unsafe` | forbidden (`#![forbid(unsafe_code)]`) | — | — |

## `sieve` (crates.io/sieve)

Despite the name, the crate named `sieve` on crates.io has **nothing to do
with email**: it is a 7-line segmented *prime* sieve ("Fast segmented sieve
of Erasthotenes"), published once as 0.1.0 in **August 2018** and untouched
since (~14 recent downloads). It is the only holder of the exact name, so
the RFC 5228 space effectively has no crate under it — and it is not a
competitor, just an unfortunate naming collision.

## `mailrs-sieve` (crates.io/mailrs-sieve)

The one live alternative. Released in 2026 (1.0.0 in May, 2.0.1 in August
2026; ~105 downloads), single maintainer, part of the
[mailrs](https://github.com/goliajp/mailrs) mail project. It parses real
Sieve script text and exposes the engine's decisions as a flat
`SieveAction { Keep, FileInto, Discard, Redirect, Reject, Vacation }` enum
for your delivery loop; version 2.0 moved from wrapping Stalwart's
`sieve-rs` to a native engine.

It is the right tool when **users author filters in Sieve syntax** and you
must be grammar-compatible (a Dovecot-class delivery agent, importable
server-side rules). That grammar support is also its weight: a script
parser and engine behind a delivery-oriented API.

## Where sieve-kit fits

sieve-kit targets the case where **your application owns the rule format**:

- Filters live as structured data — JSON config, database rows, a
  drag-and-drop rules UI — not as Sieve text.
- You want first-match, priority-ordered evaluation with actions returned
  as inert values (`PlannedAction`) that your code executes against IMAP,
  JMAP, mbox, or whatever storage you run.
- You want a tiny, sync, panic-free dependency (three crates, no async
  runtime, no parser machinery) that embeds in clients and servers alike —
  bounded regex evaluation means a hostile rule can't stall the host.
- Sieve *syntax* compatibility is explicitly out of scope (see the
  [RFC 5228 note](README.md#relationship-to-rfc-5228-sieve) in the README
  for exactly what maps and what doesn't).

If you later need script compatibility, the `FilterRule` model is a
reasonable compilation target for a Sieve-text → structured-rules
transpiler — but that layer does not exist yet.

### One-line summary

Sieve scripts from users → `mailrs-sieve`. Structured rules you control →
`sieve-kit`.
