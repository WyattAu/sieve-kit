//! `sieve-kit` — rule-based mail filtering engine.
//!
//! Defines filter rules (conditions + actions), evaluates them against
//! messages implementing the [`Filterable`] trait, and produces action plans
//! for a mail engine to execute. This crate is synchronous and I/O-free: all
//! rule evaluation is deterministic and testable without storage.
//!
//! Beyond the RFC 5228 core the typed model also covers the widely used
//! extensions: SMTP `envelope` tests (RFC 5228 §5.1), IMAP flag mutations
//! (RFC 5232), and evaluated-only `vacation` (RFC 5230) and `notify`
//! (RFC 5436) actions — see the README feature table.
//!
//! # Security
//!
//! Regex evaluation is bounded (100 ms post-check). Invalid patterns are
//! treated as non-matching (never panic). Actions are returned as values;
//! executing them is the caller's responsibility. Vacation replies are
//! *evaluated* (routed, deduped) but never sent.
//!
//! # Example
//!
//! ```
//! use sieve_kit::eval::{evaluate_rule, RegexCache};
//! use sieve_kit::types::{
//!     Condition, ConditionField, FilterRule, LogicOp, MailEnvelope, Operator,
//! };
//!
//! let rule = FilterRule {
//!     id: "r1".into(),
//!     name: "Newsletters".into(),
//!     enabled: true,
//!     priority: 0,
//!     conditions: vec![Condition {
//!         field: ConditionField::Subject,
//!         operator: Operator::Contains,
//!         value: "digest".into(),
//!         negate: false,
//!     }],
//!     condition_logic: LogicOp::And,
//!     actions: vec![],
//! };
//! let msg = MailEnvelope {
//!     subject: "Weekly digest".into(),
//!     ..MailEnvelope::default()
//! };
//! assert!(evaluate_rule(&rule, &msg, &RegexCache::default()));
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod actions;
pub mod error;
pub mod eval;
pub mod types;

pub use actions::{
    FilterMatch, PlannedAction, VacationReply, VacationTracker, apply_flag_plan, collect_matches,
};
pub use error::FilterError;
pub use eval::{
    EvalContext, EvalOutcome, EvalWarning, RegexCache, ascii_numeric_eq, evaluate_plan,
    extract_address_part,
};
pub use types::{
    Action, AddressPart, Condition, ConditionField, EnvelopePart, FieldValues, FilterRule,
    Filterable, Flag, KNOWN_NOTIFY_SCHEMES, LogicOp, MailEnvelope, Notify, Operator, Vacation,
};
