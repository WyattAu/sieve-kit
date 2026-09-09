//! Error type for filter rule construction and validation.

use thiserror::Error as ThisError;

/// Errors produced when constructing or validating filter rules.
#[derive(Debug, ThisError)]
pub enum FilterError {
    /// A `Condition` with [`Operator::Regex`](crate::types::Operator::Regex)
    /// carries a pattern that does not compile.
    #[error("invalid regex pattern {pattern:?}")]
    InvalidRegex {
        /// The offending pattern.
        pattern: String,
        /// The underlying regex parse error.
        #[source]
        source: regex::Error,
    },
    /// The rule's `id` field is empty.
    #[error("rule id must not be empty")]
    EmptyRuleId,
    /// The rule's `name` field is empty.
    #[error("rule name must not be empty")]
    EmptyRuleName,
}
