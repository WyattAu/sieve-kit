//! `sieve-kit` — rule-based mail filtering engine.
//!
//! Defines filter rules (conditions + actions), evaluates them against
//! messages implementing the [`Filterable`] trait, and produces action plans
//! for a mail engine to execute. This crate is synchronous and I/O-free: all
//! rule evaluation is deterministic and testable without storage.
//!
//! # Security
//!
//! Regex evaluation is bounded (100 ms post-check). Invalid patterns are
//! treated as non-matching (never panic). Actions are returned as values;
//! executing them is the caller's responsibility.
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

pub use actions::{FilterMatch, PlannedAction, collect_matches};
pub use error::FilterError;
pub use eval::RegexCache;
pub use types::{
    Action, Condition, ConditionField, FieldValues, FilterRule, Filterable, Flag, LogicOp,
    MailEnvelope, Operator,
};
