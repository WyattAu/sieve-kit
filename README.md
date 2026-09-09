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

Extracted from the [kestrel](https://github.com/WyattAu/kestrel) mail client.

## License

MIT OR Apache-2.0
