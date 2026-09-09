//! Filter rule types: conditions, operators, actions, and the composite rule
//! structure. These types are domain-only (no storage/async dependencies) so
//! they can be evaluated synchronously against any [`Filterable`] message.

use serde::{Deserialize, Serialize};

use crate::error::FilterError;

/// The fields a filter engine needs from a message.
///
/// Implement this trait for your message type (or use the provided
/// [`MailEnvelope`] struct) to evaluate rules against it.
pub trait Filterable {
    /// First From address email (empty if absent).
    fn from(&self) -> &str;

    /// First To address email (empty if absent).
    fn to(&self) -> &str;

    /// First Cc address email (empty if absent).
    fn cc(&self) -> &str;

    /// Subject header (empty if absent).
    fn subject(&self) -> &str;

    /// Body text (empty if absent).
    fn body(&self) -> &str;

    /// Whether the message has attachments.
    fn has_attachment(&self) -> bool;

    /// Value of a raw header by name (case-insensitive lookup), if present.
    fn header(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }
}

/// A complete filter rule: conditions + actions + metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilterRule {
    /// Stable identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the rule is active.
    pub enabled: bool,
    /// Lower = evaluated first.
    pub priority: i32,
    /// Conditions to evaluate.
    pub conditions: Vec<Condition>,
    /// How conditions are combined.
    pub condition_logic: LogicOp,
    /// Actions to execute when the rule matches.
    pub actions: Vec<Action>,
}

impl FilterRule {
    /// Validate the rule: non-empty `id` and `name`, and every
    /// [`Operator::Regex`] condition must compile.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError`] on an empty id/name or an invalid regex
    /// pattern.
    pub fn validate(&self) -> Result<(), FilterError> {
        if self.id.is_empty() {
            return Err(FilterError::EmptyRuleId);
        }
        if self.name.is_empty() {
            return Err(FilterError::EmptyRuleName);
        }
        for condition in &self.conditions {
            if condition.operator == Operator::Regex {
                regex::Regex::new(&condition.value).map_err(|source| {
                    FilterError::InvalidRegex {
                        pattern: condition.value.clone(),
                        source,
                    }
                })?;
            }
        }
        Ok(())
    }
}

/// Boolean combinator for conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicOp {
    /// All conditions must match.
    And,
    /// Any condition may match.
    Or,
}

/// A single condition clause.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Condition {
    /// Which message field to test.
    pub field: ConditionField,
    /// How to test it.
    pub operator: Operator,
    /// The comparison value (interpreted per operator).
    pub value: String,
    /// When `true`, invert the result.
    pub negate: bool,
}

/// Message fields that conditions can target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionField {
    /// First From address email.
    From,
    /// First To address email.
    To,
    /// Any Cc address email.
    Cc,
    /// Subject header.
    Subject,
    /// Message body text.
    Body,
    /// A specific header by name.
    Header(String),
    /// Whether the message has attachments.
    HasAttachment,
}

/// Comparison operators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    /// Substring match (case-insensitive).
    Contains,
    /// Exact match (case-insensitive).
    Equals,
    /// Glob-style match (`*` and `?`).
    Matches,
    /// Regular expression (bounded: 100 ms post-check, complexity limit).
    Regex,
    /// Field exists and is non-empty.
    Exists,
}

/// An IMAP message flag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    /// `\Seen` — message has been read.
    Seen,
    /// `\Answered` — message has been replied to.
    Answered,
    /// `\Flagged` — message is flagged/starred.
    Flagged,
    /// `\Deleted` — message is marked for deletion.
    Deleted,
    /// `\Draft` — message is a draft.
    Draft,
    /// A non-system keyword flag.
    Keyword(String),
}

/// An action to perform when a rule matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Move the message to a folder.
    MoveTo(String),
    /// Copy the message to a folder (original stays).
    CopyTo(String),
    /// Apply flags.
    Flag(Vec<Flag>),
    /// Mark as read (adds `\Seen`).
    MarkRead,
    /// Delete the message (move to trash).
    Delete,
    /// Forward to an email address.
    Forward(String),
}

/// Extracted field values from a message for condition evaluation.
///
/// A concrete [`Filterable`]: build one from your message, or evaluate
/// directly against your own [`Filterable`] implementation.
#[derive(Clone, Debug, Default)]
pub struct FieldValues<'a> {
    /// From address email (first).
    pub from: &'a str,
    /// To address email (first).
    pub to: &'a str,
    /// Cc address email (first, if any).
    pub cc: &'a str,
    /// Subject.
    pub subject: &'a str,
    /// Body text.
    pub body: &'a str,
    /// Whether the message has attachments.
    pub has_attachment: bool,
    /// Raw headers (name -> value) for `Header` conditions.
    pub headers: Vec<(&'a str, &'a str)>,
}

impl Filterable for FieldValues<'_> {
    fn from(&self) -> &str {
        self.from
    }

    fn to(&self) -> &str {
        self.to
    }

    fn cc(&self) -> &str {
        self.cc
    }

    fn subject(&self) -> &str {
        self.subject
    }

    fn body(&self) -> &str {
        self.body
    }

    fn has_attachment(&self) -> bool {
        self.has_attachment
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }
}

/// A simple envelope of the fields a filter engine needs.
///
/// A ready-made [`Filterable`] implementation for callers that do not want to
/// implement the trait for their own message type.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MailEnvelope {
    /// First From address email.
    pub from: String,
    /// First To address email.
    pub to: String,
    /// First Cc address email.
    pub cc: String,
    /// Subject header.
    pub subject: String,
    /// Body text.
    pub body: String,
    /// Whether the message has attachments.
    pub has_attachment: bool,
    /// Raw headers (name -> value) for `Header` conditions.
    pub headers: Vec<(String, String)>,
}

impl Filterable for MailEnvelope {
    fn from(&self) -> &str {
        &self.from
    }

    fn to(&self) -> &str {
        &self.to
    }

    fn cc(&self) -> &str {
        &self.cc
    }

    fn subject(&self) -> &str {
        &self.subject
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn has_attachment(&self) -> bool {
        self.has_attachment
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}
