//! A complete filtering pipeline: **receive → evaluate → dispatch**.
//!
//! 1. *Receive* — raw mail turns into a [`MailEnvelope`], the `Filterable`
//!    input the engine understands.
//! 2. *Evaluate* — rules are validated, sorted by priority, and the first
//!    matching rule's actions are collected (first-match wins; there is no
//!    fall-through).
//! 3. *Dispatch* — the resulting action plan is executed against a toy
//!    in-memory mailbox. In a real system this is where *your* code maps
//!    `PlannedAction` values onto IMAP/JMAP/`mbox` operations.
//!
//! Run: `cargo run --example mail_pipeline`

use std::collections::{BTreeMap, HashSet};

use sieve_kit::eval::{RegexCache, sort_rules_by_priority};
use sieve_kit::types::{
    Action, Condition, ConditionField, FilterRule, Flag, LogicOp, MailEnvelope, Operator,
};
use sieve_kit::{PlannedAction, collect_matches};

fn main() {
    let mut rules = build_rules();
    if let Err(err) = rules.iter().try_for_each(FilterRule::validate) {
        eprintln!("rule set invalid: {err}");
        std::process::exit(1);
    }
    sort_rules_by_priority(&mut rules);

    let messages = incoming();
    let mut mailbox = Mailbox::default();
    let regex_cache = RegexCache::default();

    for (id, message) in &messages {
        // ---- receive ---------------------------------------------------
        mailbox.deliver(id);
        println!(
            "received {id}: from={} subject={:?}",
            message.from, message.subject
        );

        // ---- evaluate --------------------------------------------------
        let actions = collect_matches(&rules, message, &regex_cache);
        let Some(rule) = rules
            .iter()
            .find(|rule| rule.actions == actions && !actions.is_empty())
        else {
            println!("  no rule matched -> implicit keep");
            continue;
        };
        println!(
            "  matched rule {:?} (priority {})",
            rule.name, rule.priority
        );

        // ---- dispatch --------------------------------------------------
        let plan: Vec<PlannedAction> = actions.iter().map(Into::into).collect();
        for effect in mailbox.execute(id, &plan) {
            println!("  -> {effect}");
        }
    }

    println!("\nfinal mailbox state:");
    mailbox.report();
}

/// The rule set, expressed as data. `FilterRule` is `serde`-serializable,
/// so in production these would typically be loaded from config or a DB.
fn build_rules() -> Vec<FilterRule> {
    let newsletters = FilterRule {
        id: "r1".into(),
        name: "Newsletters".into(),
        enabled: true,
        priority: 0,
        conditions: vec![
            Condition {
                field: ConditionField::From,
                operator: Operator::Contains,
                value: "newsletter".into(),
                negate: false,
            },
            Condition {
                field: ConditionField::Header("List-Id".into()),
                operator: Operator::Exists,
                value: String::new(),
                negate: false,
            },
        ],
        condition_logic: LogicOp::Or,
        actions: vec![Action::MoveTo("Newsletters".into()), Action::MarkRead],
    };

    let payments = FilterRule {
        id: "r2".into(),
        name: "Payments".into(),
        enabled: true,
        priority: 10,
        conditions: vec![Condition {
            field: ConditionField::From,
            operator: Operator::Matches,
            value: "*@payments.example.com".into(),
            negate: false,
        }],
        condition_logic: LogicOp::And,
        actions: vec![
            Action::CopyTo("Receipts".into()),
            Action::Flag(vec![Flag::Flagged]),
        ],
    };

    let junk = FilterRule {
        id: "r3".into(),
        name: "Junk".into(),
        enabled: true,
        priority: 20,
        conditions: vec![Condition {
            field: ConditionField::Subject,
            operator: Operator::Regex,
            value: r"(?i)(lottery|prize|winner)".into(),
            negate: false,
        }],
        condition_logic: LogicOp::And,
        actions: vec![Action::Delete],
    };

    let attachments = FilterRule {
        id: "r4".into(),
        name: "Attachments from strangers".into(),
        enabled: true,
        priority: 30,
        conditions: vec![
            Condition {
                field: ConditionField::HasAttachment,
                operator: Operator::Equals,
                value: "true".into(),
                negate: false,
            },
            Condition {
                field: ConditionField::From,
                operator: Operator::Contains,
                value: "work.example".into(),
                negate: true,
            },
        ],
        condition_logic: LogicOp::And,
        actions: vec![Action::Forward("archive@example.com".into())],
    };

    vec![newsletters, payments, junk, attachments]
}

