//! Action definitions and execution helpers. Actions are value types that
//! describe what to do; the host application translates them into its own
//! mail operations. Evaluation and planning here are pure: no I/O.

use crate::eval::{RegexCache, evaluate_rule};
use crate::types::{Action, FilterRule, Filterable, Flag};

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
    /// Add flags (RFC 5232 `addflag`).
    AddFlags {
        /// Flags to add.
        flags: Vec<crate::types::Flag>,
    },
    /// Remove flags (RFC 5232 `removeflag`).
    RemoveFlags {
        /// Flags to remove.
        flags: Vec<crate::types::Flag>,
    },
    /// Replace the flag set (RFC 5232 `setflag`).
    SetFlags {
        /// The new flag set.
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
    /// Send an automated reply (RFC 5230 `vacation`). The engine *evaluates*
    /// the reply — dedup, routing, default subject — but never sends: handing
    /// it to an SMTP transport is the host's responsibility.
    Vacation(VacationReply),
    /// Emit a notification to an external method (RFC 5436 `notify`).
    /// Delivery is the host's responsibility.
    Notify {
        /// Notification method URI (e.g. `mailto:ops@example.com`).
        method: String,
        /// Message body (`:message`).
        message: String,
    },
}

/// An evaluated automated reply (RFC 5230 `vacation`), ready for the host's
/// SMTP layer. The engine computes routing and defaults; the host sends it
/// and tracks the respond period (see [`VacationTracker`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VacationReply {
    /// Address to reply to: the envelope sender, falling back to the header
    /// From address. Unfilled (empty) when the outcome was built by
    /// [`build_action_plan`] instead of
    /// [`evaluate_plan`](crate::eval::evaluate_plan).
    pub to: String,
    /// Minimum days before another reply to the same sender (`:days`).
    pub days: u32,
    /// Resolved subject: the configured `:subject`, else
    /// `Re: <original subject>`, else `Automated reply`.
    pub subject: String,
    /// Configured `:from` override, if any.
    pub from: Option<String>,
    /// Response body text.
    pub message: String,
}

impl From<&Action> for PlannedAction {
    fn from(action: &Action) -> Self {
        match action {
            Action::MoveTo(to) => Self::Move { to: to.clone() },
            Action::CopyTo(to) => Self::Copy { to: to.clone() },
            Action::Flag(flags) => Self::AddFlags {
                flags: flags.clone(),
            },
            Action::Unflag(flags) => Self::RemoveFlags {
                flags: flags.clone(),
            },
            Action::SetFlags(flags) => Self::SetFlags {
                flags: flags.clone(),
            },
            Action::MarkRead => Self::MarkRead,
            Action::Delete => Self::Delete,
            Action::Forward(addr) => Self::Forward { to: addr.clone() },
            Action::Vacation(vacation) => Self::Vacation(VacationReply {
                // Recipient and default subject are runtime-resolved by
                // `evaluate_plan`; a pure translation leaves them unfilled.
                to: String::new(),
                days: vacation.days,
                subject: vacation.subject.clone().unwrap_or_default(),
                from: vacation.from.clone(),
                message: vacation.message.clone(),
            }),
            Action::Notify(notify) => Self::Notify {
                method: notify.method.clone(),
                message: notify.message.clone(),
            },
        }
    }
}

/// Fold flag mutations (RFC 5232 `addflag`/`removeflag`/`setflag`) onto a
/// current flag set, in plan order. `AddFlags` appends without duplicates,
/// `RemoveFlags` drops every listed flag, `SetFlags` replaces the set.
/// [`MarkRead`](PlannedAction::MarkRead) is a host-level concern and is
/// ignored here. Keywords compare exactly (hosts may normalize case).
#[must_use]
pub fn apply_flag_plan(current: &[Flag], plan: &[PlannedAction]) -> Vec<Flag> {
    let mut flags: Vec<Flag> = current.to_vec();
    for action in plan {
        match action {
            PlannedAction::AddFlags { flags: add } => {
                for flag in add {
                    if !flags.contains(flag) {
                        flags.push(flag.clone());
                    }
                }
            }
            PlannedAction::RemoveFlags { flags: remove } => {
                flags.retain(|f| !remove.contains(f));
            }
            PlannedAction::SetFlags { flags: set } => {
                flags = set.clone();
            }
            _ => {}
        }
    }
    flags
}

