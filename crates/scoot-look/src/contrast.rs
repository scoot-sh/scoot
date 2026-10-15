//! Contrast (design section 3.5): the four refusing pairs, `dim` and
//! `urgent` reported.
//!
//! Thresholds are the ones `nix/tests.nix:14907-14932` already applies to
//! `gtk.css`: foreground on background ≥ 4.5, accent-fg on accent ≥ 4.5,
//! accent on background ≥ 3.0, focus on background ≥ 3.0. Who a failure stops
//! depends on who chose the look ([`Origin::contrast_refuses`]): a shipped
//! look, and one arriving through `install`, is refused below a bar, with
//! the pair and the ratio named; a look the user's own configuration
//! produced gets a loud warning and is never refused.

use crate::resolved::ResolvedLook;
use crate::schema::Origin;

fn channel(c: f64) -> f64 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance, the formula of the existing Nix check.
pub fn luminance(hex: &str) -> f64 {
    let v =
        |i: usize| u8::from_str_radix(&hex[1 + 2 * i..3 + 2 * i], 16).unwrap_or(0) as f64 / 255.0;
    0.2126 * channel(v(0)) + 0.7152 * channel(v(1)) + 0.0722 * channel(v(2))
}

/// WCAG contrast ratio of two `#rrggbb` colors.
pub fn ratio(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// One measured pair.
#[derive(Debug, Clone)]
pub struct ContrastPair {
    pub name: &'static str,
    pub foreground: String,
    pub background: String,
    pub ratio: f64,
    pub threshold: f64,
    /// Below the bar: refuse, or warn.
    pub refuses: bool,
}

/// The full verdict for a resolved look.
#[derive(Debug, Clone)]
pub struct ContrastReport {
    pub pairs: Vec<ContrastPair>,
}

impl ContrastReport {
    /// Every refusing pair below its bar.
    pub fn refusals(&self) -> Vec<&ContrastPair> {
        self.pairs
            .iter()
            .filter(|p| p.refuses && p.ratio < p.threshold)
            .collect()
    }

    /// Every non-refusing pair below its bar: reported, never refused.
    pub fn warnings(&self) -> Vec<&ContrastPair> {
        self.pairs
            .iter()
            .filter(|p| !p.refuses && p.ratio < p.threshold)
            .collect()
    }
}

/// Measure a resolved look. `cursor.accent = "none"` cannot be measured and
/// is skipped; every other pair is always measured.
pub fn check(look: &ResolvedLook, origin: Origin) -> ContrastReport {
    let refuses = origin.contrast_refuses();
    let p = &look.palette;
    let mut pairs = vec![
        ContrastPair {
            name: "foreground/background",
            foreground: p.foreground.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.foreground, &p.background),
            threshold: 4.5,
            refuses,
        },
        ContrastPair {
            name: "accent-fg/accent",
            foreground: p.accent_fg.clone(),
            background: p.accent.clone(),
            ratio: ratio(&p.accent_fg, &p.accent),
            threshold: 4.5,
            refuses,
        },
        ContrastPair {
            name: "accent/background",
            foreground: p.accent.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.accent, &p.background),
            threshold: 3.0,
            refuses,
        },
        ContrastPair {
            name: "focus/background",
            foreground: p.focus.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.focus, &p.background),
            threshold: 3.0,
            refuses,
        },
        // `dim` is the role for inactive and incidental text, which WCAG
        // exempts, and `urgent` is a non-text alert indicator: reported,
        // never refused, in v1.
        ContrastPair {
            name: "dim/background",
            foreground: p.dim.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.dim, &p.background),
            threshold: 3.0,
            refuses: false,
        },
        ContrastPair {
            name: "urgent/background",
            foreground: p.urgent.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.urgent, &p.background),
            threshold: 3.0,
            refuses: false,
        },
    ];
    for pair in &mut pairs {
        // Round to one decimal, the precision the design's own table claims.
        pair.ratio = (pair.ratio * 10.0).round() / 10.0;
    }
    ContrastReport { pairs }
}

#[cfg(test)]
mod tests;
