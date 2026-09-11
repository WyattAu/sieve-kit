//! Away-mode filtering: **vacation replies (RFC 5230)**, **notifications
//! (RFC 5436)**, **IMAP flag mutations (RFC 5232)**, and **envelope tests
//! (RFC 5228 §5.1)** in one pass.
//!
//! The engine *evaluates* each rule into outcomes — a
//! [`PlannedAction::Vacation`] carries the resolved recipient, subject and
//! respond period, and a [`PlannedAction::Notify`] carries the method URI —
//! but never sends anything. Delivery and the respond-once ledger live on
//! the host side: here that means an in-memory [`VacationTracker`] and a
//! queue; in production, an SMTP client and a real notification gateway.
//!
//! Run: `cargo run --example vacation_filter`

use sieve_kit::PlannedAction;
use sieve_kit::actions::{VacationTracker, apply_flag_plan};
use sieve_kit::eval::{EvalContext, RegexCache, evaluate_plan, sort_rules_by_priority};
use sieve_kit::types::{
    Action, AddressPart, Condition, ConditionField, EnvelopePart, FilterRule, Flag, LogicOp,
    MailEnvelope, Notify, Operator, Vacation,
};

fn main() {
    // ---- rules ---------------------------------------------------------
    // While away: auto-reply to directly-addressed mail from outside, page
    // on-call for incidents, and park everything else with a flag.
    let mut rules = vec![
        FilterRule {
            id: "incident".into(),
            name: "Page on-call for incidents".into(),
            enabled: true,
            priority: 0,
            conditions: vec![about("incident")],
            condition_logic: LogicOp::And,
            actions: vec![
                Action::Notify(Notify::new(
                    "mailto:oncall@corp.example",
                    "Incident mail arrived while away",
                )),
                Action::Flag(vec![Flag::Flagged]),
            ],
        },
        FilterRule {
            id: "ooo".into(),
            name: "Out-of-office auto-reply".into(),
            enabled: true,
            priority: 10,
            conditions: vec![
                // Envelope test (RFC 5228 §5.1): only mail addressed to me
                // directly (localpart "wyatt"), not to a list I am on.
                Condition {
                    field: ConditionField::Envelope {
                        part: EnvelopePart::To,
                        address_part: AddressPart::Localpart,
                    },
                    operator: Operator::Equals,
                    value: "wyatt".into(),
                    negate: false,
                },
                Condition {
                    field: ConditionField::Envelope {
                        part: EnvelopePart::From,
                        address_part: AddressPart::Domain,
                    },
                    operator: Operator::Contains,
                    value: "corp.example".into(),
                    negate: true, // not from inside the company
                },
                Condition {
                    field: ConditionField::Subject,
                    operator: Operator::Matches,
                    value: "*".into(), // any subject
                    negate: false,
                },
            ],
            condition_logic: LogicOp::And,
            actions: vec![Action::Vacation(
                Vacation::new("I am away until Monday; will reply then.")
                    .with_days(3)
                    .with_subject("Away from mail"),
            )],
        },
        FilterRule {
            id: "park".into(),
            name: "Park everything else".into(),
            enabled: true,
            priority: 20,
            conditions: vec![Condition {
                field: ConditionField::Subject,
                operator: Operator::Matches,
                value: "*".into(),
                negate: false,
            }],
            condition_logic: LogicOp::And,
            actions: vec![Action::SetFlags(vec![Flag::Keyword("away-triage".into())])],
        },
    ];
    sort_rules_by_priority(&mut rules);

    // ---- host-side state -------------------------------------------------
    // The engine decides *whether* a reply is due (respond-once-per-period
    // via the tracker hook); the host decides *how* to send it.
    let tracker = VacationTracker::new();
    let mut outbox: Vec<(String, String)> = Vec::new();
    let regex_cache = RegexCache::default();
    let mut current_flags: Vec<Flag> = Vec::new();

    let messages: [(&str, &str, &str, &str); 5] = [
        (
            "1",
            "alice@personal.example",
            "dinner plans?",
            "wyatt@me.example",
        ),
        (
            "2",
            "alice@personal.example",
            "dinner plans? (resend)",
            "wyatt@me.example",
        ),
        (
            "3",
            "bot@lists.example",
            "weekly incident digest",
            "list@lists.example",
        ),
        (
            "4",
            "pager@vendor.example",
            "SEV incident open",
            "wyatt@me.example",
        ),
        (
            "5",
            "colleague@corp.example",
            "internal FYI",
            "wyatt@me.example",
        ),
    ];

    for (id, from, subject, rcpt_to) in messages {
        let msg = MailEnvelope {
            from: from.into(),
            to: rcpt_to.into(),
            envelope_from: Some(from.into()),
            envelope_to: Some(rcpt_to.into()),
            subject: subject.into(),
            ..MailEnvelope::default()
        };

        println!("{id}: from={from} subject={subject:?}");
        let seen = |sender: &str| tracker.seen_before(sender, 3);
        let ctx = EvalContext::default().with_seen_before(&seen);
        let outcome = evaluate_plan(&rules, &msg, &ctx, &regex_cache);

        for warning in &outcome.warnings {
            println!("   warning: {warning}");
        }
        for effect in &outcome.plan {
            match effect {
                PlannedAction::Vacation(reply) => {
                    println!(
                        "   vacation -> to={} days={} subject={:?}",
                        reply.to, reply.days, reply.subject
                    );
                    // Host-side "sending": queue the evaluated reply, then
                    // record the sender so the period rule holds.
                    outbox.push((reply.to.clone(), reply.subject.clone()));
                    tracker.record(&reply.to);
                }
                PlannedAction::Notify { method, message } => {
                    println!("   notify -> {method}: {message:?}");
                }
                PlannedAction::AddFlags { .. }
                | PlannedAction::RemoveFlags { .. }
                | PlannedAction::SetFlags { .. } => {
                    // RFC 5232 semantics: add (dedup) / remove / replace.
                    current_flags = apply_flag_plan(&current_flags, std::slice::from_ref(effect));
                    println!("   flags -> {current_flags:?}");
                }
                other => println!("   {other:?}"),
            }
        }
        if outcome.plan.is_empty() {
            if let Some(rule_id) = &outcome.rule_id {
                println!(
                    "   matched rule {rule_id}: no outcomes (reply already sent within period)"
                );
            } else {
                println!("   no rule matched -> implicit keep");
            }
        }
    }

    println!("\noutbox (evaluated replies a real host would send):");
    for (to, subject) in &outbox {
        println!("  to {to}: {subject:?}");
    }
    println!("alice was answered once, not twice: dedup across evaluations.");
}

/// Subject-contains condition.
fn about(needle: &str) -> Condition {
    Condition {
        field: ConditionField::Subject,
        operator: Operator::Contains,
        value: needle.into(),
        negate: false,
    }
}
