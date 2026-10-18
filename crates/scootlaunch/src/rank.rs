//! Ranking: fuzzy match plus frecency, per keystroke.
//!
//! The index builds [`Row`]s once ([`build_rows`]): every entry is
//! one row, and every desktop action is its own row ("Firefox: New
//! Private Window"), searchable by its name with the entry's other
//! fields alongside. Each row holds its folded fields and their
//! character masks up front, so a query never folds an entry.
//!
//! [`Ranker`] then ranks per keystroke with no allocation after
//! warm-up (held by a `count_allocations == 0` test): the folded
//! query and the score table live in reused buffers. Scoring is the
//! mask pre-filter plus one fuzzy pass per surviving field; a
//! partial story is unnecessary — the whole table sorts in place.
//! An empty query orders entry rows by frecency, then name. A typed
//! query orders rows by match score plus a frecency bonus capped at
//! a quarter of a perfect match, so a habit never outranks a much
//! better text match.
//!
//! This file is `std`-only beside the index types, like the modules
//! below it.

pub mod frecency;
pub mod fuzzy;

#[cfg(test)]
mod tests;

use frecency::Frecency;

/// The frecency bonus divisor: one recent launch is worth
/// `100 / FRECENCY_DIVISOR` points against a perfect match of
/// `18n + 7`. Small enough that a single launch nudges, large
/// enough that a real habit shows.
const FRECENCY_DIVISOR: u64 = 32;

/// One searchable row: an entry, or one of its actions. The `name`
/// slot holds the entry name, or "Entry: Action" for an action row.
#[derive(Debug, Clone)]
pub struct Row {
    /// Index into the entry slice the rows were built from.
    pub entry: usize,
    /// Which action of the entry, if this is an action row.
    pub action: Option<usize>,
    /// The entry name (or "Entry: Action"), folded: the empty-query
    /// order and the name tie-break read this.
    pub name_folded: String,
    /// The entry id, for the deterministic tie-break.
    pub id: String,
    /// The searched fields with their weights: name, generic name,
    /// joined keywords, Exec basename.
    pub fields: [Field; 4],
}

/// One searched field: folded codepoints, their mask, and the
/// weight. Codepoints, not text, so the hot loop never decodes
/// UTF-8; folding and masking happen once at row build.
#[derive(Debug, Clone)]
pub struct Field {
    pub codes: Vec<u32>,
    pub mask: u64,
    pub weight: u32,
}

/// Builds the searchable rows for `entries`: one row per entry plus
/// one per desktop action. Allocates; the per-keystroke [`Ranker`]
/// reuses these without further allocation.
pub fn build_rows(entries: &[crate::index::Entry]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        rows.push(entry_row(index, entry));
        for (action_index, action) in entry.actions.iter().enumerate() {
            rows.push(action_row(index, action_index, entry, &action.name));
        }
    }
    rows
}

/// The basename of the launched executable: argv[0], or argv[1] when
/// `Terminal=true` prefixes one. Empty (`DBusActivatable` without
/// `Exec`) means no Exec field.
fn exec_basename(exec: &[String], terminal: bool) -> &str {
    let arg = if terminal {
        exec.get(1).or_else(|| exec.first())
    } else {
        exec.first()
    };
    match arg {
        Some(exe) => exe.rsplit('/').next().unwrap_or(exe),
        None => "",
    }
}

fn field_of(folded: &str, weight: u32) -> Field {
    let codes: Vec<u32> = folded.chars().map(|char| char as u32).collect();
    let mask = fuzzy::mask_of_codes(&codes);
    Field {
        codes,
        mask,
        weight,
    }
}

fn fold(text: &str) -> String {
    let mut out = String::new();
    fuzzy::fold_into(text, &mut out);
    out
}

fn entry_row(index: usize, entry: &crate::index::Entry) -> Row {
    let name_folded = fold(&entry.name);
    let generic_folded = fold(entry.generic_name.as_deref().unwrap_or(""));
    let keywords_folded = fold(&entry.keywords.join(" "));
    let exec_folded = fold(exec_basename(&entry.exec, entry.terminal));
    Row {
        entry: index,
        action: None,
        name_folded: name_folded.clone(),
        id: entry.id.clone(),
        fields: [
            field_of(&name_folded, fuzzy::WEIGHT_NAME),
            field_of(&generic_folded, fuzzy::WEIGHT_GENERIC),
            field_of(&keywords_folded, fuzzy::WEIGHT_KEYWORDS),
            field_of(&exec_folded, fuzzy::WEIGHT_EXEC),
        ],
    }
}

fn action_row(index: usize, action_index: usize, entry: &crate::index::Entry, action: &str) -> Row {
    let title = format!("{}: {action}", entry.name);
    let name_folded = fold(&title);
    let generic_folded = fold(entry.generic_name.as_deref().unwrap_or(""));
    let keywords_folded = fold(&entry.keywords.join(" "));
    let exec_folded = fold(exec_basename(&entry.exec, entry.terminal));
    Row {
        entry: index,
        action: Some(action_index),
        name_folded: name_folded.clone(),
        id: entry.id.clone(),
        fields: [
            field_of(&name_folded, fuzzy::WEIGHT_NAME),
            field_of(&generic_folded, fuzzy::WEIGHT_GENERIC),
            field_of(&keywords_folded, fuzzy::WEIGHT_KEYWORDS),
            field_of(&exec_folded, fuzzy::WEIGHT_EXEC),
        ],
    }
}

