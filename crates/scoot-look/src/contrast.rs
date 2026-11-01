//! Contrast (design section 3.5): the four refusing pairs; `dim`, `urgent`,
//! `dim` on `surface` and the inactive ring (`border`) reported.
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
/// Never panics: a malformed color reads as black. The pipeline only calls
/// this on resolved looks whose colors passed the `#rrggbb` grammar, so the
/// fallback is unreachable there; it exists because `ResolvedLook` fields
/// are public and a caller can always build one by hand.
pub fn luminance(hex: &str) -> f64 {
    let byte = |i: usize| {
        hex.as_bytes()
            .get(1 + 2 * i..3 + 2 * i)
            .and_then(|s| std::str::from_utf8(s).ok())
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(0) as f64
            / 255.0
    };
    0.2126 * channel(byte(0)) + 0.7152 * channel(byte(1)) + 0.0722 * channel(byte(2))
}

/// WCAG contrast ratio of two `#rrggbb` colors.
///
/// The stored value is always the full-precision measurement: bars are
/// compared on this unrounded value, so a real 4.46 is still a warning
/// against a 4.5 bar. Human output prints [`format_ratio`] (two decimals);
/// machine output (`scoot look show --json`) carries the full value.
pub fn ratio(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The ratio as human output shows it: two decimals. The bar comparison
/// itself runs on the unrounded [`ratio`], so a warning line always means
/// below the bar; printing hundredths (not tenths) shrinks the window in
/// which a failing pair can print as its bar from ±0.05 to ±0.005.
pub fn format_ratio(ratio: f64) -> String {
    format!("{ratio:.2}")
}

/// The card fill's two stops over `surface` for `effects.fill = "gradient"`,
/// the colors `scootui::effects::FillStyle::stops` paints: a dark surface
/// (luminance under 0.5) is lightened 26/255 toward white at the top, a light
/// one darkened 20/255 toward black at the bottom. Kept in step with that
/// function by hand: this crate is pure and cannot depend on `scootui`.
fn gradient_stops(surface: &str) -> [String; 2] {
    let channel = |i: usize| {
        surface
            .as_bytes()
            .get(1 + 2 * i..3 + 2 * i)
            .and_then(|s| std::str::from_utf8(s).ok())
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(0) as u32
    };
    let (target, toward) = if luminance(surface) < 0.5 {
        (255, 26)
    } else {
        (0, 20)
    };
    let mixed = |c: u32| (c * (255 - toward) + target * toward + 127) / 255;
    let moved = format!(
        "#{:02x}{:02x}{:02x}",
        mixed(channel(0)),
        mixed(channel(1)),
        mixed(channel(2))
    );
    [surface.to_string(), moved]
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

/// `dim` on the look's surface; with `effects.fill = "gradient"`, on
/// whichever of the two gradient stops it contrasts with least.
fn dim_on_surface(look: &ResolvedLook) -> ContrastPair {
    let p = &look.palette;
    let stops = if look.effects.fill.as_deref() == Some("gradient") {
        gradient_stops(&p.surface)
    } else {
        [p.surface.clone(), p.surface.clone()]
    };
    let [first, second] = stops;
    let (background, worst) = [first, second]
        .into_iter()
        .map(|stop| {
            let r = ratio(&p.dim, &stop);
            (stop, r)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("two stops");
    ContrastPair {
        name: "dim/surface",
        foreground: p.dim.clone(),
        background,
        ratio: worst,
        threshold: 4.5,
        refuses: false,
    }
}

/// Measure a resolved look. `cursor.accent = "none"` cannot be measured and
/// is skipped; every other pair is always measured.
pub fn check(look: &ResolvedLook, origin: Origin) -> ContrastReport {
    let refuses = origin.contrast_refuses();
    let p = &look.palette;
    let pairs = vec![
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
        // `dim` draws real text on the card and the launcher panel, which
        // sit on `surface` (the card fill), so it gets the text bar there.
        // Reported, not refused, with the rest of the `dim` checks. A
        // gradient fill shifts the surface across the card, so the worse
        // of its two stops is the one measured.
        dim_on_surface(look),
        // The inactive focus ring is `border`: an unfocused window's edge,
        // a UI outline held to the WCAG 3:1 floor. Reported, not refused.
        ContrastPair {
            name: "border/background",
            foreground: p.border.clone(),
            background: p.background.clone(),
            ratio: ratio(&p.border, &p.background),
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
    // The stored ratio stays full-precision: `refusals` and `warnings`
    // compare on the unrounded value, and callers print `format_ratio`.
    ContrastReport { pairs }
}

#[cfg(test)]
mod tests;
