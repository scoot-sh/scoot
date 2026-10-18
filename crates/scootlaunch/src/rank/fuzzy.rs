//! The fuzzy matcher: a case-folded subsequence match with
//! fzf-style bonuses, field weights, and a character-mask pre-filter.
//!
//! The query and every searched field are folded to a common case
//! first ([`fold_into`]), so matching is a plain substring-style scan
//! over folded text. Each field carries a 64-bit "characters present"
//! mask ([`mask_of`]): when the query's mask is not a subset of the
//! field's, no subsequence match is possible and the field is skipped
//! with one AND. What remains scores ([`score`]): a base per matched
//! character, bonuses at a field start, after a separator and for
//! consecutive characters, and a gap penalty per skipped character.
//!
//! Weights pick which field's match counts most: the entry keeps the
//! best weighted field score. `Name` 1.0, `GenericName` 0.6,
//! `Keywords` 0.5, the Exec basename 0.4. `Comment` is shown but never
//! searched, which keeps results from turning noisy.
//!
//! This file is `std`-only, so the fuzz target can compile it by
//! `#[path]` with nothing else. Nothing here allocates: folding
//! writes into a caller buffer, scoring reads only.

#[cfg(test)]
mod tests;

/// Base score per matched character.
const BASE: i32 = 10;
/// Bonus for a match at the very start of the field.
const START: i32 = 15;
/// Bonus for a match right after a separator (a word start).
const WORD: i32 = 12;
/// Bonus for a match directly consecutive to the previous one.
const CONSECUTIVE: i32 = 8;
/// Penalty per skipped character between matches (and before the
/// first match).
const GAP: i32 = 2;

/// Field weights as numerators over 10: `Name` 1.0, `GenericName`
/// 0.6, `Keywords` 0.5, the Exec basename 0.4.
pub const WEIGHT_NAME: u32 = 10;
/// [`WEIGHT_NAME`] for `GenericName`.
pub const WEIGHT_GENERIC: u32 = 6;
/// [`WEIGHT_NAME`] for `Keywords`.
pub const WEIGHT_KEYWORDS: u32 = 5;
/// [`WEIGHT_NAME`] for the Exec basename.
pub const WEIGHT_EXEC: u32 = 4;

/// Folded codepoints that start a word: a match right after one
/// takes the [`WORD`] bonus.
fn is_separator_code(code: u32) -> bool {
    matches!(
        code,
        0x20 | 0x09 | 0x2D | 0x5F | 0x2F | 0x2E | 0x28 | 0x3A | 0x2B | 0x27
    )
}

/// Folds one character of the query or a field into `out`. ASCII and
/// most scripts fold through lowercase; the few characters whose
/// full case folding expands to more than their lowercase are mapped
/// by hand (`ß` matches "ss", `ſ` matches "s", `K` matches "k").
/// This is an approximation of full case folding, documented as one:
/// it misses locale-specific rules (Turkish dotted/dotless `i`) and
/// compatibility expansions beyond the three above.
fn fold_char(char: char, out: &mut String) {
    match char {
        'ß' | 'ẞ' => out.push_str("ss"),
        'ſ' | 'ẛ' => out.push('s'),
        'K' => out.push('k'),
        _ => {
            for folded in char.to_lowercase() {
                out.push(folded);
            }
        }
    }
}

/// Folds `text` into `out`, reusing its buffer (no allocation once it
/// is large enough). The folded form is what [`score`] and
/// [`mask_of`] read.
pub fn fold_into(text: &str, out: &mut String) {
    out.clear();
    for char in text.chars() {
        fold_char(char, out);
    }
}

/// The "characters present" mask of folded text: one bit per
/// character, hashed to 64 bits. A query whose mask is not a subset
/// of a field's mask cannot match it as a subsequence, so the field
/// is skipped with one AND. Collisions only cost a wasted scoring
/// pass, never a missed match.
///
/// The unit tests and the fuzz target read this; the ranker reads
/// [`mask_of_codes`] over its stored codepoints.
#[allow(dead_code)]
pub fn mask_of(folded: &str) -> u64 {
    let mut mask = 0u64;
    for char in folded.chars() {
        let bit = (char as u32).wrapping_mul(0x9E37_79B1) >> 26;
        mask |= 1u64 << bit;
    }
    mask
}

/// The mask over folded codepoints: what the ranker reads.
pub fn mask_of_codes(codes: &[u32]) -> u64 {
    let mut mask = 0u64;
    for code in codes.iter().copied() {
        let bit = code.wrapping_mul(0x9E37_79B1) >> 26;
        mask |= 1u64 << bit;
    }
    mask
}

/// Answers whether `query_mask` allows a match against `field_mask`.
#[inline]
pub fn mask_allows(query_mask: u64, field_mask: u64) -> bool {
    query_mask & !field_mask == 0
}