/// The per-keystroke ranker: scratch buffers reused across queries,
/// so ranking allocates nothing after warm-up.
#[derive(Debug)]
pub struct Ranker {
    folded: String,
    codes: Vec<u32>,
    scores: Vec<(u32, i32)>,
    order: Vec<u32>,
    dp: fuzzy::Dp,
}

impl Ranker {
    /// Empty buffers; the first query sizes them (the warm-up).
    pub fn new() -> Self {
        Ranker {
            folded: String::new(),
            codes: Vec::new(),
            scores: Vec::new(),
            order: Vec::new(),
            dp: fuzzy::Dp::new(),
        }
    }

    /// Ranks `rows` for `query`, best row first, as row indices into
    /// `rows`. Entry rows hidden by the caller stay hidden: the
    /// caller filters them before or after. Score ties break by row
    /// index, which is entry id order with each entry's actions after
    /// it: deterministic, stable across runs, and an integer compare
    /// (the thousands-tied case in the latency bar). The returned
    /// slice borrows the ranker: rank again to re-rank.
    pub fn rank(&mut self, query: &str, rows: &[Row], frecency: &Frecency, now: u64) -> &[u32] {
        fuzzy::fold_into(query, &mut self.folded);
        if self.folded.is_empty() {
            return self.rank_empty(rows, frecency, now);
        }
        self.codes.clear();
        self.codes
            .extend(self.folded.chars().map(|char| char as u32));
        let query_mask = fuzzy::mask_of_codes(&self.codes);
        let cap = fuzzy::perfect(self.codes.len()) / 4;
        self.scores.clear();
        for (index, row) in rows.iter().enumerate() {
            let mut best = 0;
            let mut matched = false;
            for field in &row.fields {
                if field.codes.is_empty() || !fuzzy::mask_allows(query_mask, field.mask) {
                    continue;
                }
                if let Some(raw) = fuzzy::score_codes(&field.codes, &self.codes, &mut self.dp) {
                    // A far-apart scatter clamps to zero but still
                    // matched: it ranks below every positive, out
                    // only when nothing matched at all.
                    matched = true;
                    best = best.max(fuzzy::weigh(raw, field.weight));
                }
            }
            // A row with no field match is out, however loved: the
            // bonus rides on a text match, never alone.
            if !matched {
                continue;
            }
            let bonus = frecency_bonus(frecency, &row.id, now, cap);
            self.scores.push((index as u32, best.saturating_add(bonus)));
        }
        self.scores.sort_unstable_by(|a, b| {
            // Score first, then build order: an integer compare even
            // when thousands of rows tie (single-char queries).
            b.1.cmp(&a.1).then(a.0.cmp(&b.0))
        });
        self.order.clear();
        self.order.extend(self.scores.iter().map(|scored| scored.0));
        &self.order
    }

    /// The empty-query order: entry rows only (actions surface
    /// through typing), by frecency weight, then folded name, then
    /// id.
    fn rank_empty(&mut self, rows: &[Row], frecency: &Frecency, now: u64) -> &[u32] {
        self.order.clear();
        self.order.extend(
            rows.iter()
                .enumerate()
                .filter(|(_, row)| row.action.is_none())
                .map(|(index, _)| index as u32),
        );
        self.order.sort_unstable_by(|a, b| {
            let (ra, rb) = (&rows[*a as usize], &rows[*b as usize]);
            frecency
                .weight(&rb.id, now)
                .cmp(&frecency.weight(&ra.id, now))
                .then(ra.name_folded.cmp(&rb.name_folded))
                .then(ra.id.cmp(&rb.id))
        });
        &self.order
    }
}

/// The frecency bonus for one row: the entry's weight, scaled down
/// and capped at a quarter of a perfect match for this query. Zero
/// for an empty query (the empty order already is frecency).
fn frecency_bonus(frecency: &Frecency, id: &str, now: u64, cap: i32) -> i32 {
    let scaled = frecency.weight(id, now) / FRECENCY_DIVISOR;
    (scaled.min(cap.max(0) as u64)) as i32
}

/// Maps a row order back to entries: each entry once, at its best
/// row's position. Hidden entries stay out unless `include_hidden`
/// (the `--all` flag): `--query` lists what the UI would show.
pub fn ranked_entries<'a>(
    entries: &'a [crate::index::Entry],
    rows: &[Row],
    order: &[u32],
    include_hidden: bool,
) -> Vec<&'a crate::index::Entry> {
    let mut seen = vec![false; entries.len()];
    let mut out = Vec::new();
    for row in order.iter().map(|index| &rows[*index as usize]) {
        if !include_hidden && entries[row.entry].hidden.is_some() {
            continue;
        }
        if !seen[row.entry] {
            seen[row.entry] = true;
            out.push(&entries[row.entry]);
        }
    }
    out
}
