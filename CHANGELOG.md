# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

### Added

- `examples/mail_pipeline.rs`: complete receive → evaluate → dispatch
  pipeline over an in-memory mailbox — four rules (newsletters, payments,
  junk regex, attachments from strangers), priority-ordered first-match
  evaluation, and every `PlannedAction` variant executed.
- README: relationship-to-RFC-5228 note mapping each implemented piece to
  its Sieve concept (tests, comparators, `allof`/`anyof`/`not`,
  `fileinto`/`imap4flags`/`discard`/`redirect`) with an explicit
  future-work list (script grammar, multi-rule firing, extensions).
- COMPARISON.md: positioning against `mailrs-sieve` (active RFC 5228 script
  interpreter) and the dead `sieve` crate (a prime-number sieve, unrelated
  to mail), with ecosystem status as of September 2026.

## [0.1.0] - 2026-09-09