/// Scores `query` (already folded) against `field` (already folded):
/// `Some` total when every query character appears in order, `None`
/// otherwise. A dynamic program over the alignment matrix: each
/// query character takes the previous match that maximizes the base
/// plus the start, word and consecutive bonuses minus the gap
/// penalties, so a later word start beats an earlier mid-word
/// scatter without losing a longer global match. Runs in
/// query-by-field time into the reused rows of `dp` (no allocation
/// once they fit the longest field). A query longer than the field
/// cannot match. The total is clamped at zero (a match is a match:
/// far-apart scatters all score alike). An empty query matches
/// everything with 0.
///
/// This is the string form, for tests and the fuzz target; the
/// ranker reads [`score_codes`] over pre-folded codepoints.
#[allow(dead_code)]
pub fn score(field: &str, query: &str, dp: &mut Dp) -> Option<i32> {
    let field: Vec<u32> = field.chars().map(|char| char as u32).collect();
    let query: Vec<u32> = query.chars().map(|char| char as u32).collect();
    score_codes(&field, &query, dp)
}

/// Scores folded codepoint slices: the [`score`] dynamic program
/// without the string decoding. A prefix hit answers the prefix
/// total directly (the optimal alignment: start bonus, then every
/// step consecutive), so the common typed-ahead case costs one
/// `starts_with`, not the matrix.
pub fn score_codes(field: &[u32], query: &[u32], dp: &mut Dp) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if query.len() > field.len() {
        return None;
    }
    if field.starts_with(query) {
        return Some(prefix_total(query.len()));
    }
    dp.resize(field.len());
    let Dp { prev, cur } = dp;
    let mut first = true;
    let mut best: i32 = NEG;
    for want in query.iter().copied() {
        let mut running: i32 = NEG;
        let mut prev_code: Option<u32> = None;
        best = NEG;
        for (index, code) in field.iter().copied().enumerate() {
            // Fold the position before this one into the running
            // best: an alignment for the previous query character
            // ends strictly before here.
            if !first && index > 0 && prev[index - 1] != NEG {
                running = running.max(
                    prev[index - 1]
                        .saturating_add((index as i32).saturating_sub(1).saturating_mul(GAP)),
                );
            }
            if code == want {
                let sub = if first {
                    let mut sub = BASE.saturating_sub((index as i32).saturating_mul(GAP));
                    if index == 0 {
                        sub = sub.saturating_add(START);
                    } else if prev_code.is_some_and(is_separator_code) {
                        sub = sub.saturating_add(WORD);
                    }
                    sub
                } else if index > 0 && prev[index - 1] != NEG {
                    // Continuing a run: the consecutive bonus, never
                    // stacked with the word bonus. A run that started
                    // at a word start already carries it.
                    prev[index - 1]
                        .saturating_add(BASE)
                        .saturating_add(CONSECUTIVE)
                } else if running != NEG {
                    running
                        .saturating_sub((index as i32).saturating_sub(1).max(0).saturating_mul(GAP))
                        .saturating_add(BASE)
                        .saturating_add(if prev_code.is_some_and(is_separator_code) {
                            WORD
                        } else {
                            0
                        })
                } else {
                    NEG
                };
                cur[index] = sub;
                best = best.max(sub);
            } else {
                cur[index] = NEG;
            }
            prev_code = Some(code);
        }
        if best == NEG {
            return None;
        }
        std::mem::swap(prev, cur);
        first = false;
    }
    Some(best.max(0))
}

/// The prefix total for a query of `len` characters: start bonus,
/// then every step consecutive. The optimal alignment for a prefix
/// hit, answered without the matrix.
fn prefix_total(len: usize) -> i32 {
    perfect(len)
}

/// The reused alignment rows for [`score`]: two rows of field width.
/// Grows to the longest field seen (the warm-up), then allocates no
/// more.
#[derive(Debug, Default)]
pub struct Dp {
    prev: Vec<i32>,
    cur: Vec<i32>,
}

/// Below any reachable score: half of `i32::MIN`, so saturating
/// arithmetic around it stays put.
const NEG: i32 = i32::MIN / 2;

impl Dp {
    /// Empty rows; the first [`score`] call sizes them.
    pub fn new() -> Self {
        Dp::default()
    }

    fn resize(&mut self, width: usize) {
        if self.prev.len() < width {
            self.prev.resize(width, NEG);
            self.cur.resize(width, NEG);
        }
        self.prev[..width].fill(NEG);
        self.cur[..width].fill(NEG);
    }
}

/// The score of a prefix match: every query character consecutive
/// from the field start. Not the ceiling (a word start later can add
/// more), but the scale the frecency bonus is capped against: a
/// quarter of this, so a habit never outranks a much better text
/// match.
pub fn perfect(query_chars: usize) -> i32 {
    if query_chars == 0 {
        return 0;
    }
    START
        .saturating_add(query_chars as i32 * BASE)
        .saturating_add(
            (query_chars as i32)
                .saturating_sub(1)
                .saturating_mul(CONSECUTIVE),
        )
}

/// Weights a raw field score: `raw * weight / 10`, never negative.
pub fn weigh(raw: i32, weight: u32) -> i32 {
    let scaled = (raw.max(0) as u32).saturating_mul(weight) / 10;
    scaled.min(i32::MAX as u32) as i32
}
