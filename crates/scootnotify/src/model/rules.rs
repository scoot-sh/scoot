//! `[[rule]]` matching and applying: which rules fire, and what they
//! steer.
//!
//! A rule fires when every match key it names holds: `app_name` and
//! `desktop_entry` exactly, `category` exactly or by a `class.` prefix
//! (a rule `category` ending in a dot), `urgency` exactly, and
//! `summary_contains` as a plain substring of the summary. A rule with
//! no match key matches everything. Every matching rule applies, and
//! for the scalar actions the later rule wins; every matching `exec`
//! runs, bounded by the spawner's four running children. `exec` never
//! fires for a `replaces_id` update (a volume OSD holding one card must
//! not ring the hook on every tick) — only for fresh arrivals.

use crate::bus::hints::Urgency;
use crate::config::{Rule, RuleUrgency};

/// What the matching rules steer for one fresh arrival.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The card's urgency, over the hint's.
    pub urgency: Option<Urgency>,
    /// The card's expiry, over the resolved one.
    pub timeout: Option<Timeout>,
    /// Straight to history, never on screen.
    pub hide: bool,
    /// False keeps the card out of history (held and closed alike).
    pub keep_history: bool,
    /// This card's output, over the file's.
    pub output: Option<String>,
    /// Shows through DND, like a critical card does by default.
    pub dnd_bypass: bool,
    /// Every matching `exec`, in rule order.
    pub execs: Vec<Vec<String>>,
}

impl Default for Decision {
    fn default() -> Self {
        Self {
            urgency: None,
            timeout: None,
            hide: false,
            keep_history: true,
            output: None,
            dnd_bypass: false,
            execs: Vec::new(),
        }
    }
}

/// A rule's expiry override: never, or this many milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    Never,
    Ms(u32),
}

/// Whether `rule` fires for a card with these fields.
pub fn matches(
    rule: &Rule,
    app_name: &str,
    desktop_entry: &str,
    category: &str,
    urgency: Urgency,
    summary: &str,
) -> bool {
    if rule.app_name.as_deref().is_some_and(|want| want != app_name) {
        return false;
    }
    if rule
        .desktop_entry
        .as_deref()
        .is_some_and(|want| want != desktop_entry)
    {
        return false;
    }
    if let Some(want) = rule.category.as_deref()
        && !category_matches(want, category)
    {
        return false;
    }
    if rule.urgency.is_some_and(|want| urgency_of(want) != urgency) {
        return false;
    }
    if rule
        .summary_contains
        .as_deref()
        .is_some_and(|needle| !summary.contains(needle))
    {
        return false;
    }
    true
}

/// A rule `category` against a card's: exact, or a `class.` prefix when
/// the rule's ends with a dot.
fn category_matches(want: &str, got: &str) -> bool {
    if let Some(prefix) = want.strip_suffix('.') {
        got == prefix || got.starts_with(want)
    } else {
        want == got
    }
}

fn urgency_of(want: RuleUrgency) -> Urgency {
    match want {
        RuleUrgency::Low => Urgency::Low,
        RuleUrgency::Normal => Urgency::Normal,
        RuleUrgency::Critical => Urgency::Critical,
    }
}

/// Applies every matching rule in `rules` (file order) to one fresh
/// arrival: later scalars win, every `exec` accumulates.
pub fn decide(
    rules: &[Rule],
    app_name: &str,
    desktop_entry: &str,
    category: &str,
    urgency: Urgency,
    summary: &str,
) -> Decision {
    let mut decision = Decision::default();
    for rule in rules {
        if !matches(rule, app_name, desktop_entry, category, urgency, summary) {
            continue;
        }
        if let Some(urgency) = rule.set_urgency {
            decision.urgency = Some(urgency_of(urgency));
        }
        if let Some(timeout) = rule.timeout_ms {
            decision.timeout = Some(if timeout == 0 {
                Timeout::Never
            } else {
                Timeout::Ms(timeout)
            });
        }
        if rule.hide {
            decision.hide = true;
        }
        // `history = false` is sticky: once a matching rule keeps a
        // card out, a later rule cannot opt it back in (a later `true`
        // is the default saying nothing).
        if !rule.history {
            decision.keep_history = false;
        }
        if let Some(output) = &rule.output {
            decision.output = Some(output.clone());
        }
        if rule.dnd_bypass {
            decision.dnd_bypass = true;
        }
        if let Some(exec) = &rule.exec {
            decision.execs.push(exec.clone());
        }
    }
    decision
}

#[cfg(test)]
mod tests;
