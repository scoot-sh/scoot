//! `[floating]` and `[[window_rule]]`: which windows float when they map,
//! and the modifier that drags them (`[floating] modifier`, see
//! [`drag_modifier`]).
//!
//! The decision is made once per window, at its first commit (see
//! `floating.rs`), from what the window has said about itself by then and
//! what the user wrote here. In order of strength, the window's own signals,
//! each on by default and all switched off together by `[floating] auto =
//! false`:
//!
//! 1. an `xdg_dialog_v1` object (the `xdg-dialog-v1` protocol) -- the
//!    standard way a toolkit says "this is a dialog";
//! 2. a parent (`xdg_toplevel.set_parent`) -- a transient window;
//! 3. a fixed size (`min_size == max_size`, both axes non-zero) -- a window
//!    that cannot be resized, which a column would stretch or leave mostly
//!    empty.
//!
//! Then the rules, in file order: a rule whose matchers all match sets what
//! it names (`float`, `size`), and a later matching rule overrides an earlier
//! one field by field. `float = false` is how a user keeps a window the
//! heuristics would float in the strip.
//!
//! # Matching: globs, not regexes
//!
//! `match_app_id` and `match_title` are globs over the whole string: `*`
//! matches any run of characters (including none), `?` exactly one, and
//! everything else itself; the match is case-sensitive. Chosen over regexes
//! for three reasons:
//!
//! - **The common rules are shorter and cannot be half-right.** An app id is
//!   usually matched exactly (`"org.gnome.Calculator"`), which is the glob
//!   as written; a regex needs `^...$` to mean that, and without the anchors
//!   `"foot"` also matches `"footclient"`. A title is usually matched by a
//!   fragment (`"*Preferences*"`), which reads as what it is.
//! - **No new dependency and no pathological input.** The matcher below is
//!   a few lines, allocation-light, and bounded by the pattern and title
//!   lengths ([`MAX_PATTERN_LEN`]); a regex engine is a crate this workspace
//!   does not otherwise pull in for the compositor.
//! - **What a window rule needs is membership, not extraction.** Nothing here
//!   captures a group or rewrites a title.
//!
//! # Failure semantics
//!
//! The same as every other config section (see `config.rs`'s module doc): a
//! structurally bad file (an unknown key, a wrong type) fails the whole
//! parse -- defaults at startup, a refused reload -- and a rule that parses
//! but cannot be used (no matcher, an over-long pattern, a size that is not
//! positive or is absurdly large, a rule that sets nothing) is skipped with a
//! warning naming it, the rest still apply. A reload also lists each skipped
//! rule in its `refused` reply, since a reload has somewhere to say so.

use scoot_core::Size;
use scoot_ipc::Modifier;
use serde::Deserialize;

#[cfg(test)]
mod tests;

/// The longest glob a rule may use, in bytes. Far past any real app id or
/// title fragment; what it bounds is the matcher's worst case (pattern
/// length times title length) at every window map.
pub const MAX_PATTERN_LEN: usize = 512;

/// The largest `size` a rule may ask for, per axis, in logical pixels: the
/// same bound `--width`/`--height` have (`cli::MAX_OUTPUT_DIMENSION`), past
/// which no output can be. A floating window is clamped to its output's
/// usable area anyway; this only refuses numbers that cannot be meant.
pub const MAX_RULE_SIZE: i64 = 65_535;

/// `[floating]` as written.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FloatingConfig {
    auto: Option<bool>,
    /// The modifier held to move (left button) and resize (right button) a
    /// floating window by dragging anywhere on it: a modifier name as a
    /// `[binds]` combo spells one (`super`, `alt`, `ctrl`, `shift`, or an
    /// alias). A string rather than an enum so a typo falls back to the
    /// default with a warning instead of failing the whole file.
    modifier: Option<String>,
}