/// In-memory ledger for vacation respond-once-per-sender-per-period
/// semantics (RFC 5230). Record a reply when one is sent; query with
/// [`seen_before`](Self::seen_before) to decide whether another is due.
/// Thread-safe; usable directly as an
/// [`EvalContext::seen_before`](crate::eval::EvalContext) predicate.
///
/// ```
/// use sieve_kit::actions::VacationTracker;
///
/// let tracker = VacationTracker::new();
/// assert!(!tracker.seen_before("alice@example.com", 7));
/// tracker.record("alice@example.com");
/// assert!(tracker.seen_before("alice@example.com", 7));
/// assert!(!tracker.seen_before("bob@example.com", 7));
/// ```
#[derive(Default)]
pub struct VacationTracker {
    entries: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl VacationTracker {
    /// Create an empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a reply was recorded for `sender` less than `days` days ago.
    /// Poisoned state is treated as "nothing recorded" (never panics).
    #[must_use]
    pub fn seen_before(&self, sender: &str, days: u32) -> bool {
        let Ok(entries) = self.entries.lock() else {
            return false;
        };
        entries.get(sender).is_some_and(|recorded| {
            recorded.elapsed() < std::time::Duration::from_secs(u64::from(days) * 86_400)
        })
    }

    /// Record that a reply was sent to `sender`, effective now.
    pub fn record(&self, sender: &str) {
        self.record_at(sender, std::time::Instant::now());
    }

    /// Record a reply with an explicit timestamp — the injection point for
    /// tests and clock-control.
    pub fn record_at(&self, sender: &str, at: std::time::Instant) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(sender.to_string(), at);
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

    #[test]
    fn build_action_plan_translates_flag_mutation_variants() {
        let actions = vec![
            Action::Flag(vec![Flag::Flagged]),
            Action::Unflag(vec![Flag::Seen]),
            Action::SetFlags(vec![Flag::Answered]),
        ];
        let plan = build_action_plan(&actions);
        assert_eq!(
            plan,
            vec![
                PlannedAction::AddFlags {
                    flags: vec![Flag::Flagged]
                },
                PlannedAction::RemoveFlags {
                    flags: vec![Flag::Seen]
                },
                PlannedAction::SetFlags {
                    flags: vec![Flag::Answered]
                },
            ]
        );
    }

    #[test]
    fn build_action_plan_translates_vacation_and_notify() {
        let plan = build_action_plan(&[
            Action::Vacation(
                crate::types::Vacation::new("away")
                    .with_days(2)
                    .with_from("me@example.com"),
            ),
            Action::Notify(crate::types::Notify::new("mailto:x@y", "ping")),
        ]);
        assert_eq!(
            plan,
            vec![
                PlannedAction::Vacation(VacationReply {
                    // Pure translation leaves the recipient unfilled;
                    // `evaluate_plan` resolves it from envelope data.
                    to: String::new(),
                    days: 2,
                    subject: String::new(),
                    from: Some("me@example.com".to_string()),
                    message: "away".to_string(),
                }),
                PlannedAction::Notify {
                    method: "mailto:x@y".to_string(),
                    message: "ping".to_string(),
                },
            ]
        );
    }

    #[test]
    fn apply_flag_plan_adds_without_duplicates() {
        let current = vec![Flag::Seen];
        let plan = build_action_plan(&[Action::Flag(vec![Flag::Seen, Flag::Flagged])]);
        let flags = apply_flag_plan(&current, &plan);
        assert_eq!(flags, vec![Flag::Seen, Flag::Flagged]);
    }

    #[test]
    fn apply_flag_plan_removes_listed_flags_only() {
        let current = vec![Flag::Seen, Flag::Flagged, Flag::Keyword("work".into())];
        let plan = build_action_plan(&[Action::Unflag(vec![
            Flag::Seen,
            Flag::Keyword("nope".into()),
        ])]);
        let flags = apply_flag_plan(&current, &plan);
        assert_eq!(flags, vec![Flag::Flagged, Flag::Keyword("work".into())]);
    }

    #[test]
    fn apply_flag_plan_set_replaces_whole_set() {
        let current = vec![Flag::Seen, Flag::Flagged];
        let plan = build_action_plan(&[Action::SetFlags(vec![Flag::Draft])]);
        let flags = apply_flag_plan(&current, &plan);
        assert_eq!(flags, vec![Flag::Draft]);
    }

    #[test]
    fn apply_flag_plan_folds_in_order() {
        let current = vec![];
        let plan = build_action_plan(&[
            Action::Flag(vec![Flag::Flagged]),
            Action::Flag(vec![Flag::Seen]),
            Action::Unflag(vec![Flag::Flagged]),
            Action::SetFlags(vec![Flag::Answered, Flag::Draft]),
        ]);
        let flags = apply_flag_plan(&current, &plan);
        assert_eq!(flags, vec![Flag::Answered, Flag::Draft]);
    }

    #[test]
    fn apply_flag_plan_ignores_non_flag_actions() {
        let current = vec![Flag::Seen];
        let plan = build_action_plan(&[Action::MoveTo("Archive".into()), Action::MarkRead]);
        let flags = apply_flag_plan(&current, &plan);
        assert_eq!(flags, vec![Flag::Seen]);
    }

    #[test]
    fn vacation_tracker_responds_once_within_period() {
        let tracker = VacationTracker::new();
        assert!(!tracker.seen_before("a@b.c", 7));
        tracker.record("a@b.c");
        assert!(tracker.seen_before("a@b.c", 7));
        assert!(!tracker.seen_before("other@b.c", 7));
        // Period boundary: 7 days exactly is due again, 6 days is not.
        let now = std::time::Instant::now();
        tracker.record_at("old@b.c", now - std::time::Duration::from_secs(7 * 86_400));
        assert!(!tracker.seen_before("old@b.c", 7));
        tracker.record_at(
            "recent@b.c",
            now - std::time::Duration::from_secs(6 * 86_400),
        );
        assert!(tracker.seen_before("recent@b.c", 7));
    }
}
