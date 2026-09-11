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

    /// Envelope sender (SMTP `MAIL FROM`), when the caller supplies envelope
    /// data. Used by the [`envelope`](ConditionField::Envelope) test (RFC 5228
    /// §5.1) and by [`vacation`](Action::Vacation) reply routing.
    ///
    /// Returns `None` when envelope data is unavailable; envelope conditions
    /// then evaluate as non-matching.
    fn envelope_from(&self) -> Option<&str> {
        None
    }

    /// Envelope recipient (SMTP `RCPT TO`), when the caller supplies envelope
    /// data. See [`envelope_from`](Self::envelope_from).
    fn envelope_to(&self) -> Option<&str> {
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
        for action in &self.actions {
            if let Action::Vacation(vacation) = action {
                vacation.validate()?;
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
    /// An SMTP envelope value (RFC 5228 §5.1 `envelope` test).
    ///
    /// Resolved from [`Filterable::envelope_from`] /
    /// [`Filterable::envelope_to`]; when the caller does not supply envelope
    /// data, the test evaluates as non-matching.
    Envelope {
        /// Which envelope value to test.
        part: EnvelopePart,
        /// Which portion of the address to compare.
        address_part: AddressPart,
    },
}

/// Which SMTP envelope value an [`envelope`](ConditionField::Envelope) test
/// reads (RFC 5228 §5.1 string-list `"from"` / `"to"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvelopePart {
    /// Envelope sender (`MAIL FROM`), i.e. the return path.
    From,
    /// Envelope recipient (`RCPT TO`).
    To,
}

/// Which portion of an address an [`envelope`](ConditionField::Envelope) test
/// compares (RFC 5228 §5.1 `:all` / `:localpart` / `:domain`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressPart {
    /// The whole address (default in RFC 5228).
    All,
    /// Everything before the last `@` (the whole value when there is no `@`).
    Localpart,
    /// Everything after the last `@` (empty when there is no `@`).
    Domain,
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
    /// Numeric equality per the `i;ascii-numeric` comparator (RFC 4790):
    /// both sides must be all-ASCII-digit strings (leading zeros ignored,
    /// empty equals only empty); any non-numeric value never matches.
    NumericEquals,
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
    /// Move the message to a folder (`fileinto`).
    MoveTo(String),
    /// Copy the message to a folder (original stays).
    CopyTo(String),
    /// Add flags to the message (RFC 5232 `addflag`).
    Flag(Vec<Flag>),
    /// Remove flags from the message (RFC 5232 `removeflag`).
    Unflag(Vec<Flag>),
    /// Replace the message's flag set (RFC 5232 `setflag`).
    SetFlags(Vec<Flag>),
    /// Mark as read (adds `\Seen`).
    MarkRead,
    /// Delete the message (move to trash, `discard` — host decides).
    Delete,
    /// Forward to an email address (`redirect`).
    Forward(String),
    /// Send an automated reply, at most once per sender per period
    /// (`vacation`, RFC 5230). Evaluation produces a
    /// [`PlannedAction::Vacation`](crate::actions::PlannedAction::Vacation)
    /// outcome; actual SMTP sending is the host's responsibility.
    Vacation(Vacation),
    /// Emit a notification to an external method (`notify`, RFC 5436).
    /// Evaluation produces a
    /// [`PlannedAction::Notify`](crate::actions::PlannedAction::Notify)
    /// outcome; actual delivery is the host's responsibility.
    Notify(Notify),
}

/// Configuration for the [`vacation`](Action::Vacation) action (RFC 5230).
///
/// `days` defaults to 7 (the RFC default for an omitted `:days`). The
/// engine resolves the reply-to sender from envelope data at evaluation
/// time and applies respond-once-per-sender-per-period semantics via the
/// caller-supplied dedup hook (see
/// [`EvalContext`](crate::eval::EvalContext)); the outcome carries enough
/// information (`to`, `days`) for hosts that prefer to dedup themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vacation {
    /// Minimum days between responses to the same sender (`:days`).
    pub days: u32,
    /// Override subject for the reply (`:subject`); when unset the engine
    /// uses `Re: <original subject>`.
    pub subject: Option<String>,
    /// Override From address for the reply (`:from`).
    pub from: Option<String>,
    /// Response body text (the positional `message` argument).
    pub message: String,
}

impl Vacation {
    /// Create a vacation action with the RFC-default 7-day respond period.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            days: 7,
            subject: None,
            from: None,
            message: message.into(),
        }
    }

    /// Set the minimum respond period in days (`:days`).
    #[must_use]
    pub fn with_days(mut self, days: u32) -> Self {
        self.days = days;
        self
    }

    /// Set the reply subject override (`:subject`).
    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the reply From address override (`:from`).
    #[must_use]
    pub fn with_from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// Validate the configuration: non-empty message and `days >= 1`.
    ///
    /// # Errors
    ///
    /// Returns [`FilterError::InvalidVacation`] when the message body is
    /// empty or the period is zero.
    pub fn validate(&self) -> Result<(), FilterError> {
        if self.message.is_empty() {
            return Err(FilterError::InvalidVacation {
                reason: "message must not be empty".to_string(),
            });
        }
        if self.days == 0 {
            return Err(FilterError::InvalidVacation {
                reason: "days must be at least 1".to_string(),
            });
        }
        Ok(())
    }
}

