//! Action definitions and execution helpers. Actions are value types that
//! describe what to do; the host application translates them into its own
//! mail operations. Evaluation and planning here are pure: no I/O.

use crate::eval::{RegexCache, evaluate_rule};
use crate::types::{Action, FilterRule, Filterable};

/// The result of evaluating a rule: which message and what actions to apply.
#[derive(Clone, Debug)]
pub struct FilterMatch {
    /// The matched message identifier.
    pub message_id: String,
    /// Actions to execute, in rule order.
    pub actions: Vec<Action>,
}

impl FilterMatch {
    /// Create a match for the given message id.
    #[must_use]
    pub fn new(message_id: impl Into<String>, actions: Vec<Action>) -> Self {
        Self {
            message_id: message_id.into(),
            actions,
        }
    }
}

/// A single planned action ready for execution by the host application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlannedAction {
    /// Move to folder.
    Move {
        /// Destination folder.
        to: String,
    },
    /// Copy to folder.
    Copy {
        /// Destination folder.
        to: String,
    },
    /// Apply flags.
    AddFlags {
        /// Flags to add.
        flags: Vec<crate::types::Flag>,
    },
    /// Mark as read.
    MarkRead,
    /// Delete (move to trash).
    Delete,
    /// Forward to address.
    Forward {
        /// Recipient email.
        to: String,
    },
}

impl From<&Action> for PlannedAction {
    fn from(action: &Action) -> Self {
        match action {
            Action::MoveTo(to) => Self::Move { to: to.clone() },
            Action::CopyTo(to) => Self::Copy { to: to.clone() },
            Action::Flag(flags) => Self::AddFlags {
                flags: flags.clone(),
            },
            Action::MarkRead => Self::MarkRead,
            Action::Delete => Self::Delete,
            Action::Forward(addr) => Self::Forward { to: addr.clone() },
        }
    }
}

/// Translate a list of actions into a plan that the host can execute.
///
/// This is a pure function: it collects the actions without performing I/O.
#[must_use]
pub fn build_action_plan(actions: &[Action]) -> Vec<PlannedAction> {
    actions.iter().map(PlannedAction::from).collect()
}

/// Collect the actions from the first rule matching a message (in the given
/// rule order — use [`sort_rules_by_priority`](crate::eval::sort_rules_by_priority)
/// to order by priority first).
///
/// If no rule matches, returns an empty vec.
#[must_use]
pub fn collect_matches<F: Filterable + ?Sized>(
    rules: &[FilterRule],
    msg: &F,
    regex_cache: &RegexCache,
) -> Vec<Action> {
    for rule in rules {
        if evaluate_rule(rule, msg, regex_cache) {
            return rule.actions.clone();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::types::{Condition, ConditionField, LogicOp, MailEnvelope, Operator};

    fn test_rule(actions: Vec<Action>) -> FilterRule {
        FilterRule {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            priority: 0,
            conditions: vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Contains,
                value: "hello".to_string(),
                negate: false,
            }],
            condition_logic: LogicOp::And,
            actions,
        }
    }

    fn make_envelope() -> MailEnvelope {
        MailEnvelope {
            subject: "Hello World".to_string(),
            ..MailEnvelope::default()
        }
    }

    #[test]
    fn collect_matches_returns_first_rule_actions() {
        let rules = vec![
            test_rule(vec![Action::MarkRead]),
            test_rule(vec![Action::Delete]),
        ];
        let msg = make_envelope();
        let cache = RegexCache::default();
        let matches = collect_matches(&rules, &msg, &cache);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], Action::MarkRead);
    }

    #[test]
    fn collect_matches_returns_empty_when_no_match() {
        let rules = vec![test_rule(vec![Action::MarkRead])];
        let msg = MailEnvelope {
            subject: "no match here".to_string(),
            ..MailEnvelope::default()
        };
        let cache = RegexCache::default();
        let matches = collect_matches(&rules, &msg, &cache);
        assert!(matches.is_empty());
    }

    #[test]
    fn build_action_plan_translates_all_variants() {
        let actions = vec![
            Action::MoveTo("Archive".to_string()),
            Action::CopyTo("Keep".to_string()),
            Action::Flag(vec![crate::types::Flag::Flagged]),
            Action::MarkRead,
            Action::Delete,
            Action::Forward("a@b.com".to_string()),
        ];
        let plan = build_action_plan(&actions);
        assert_eq!(plan.len(), 6);
        assert_eq!(
            plan[0],
            PlannedAction::Move {
                to: "Archive".to_string()
            }
        );
        assert_eq!(
            plan[1],
            PlannedAction::Copy {
                to: "Keep".to_string()
            }
        );
        assert_eq!(
            plan[2],
            PlannedAction::AddFlags {
                flags: vec![crate::types::Flag::Flagged]
            }
        );
        assert_eq!(plan[3], PlannedAction::MarkRead);
        assert_eq!(plan[4], PlannedAction::Delete);
        assert_eq!(
            plan[5],
            PlannedAction::Forward {
                to: "a@b.com".to_string()
            }
        );
    }

    #[test]
    fn filter_match_new_accepts_any_id() {
        let m = FilterMatch::new("msg-42", vec![Action::MarkRead]);
        assert_eq!(m.message_id, "msg-42");
        assert_eq!(m.actions, vec![Action::MarkRead]);
    }
}
