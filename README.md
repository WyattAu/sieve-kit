- Action plans as plain values; execution is the host's responsibility
- Rules are `serde`-serializable data; load them from JSON, TOML, or a DB
pipeline; receive → evaluate → dispatch; over an in-memory mailbox with
| `discard` | 5228 | `Action::Delete` (to trash; host decides) |
| has-attachment (sieve-kit-specific) | - | `ConditionField::HasAttachment` |
`VacationTracker`), and returns inert outcome values; the host owns
- `evaluate_rule` / `evaluate_condition`; boolean matching
- `collect_matches`; first-match actions
- `evaluate_plan`; first-match **outcomes** with vacation dedup, notify
- **The RFC 5228 grammar itself**; there is no parser or serializer for
- **Multiple rule firing**; evaluation returns the *first* match's