/// Configuration for the [`notify`](Action::Notify) action (RFC 5436).
///
/// `method` is a URI such as `"mailto:ops@example.com"` or
/// `"xmpp:user@host"`. Any string is accepted (construction never fails);
/// evaluation emits an
/// [`EvalWarning::UnknownNotifyMethod`](crate::eval::EvalWarning::UnknownNotifyMethod)
/// when the URI scheme is not one of the recognized notification schemes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notify {
    /// Notification method URI (`:method`).
    pub method: String,
    /// Human-readable message body (`:message`); empty means unset.
    pub message: String,
}

/// URI schemes recognized as notification methods during evaluation.
/// `mailto` is defined by RFC 5436; the others are common extension
/// schemes. Unknown schemes still parse — evaluation just warns.
pub const KNOWN_NOTIFY_SCHEMES: [&str; 6] = ["mailto", "xmpp", "sms", "tel", "http", "https"];

impl Notify {
    /// Create a notification action for the given method URI.
    #[must_use]
    pub fn new(method: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            message: message.into(),
        }
    }

    /// The URI scheme of [`method`](Self::method) (lowercased), i.e. the
    /// portion before the first `:`, or `None` when the method has no
    /// scheme.
    #[must_use]
    pub fn scheme(&self) -> Option<String> {
        let (scheme, _) = self.method.split_once(':')?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme.is_empty() {
            None
        } else {
            Some(scheme)
        }
    }

    /// Whether the method URI uses a scheme in [`KNOWN_NOTIFY_SCHEMES`].
    #[must_use]
    pub fn has_known_scheme(&self) -> bool {
        self.scheme()
            .is_some_and(|s| KNOWN_NOTIFY_SCHEMES.contains(&s.as_str()))
    }
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
    /// Envelope sender (SMTP `MAIL FROM`) for `Envelope` conditions and
    /// vacation reply routing; `None` when the caller has no envelope data.
    pub envelope_from: Option<&'a str>,
    /// Envelope recipient (SMTP `RCPT TO`); `None` when the caller has no
    /// envelope data.
    pub envelope_to: Option<&'a str>,
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

    fn envelope_from(&self) -> Option<&str> {
        self.envelope_from
    }

    fn envelope_to(&self) -> Option<&str> {
        self.envelope_to
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
    /// Envelope sender (SMTP `MAIL FROM`) for `Envelope` conditions and
    /// vacation reply routing; `None` when not supplied.
    pub envelope_from: Option<String>,
    /// Envelope recipient (SMTP `RCPT TO`); `None` when not supplied.
    pub envelope_to: Option<String>,
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

    fn envelope_from(&self) -> Option<&str> {
        self.envelope_from.as_deref()
    }

    fn envelope_to(&self) -> Option<&str> {
        self.envelope_to.as_deref()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn vacation_validate_rejects_empty_message() {
        let vacation = Vacation::new("");
        assert!(matches!(
            vacation.validate(),
            Err(FilterError::InvalidVacation { reason }) if reason.contains("message")
        ));
        assert!(Vacation::new("body").validate().is_ok());
    }

    #[test]
    fn vacation_validate_rejects_zero_days() {
        let vacation = Vacation::new("body").with_days(0);
        assert!(matches!(
            vacation.validate(),
            Err(FilterError::InvalidVacation { reason }) if reason.contains("days")
        ));
        assert!(vacation.with_days(1).validate().is_ok());
    }

    #[test]
    fn vacation_builders_set_fields_and_defaults() {
        let vacation = Vacation::new("away")
            .with_days(3)
            .with_subject("OOO")
            .with_from("me@example.com");
        assert_eq!(vacation.days, 3);
        assert_eq!(vacation.subject.as_deref(), Some("OOO"));
        assert_eq!(vacation.from.as_deref(), Some("me@example.com"));
        assert_eq!(vacation.message, "away");
        // RFC 5230 default when :days is omitted.
        assert_eq!(Vacation::new("x").days, 7);
    }

    #[test]
    fn rule_validate_checks_vacation_actions() {
        let rule = FilterRule {
            id: "r".into(),
            name: "r".into(),
            enabled: true,
            priority: 0,
            conditions: vec![],
            condition_logic: LogicOp::And,
            actions: vec![Action::Vacation(Vacation::new(""))],
        };
        assert!(matches!(
            rule.validate(),
            Err(FilterError::InvalidVacation { .. })
        ));
    }

    #[test]
    fn notify_scheme_extraction() {
        assert_eq!(
            Notify::new("MAILTO:x@y", "").scheme().as_deref(),
            Some("mailto")
        );
        assert_eq!(
            Notify::new("xmpp:user@host", "").scheme().as_deref(),
            Some("xmpp")
        );
        assert_eq!(Notify::new("no-scheme", "").scheme(), None);
        assert_eq!(Notify::new(":empty-scheme", "").scheme(), None);
        assert!(Notify::new("mailto:x@y", "").has_known_scheme());
        assert!(Notify::new("https://hook.example", "").has_known_scheme());
        assert!(!Notify::new("carrier-pigeon:perth", "").has_known_scheme());
    }

    #[test]
    fn filterable_envelope_defaults_to_none() {
        #[allow(dead_code)]
        struct Bare;
        impl Filterable for Bare {
            fn from(&self) -> &str {
                ""
            }
            fn to(&self) -> &str {
                ""
            }
            fn cc(&self) -> &str {
                ""
            }
            fn subject(&self) -> &str {
                ""
            }
            fn body(&self) -> &str {
                ""
            }
            fn has_attachment(&self) -> bool {
                false
            }
        }
        let bare = Bare;
        assert_eq!(bare.envelope_from(), None);
        assert_eq!(bare.envelope_to(), None);
    }
}