fn incoming() -> Vec<(String, MailEnvelope)> {
    vec![
        (
            "msg-1".to_string(),
            MailEnvelope {
                from: "newsletter@lists.example.com".into(),
                to: "you@example.com".into(),
                subject: "Weekly digest".into(),
                headers: vec![("List-Id".into(), "<digest.lists.example.com>".into())],
                ..MailEnvelope::default()
            },
        ),
        (
            "msg-2".to_string(),
            MailEnvelope {
                from: "billing@payments.example.com".into(),
                to: "you@example.com".into(),
                subject: "Invoice #4112".into(),
                ..MailEnvelope::default()
            },
        ),
        (
            "msg-3".to_string(),
            MailEnvelope {
                from: "winner@sketchy.example".into(),
                to: "you@example.com".into(),
                subject: "You are our LOTTERY prize winner!!!".into(),
                ..MailEnvelope::default()
            },
        ),
        (
            "msg-4".to_string(),
            MailEnvelope {
                from: "colleague@work.example".into(),
                to: "you@example.com".into(),
                subject: "Lunch?".into(),
                ..MailEnvelope::default()
            },
        ),
        (
            "msg-5".to_string(),
            MailEnvelope {
                from: "stranger@random.example".into(),
                to: "you@example.com".into(),
                subject: "Document as requested".into(),
                has_attachment: true,
                ..MailEnvelope::default()
            },
        ),
    ]
}

/// A toy mailbox the plan is dispatched onto. Every `PlannedAction` maps to
/// exactly one mailbox mutation — the engine never performs I/O itself.
#[derive(Default)]
struct Mailbox {
    inbox: Vec<String>,
    folders: BTreeMap<String, Vec<String>>,
    trash: Vec<String>,
    read: HashSet<String>,
    flags: BTreeMap<String, Vec<Flag>>,
    forwarded: Vec<(String, String)>,
    replies: Vec<(String, String)>,
    notifications: Vec<(String, String)>,
}

impl Mailbox {
    fn deliver(&mut self, id: &str) {
        self.inbox.push(id.to_string());
    }

    fn execute(&mut self, id: &str, plan: &[PlannedAction]) -> Vec<String> {
        let mut effects = Vec::new();
        for action in plan {
            match action {
                PlannedAction::Move { to } => {
                    self.inbox.retain(|m| m != id);
                    self.folders
                        .entry(to.clone())
                        .or_default()
                        .push(id.to_string());
                    effects.push(format!("filed into {to}"));
                }
                PlannedAction::Copy { to } => {
                    self.folders
                        .entry(to.clone())
                        .or_default()
                        .push(id.to_string());
                    effects.push(format!("copied to {to}"));
                }
                PlannedAction::AddFlags { flags } => {
                    self.flags
                        .entry(id.to_string())
                        .or_default()
                        .extend(flags.iter().cloned());
                    effects.push(format!("flags {flags:?}"));
                }
                PlannedAction::RemoveFlags { flags } => {
                    if let Some(current) = self.flags.get_mut(id) {
                        current.retain(|f| !flags.contains(f));
                    }
                    effects.push(format!("unflags {flags:?}"));
                }
                PlannedAction::SetFlags { flags } => {
                    self.flags.insert(id.to_string(), flags.clone());
                    effects.push(format!("flags set to {flags:?}"));
                }
                PlannedAction::MarkRead => {
                    self.read.insert(id.to_string());
                    effects.push("marked read".into());
                }
                PlannedAction::Delete => {
                    self.inbox.retain(|m| m != id);
                    self.trash.push(id.to_string());
                    effects.push("moved to trash".into());
                }
                PlannedAction::Forward { to } => {
                    self.forwarded.push((id.to_string(), to.clone()));
                    effects.push(format!("forwarded to {to}"));
                }
                PlannedAction::Vacation(reply) => {
                    self.replies.push((reply.to.clone(), reply.subject.clone()));
                    effects.push(format!(
                        "vacation reply queued to {} (once per {} days)",
                        reply.to, reply.days
                    ));
                }
                PlannedAction::Notify { method, message } => {
                    self.notifications.push((method.clone(), message.clone()));
                    effects.push(format!("notify via {method}: {message}"));
                }
            }
        }
        effects
    }

    fn report(&self) {
        println!("  inbox: {:?}", self.inbox);
        for (folder, ids) in &self.folders {
            println!("  {folder}: {ids:?}");
        }
        println!("  trash: {:?}", self.trash);
        println!("  read:  {:?}", self.read);
        for (id, flags) in &self.flags {
            println!("  flags on {id}: {flags:?}");
        }
        for (id, to) in &self.forwarded {
            println!("  forwarded {id} -> {to}");
        }
        for (to, subject) in &self.replies {
            println!("  vacation reply to {to}: {subject:?}");
        }
        for (method, message) in &self.notifications {
            println!("  notification {method}: {message:?}");
        }
    }
}
