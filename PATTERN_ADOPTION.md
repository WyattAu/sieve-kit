# Pattern adoption

Declared per the pattern canon adoption format; every claim is verified against code in this repo.

| Pattern | Status | Evidence |
|---|---|---|
| T13 | adopted | src/lib.rs:5 (I/O-free evaluation; actions returned as values for the caller to execute) + src/types.rs:1 (domain-only rule types) |
| T21 | n/a | stateless rule evaluation; no event history or persistence surface |
