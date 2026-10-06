//! The pieces every binary's `--help` shares: where the human reference
//! lives, and the typo guesser behind every "did you mean".
//!
//! [`DOCS_URL`] is the one copy of the docs domain, so a domain move is one
//! edit: every SEE ALSO line renders from it (plus [`docs_tail`]), and the
//! tests in each binary pin that its pages name the live index. [`suggest`]
//! is the one copy of the typo guesser the client, the bar and the wallpaper
//! daemon all used to carry separately; a cold path (one process per
//! `--help`, one guess per usage error), so the small allocations cost
//! nothing at runtime.

/// Where the human reference lives: the docs site, whose agent-readable
/// index is `{DOCS_URL}/llms.txt` (one twin per page plus per-app sets).
pub const DOCS_URL: &str = "https://www.scoot.sh";

/// The shared SEE ALSO tail: the page's own reference, then the live
/// agent index. `page` is the path below [`DOCS_URL`] (`"msg/"`,
/// `"scootbar/cli.md"`), so every binary's pages end on the same two lines.
pub fn docs_tail(page: &str) -> String {
    format!("    docs: {DOCS_URL}/{page}\n    agents start at {DOCS_URL}/llms.txt\n")
}

/// The closest candidate to `input`, if it is close enough to be a typo
/// rather than a guess. Plain Levenshtein over chars; the bar is about a
/// quarter of the longer word (at least 1, at most 3), so `windwos` finds
/// `windows` while `--bogus` finds nothing to guess. An exact match is never
/// a suggestion.
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for candidate in candidates {
        if candidate == input {
            continue;
        }
        let distance = levenshtein(input, candidate);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((candidate, distance));
        }
    }
    let (candidate, distance) = best?;
    let longest = input.chars().count().max(candidate.chars().count());
    let allowance = (longest / 4 + 1).clamp(1, 3);
    (distance <= allowance && distance < longest).then_some(candidate)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = prev[j] + usize::from(ca != cb);
            current[j + 1] = (prev[j + 1] + 1).min(current[j] + 1).min(substitution);
        }
        std::mem::swap(&mut prev, &mut current);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_names_the_page_and_the_live_index() {
        // The one constant every binary renders from: the page it names,
        // plus the live agent index, on the same two lines.
        assert_eq!(DOCS_URL, "https://www.scoot.sh");
        let tail = docs_tail("msg/");
        assert!(tail.contains("https://www.scoot.sh/msg/"), "{tail}");
        assert!(tail.contains("https://www.scoot.sh/llms.txt"), "{tail}");
    }

    #[test]
    fn typos_find_their_word_and_garbage_finds_nothing() {
        // The union of the vectors the three copies each pinned, so the one
        // copy keeps every behaviour the callers relied on: request verbs,
        // flags, msg commands and module-ish words alike.
        for (typo, word) in [
            ("subscrib", "subscribe"),
            ("windwos", "windows"),
            ("ouptuts", "outputs"),
            ("screeshot", "screenshot"),
            ("actoin", "action"),
            ("keybaord", "keyboard"),
            ("focus-colum", "focus-column"),
            ("togle-fullscreen", "toggle-fullscreen"),
            ("--heigth", "--height"),
            ("--ouptuts", "--outputs"),
            ("qurey", "query"),
            ("queery", "query"),
            ("klll", "kill"),
        ] {
            let candidates = [
                "subscribe",
                "windows",
                "outputs",
                "screenshot",
                "action",
                "keyboard",
                "focus-column",
                "toggle-fullscreen",
                "--height",
                "--outputs",
                "query",
                "kill",
            ];
            assert_eq!(suggest(typo, candidates), Some(word), "{typo}");
        }
        for garbage in ["xyzzy", "", "q"] {
            assert_eq!(suggest(garbage, ["windows", "query"]), None, "{garbage}");
        }
        // An exact match is never a suggestion.
        assert_eq!(suggest("query", ["query"]), None);
    }
}
