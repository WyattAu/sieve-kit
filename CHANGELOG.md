# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [0.2.0] - 2026-09-11

### Added

- **`vacation` action (RFC 5230)**: `Action::Vacation` (`:days`/`:subject`/
  `:from`/message, RFC-default 7-day period). Evaluation resolves the reply
  recipient from envelope data (falling back to the header From), fills the
  default `Re: <original subject>` subject, applies respond-once-per-
  sender-per-period dedup via a caller-supplied `seen_before` hook, and
  returns a `PlannedAction::Vacation` outcome. Sending stays out of scope.
  Ships with `VacationTracker`, a ready-made in-memory dedup ledger, and
  `FilterError::InvalidVacation` for empty messages / zero-day periods.
- **`notify` action (RFC 5436)**: `Action::Notify` (`:method` URI /
  `:message`) evaluates to a `PlannedAction::Notify` outcome. Unknown URI
  schemes parse fine but surface an `EvalWarning::UnknownNotifyMethod`.
- **IMAP flag actions (RFC 5232)**: `Action::Unflag` (`removeflag`) and
  `Action::SetFlags` (`setflag`) alongside the existing add-flag action,
  with `PlannedAction::{AddFlags, RemoveFlags, SetFlags}` outcomes and the
  pure `apply_flag_plan` folder (add-dedup / remove / replace).
- **`envelope` test (RFC 5228 §5.1)**: `ConditionField::Envelope` with
  `:all`/`:localpart`/`:domain` address parts over the `from`/`to`
  envelope values, supplied by callers via the new
  `Filterable::{envelope_from, envelope_to}` methods (and new `MailEnvelope`
  / `FieldValues` fields). Missing envelope data evaluates as
  non-matching.
- **comparator `i;ascii-numeric` (RFC 4790)**: `Operator::NumericEquals` —
  all-digit operands, leading zeros ignored, empty equals only empty,
  non-numeric never matches.
- **Outcome-level evaluation**: `evaluate_plan` +
  `EvalContext`/`EvalOutcome`/`EvalWarning` — first-match evaluation that
  resolves vacation/notify runtime context and reports non-fatal
  misconfigurations instead of failing.
- `examples/vacation_filter.rs`: away-mode rule set demonstrating vacation
  dedup across messages, notify paging, envelope-based list exclusion, and
  flag mutations.
- `tests/sieve_extensions.rs`: integration suite for all of the above
  (parse → validate → evaluate → outcome assertions).

### Docs

- README: feature table mapping RFC 5228 core + RFC 5230/5232/5436
  extensions to the typed API, with evaluation entry points and an updated
  future-work list.
- `examples/mail_pipeline.rs`: complete receive → evaluate → dispatch
  pipeline over an in-memory mailbox — four rules (newsletters, payments,
  junk regex, attachments from strangers), priority-ordered first-match
  evaluation, and every `PlannedAction` variant executed.
- COMPARISON.md: positioning against `mailrs-sieve` (active RFC 5228 script
  interpreter) and the dead `sieve` crate (a prime-number sieve, unrelated
  to mail), with ecosystem status as of September 2026.

## [0.1.0] - 2026-09-09
