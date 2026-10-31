| What it is | typed rule engine (conditions → action plan) | RFC 5228 Sieve **script** interpreter | **prime-number sieve**; not mail |
| Input format | `FilterRule` structs (serde: JSON/TOML/DB rows) | Sieve script text (RFC 5228 grammar) |; |
| Semantics | first-match-wins, priority-ordered | full RFC 5228 action stream (Keep, FileInto, Discard, Redirect, Reject, Vacation) |; |
| Tests/operators | 9 fields incl. `envelope`, 6 operators incl. bounded regex + `i;ascii-numeric` | full RFC 5228 + extensions |; |
| Extensions | `vacation` (RFC 5230, evaluated + dedup hook), `notify` (RFC 5436), IMAP flags (RFC 5232) | engine-dependent |; |
| Dependencies | `regex`, `serde`, `thiserror` | engine + parser stack (wraps a full Sieve engine) |; |
| Execution model | pure, sync, I/O-free; host executes the plan | delivery-loop integration |; |
| `unsafe` | forbidden (`#![forbid(unsafe_code)]`) |; |; |
the RFC 5228 space effectively has no crate under it; and it is not a
- Filters live as structured data; JSON config, database rows, a
  drag-and-drop rules UI; not as Sieve text.
  runtime, no parser machinery) that embeds in clients and servers alike -
transpiler; but that layer does not exist yet.