/// The modifier the session drags floating windows with, when the file
/// names none (or one that is not a modifier): Super, the modifier every
/// default binding already uses.
pub const DEFAULT_DRAG_MODIFIER: Modifier = Modifier::Super;

/// `[floating] modifier`, resolved: the named modifier, or
/// [`DEFAULT_DRAG_MODIFIER`] when the table or key is absent -- and, with a
/// warning naming the value, when it names something that is not a single
/// modifier (a combo like `super+shift` included: one modifier drags).
pub fn drag_modifier(floating: Option<&FloatingConfig>) -> Modifier {
    let Some(name) = floating.and_then(|floating| floating.modifier.as_deref()) else {
        return DEFAULT_DRAG_MODIFIER;
    };
    Modifier::parse(name.trim()).unwrap_or_else(|| {
        tracing::warn!(
            modifier = name,
            "[floating] modifier is not one of super, alt, ctrl or shift; dragging \
             floating windows with super"
        );
        DEFAULT_DRAG_MODIFIER
    })
}

/// One `[[window_rule]]` as written.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WindowRuleConfig {
    match_app_id: Option<String>,
    match_title: Option<String>,
    float: Option<bool>,
    /// `[width, height]` in logical pixels. Read as `i64` so an out-of-range
    /// number skips this rule rather than failing the whole file.
    size: Option<[i64; 2]>,
}

/// A usable window rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowRule {
    /// Its position in the file, 1-based: what the log names it by.
    position: usize,
    app_id: Option<Glob>,
    title: Option<Glob>,
    float: Option<bool>,
    size: Option<Size>,
}

/// Everything that decides whether a window floats when it maps: what the
/// session runs with (`State::floating_rules`), and what a reload diffs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloatingRules {
    /// Whether the window's own signals (dialog, parent, fixed size) float
    /// it. `[floating] auto`, on by default.
    pub auto: bool,
    /// The usable `[[window_rule]]`s, in file order.
    pub rules: Vec<WindowRule>,
}

impl Default for FloatingRules {
    fn default() -> Self {
        Self {
            auto: true,
            rules: Vec::new(),
        }
    }
}

/// What a window has said about itself by its first commit.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapSignals<'a> {
    pub app_id: &'a str,
    pub title: &'a str,
    /// It has an `xdg_dialog_v1` object.
    pub dialog: bool,
    /// It named a parent (`set_parent`).
    pub parent: bool,
    /// Its minimum and maximum size are equal and non-zero on both axes.
    pub fixed_size: bool,
}

/// The decision for one window, with why -- which the log says, so a user
/// wondering why a window floats (or does not) can find out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub float: bool,
    /// The initial size to ask a floating window for, from a rule.
    pub size: Option<Size>,
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Nothing asked for floating: it tiles.
    Default,
    Dialog,
    Parent,
    FixedSize,
    /// The last matching rule that set `float`, by its position in the
    /// file (1-based, counting skipped rules too).
    Rule(usize),
}

impl FloatingRules {
    /// Builds the session's rules from the file's two sections, keeping the
    /// usable rules and describing each one it had to skip -- the caller
    /// warns (startup) or also refuses by name (reload).
    pub fn from_config(
        floating: Option<FloatingConfig>,
        rules: &[WindowRuleConfig],
    ) -> (Self, Vec<String>) {
        let mut skipped = Vec::new();
        let mut usable = Vec::with_capacity(rules.len());
        for (index, rule) in rules.iter().enumerate() {
            match WindowRule::validate(rule, index + 1) {
                Ok(rule) => usable.push(rule),
                Err(reason) => skipped.push(format!("window_rule #{} ({reason})", index + 1)),
            }
        }
        let auto = floating.and_then(|f| f.auto).unwrap_or(true);
        (
            Self {
                auto,
                rules: usable,
            },
            skipped,
        )
    }

    /// Whether a window with these `signals` floats, and at what size.
    pub fn decide(&self, signals: &MapSignals<'_>) -> Decision {
        let (mut float, mut reason) = if !self.auto {
            (false, Reason::Default)
        } else if signals.dialog {
            (true, Reason::Dialog)
        } else if signals.parent {
            (true, Reason::Parent)
        } else if signals.fixed_size {
            (true, Reason::FixedSize)
        } else {
            (false, Reason::Default)
        };
        let mut size = None;
        for rule in &self.rules {
            if !rule.matches(signals) {
                continue;
            }
            if let Some(rule_float) = rule.float {
                float = rule_float;
                reason = Reason::Rule(rule.position);
            }
            if rule.size.is_some() {
                size = rule.size;
            }
        }
        Decision {
            float,
            size: size.filter(|_| float),
            reason,
        }
    }
}

impl WindowRule {
    fn validate(rule: &WindowRuleConfig, position: usize) -> Result<Self, String> {
        if rule.match_app_id.is_none() && rule.match_title.is_none() {
            return Err(
                "names neither match_app_id nor match_title; use match_app_id = \"*\" to match every window"
                    .into(),
            );
        }
        if rule.float.is_none() && rule.size.is_none() {
            return Err("sets neither float nor size".into());
        }
        let size = match rule.size {
            None => None,
            Some([w, h]) => {
                let range = 1..=MAX_RULE_SIZE;
                if !range.contains(&w) || !range.contains(&h) {
                    return Err(format!(
                        "size [{w}, {h}] must be positive and at most {MAX_RULE_SIZE} on each axis"
                    ));
                }
                // In range by the check above.
                Some(Size::new(w as i32, h as i32))
            }
        };
        Ok(Self {
            position,
            app_id: rule.match_app_id.as_deref().map(Glob::new).transpose()?,
            title: rule.match_title.as_deref().map(Glob::new).transpose()?,
            float: rule.float,
            size,
        })
    }

    fn matches(&self, signals: &MapSignals<'_>) -> bool {
        self.app_id
            .as_ref()
            .is_none_or(|glob| glob.matches(signals.app_id))
            && self
                .title
                .as_ref()
                .is_none_or(|glob| glob.matches(signals.title))
    }
}

/// A whole-string glob: `*` any run of characters, `?` one character,
/// anything else itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob {
    pattern: Vec<char>,
}

impl Glob {
    fn new(pattern: &str) -> Result<Self, String> {
        if pattern.len() > MAX_PATTERN_LEN {
            return Err(format!(
                "a pattern is {} bytes long, past the {MAX_PATTERN_LEN}-byte limit",
                pattern.len()
            ));
        }
        Ok(Self {
            pattern: pattern.chars().collect(),
        })
    }

    /// Whether the whole of `text` matches.
    ///
    /// The standard greedy match with one backtrack point: on a mismatch
    /// after a `*`, the star takes one more character and the rest of the
    /// pattern is retried from there. Only the most recent star ever needs
    /// revisiting (an earlier one could only have taken characters the later
    /// one can take instead), so this is O(pattern x text) in the worst case
    /// and linear in the usual one, with no recursion.
    pub fn matches(&self, text: &str) -> bool {
        let pattern = &self.pattern;
        let text: Vec<char> = text.chars().collect();
        let (mut p, mut t) = (0, 0);
        // The pattern index just past the last `*`, and the text index that
        // star currently stops at.
        let mut star: Option<(usize, usize)> = None;
        while t < text.len() {
            match pattern.get(p) {
                Some('*') => {
                    star = Some((p + 1, t));
                    p += 1;
                }
                Some(&c) if c == '?' || c == text[t] => {
                    p += 1;
                    t += 1;
                }
                _ => match star {
                    Some((after, stopped)) => {
                        p = after;
                        t = stopped + 1;
                        star = Some((after, stopped + 1));
                    }
                    None => return false,
                },
            }
        }
        pattern[p..].iter().all(|&c| c == '*')
    }
}
