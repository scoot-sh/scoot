//! The ranking's tests: ordering pins, edge cases, and the
//! allocation and latency bars.

use super::frecency::Frecency;
use super::*;

fn entry(
    id: &str,
    name: &str,
    generic: Option<&str>,
    keywords: &[&str],
    exec: &str,
) -> crate::index::Entry {
    entry_with_actions(id, name, generic, keywords, exec, &[])
}

fn entry_with_actions(
    id: &str,
    name: &str,
    generic: Option<&str>,
    keywords: &[&str],
    exec: &str,
    actions: &[(&str, &str)],
) -> crate::index::Entry {
    crate::index::Entry {
        id: id.to_string(),
        name: name.to_string(),
        generic_name: generic.map(str::to_string),
        comment: None,
        keywords: keywords.iter().map(|word| word.to_string()).collect(),
        icon: None,
        exec: exec.split(' ').map(str::to_string).collect(),
        terminal: false,
        dbus_activatable: false,
        actions: actions
            .iter()
            .map(|(aid, aname)| crate::index::Action {
                id: aid.to_string(),
                name: aname.to_string(),
                exec: vec![exec.to_string()],
            })
            .collect(),
        path: std::path::PathBuf::from(format!("/x/{id}")),
        hidden: None,
    }
}

fn rank_names(
    query: &str,
    entries: &[crate::index::Entry],
    history: &Frecency,
    now: u64,
) -> Vec<String> {
    let rows = build_rows(entries);
    let mut ranker = Ranker::new();
    // Warm up first: buffers sized, like the UI's second keystroke.
    let _ = ranker.rank("a warmed-up query", &rows, history, now);
    let order = ranker.rank(query, &rows, history, now).to_vec();
    ranked_entries(entries, &rows, &order, false)
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

#[test]
fn a_prefix_beats_a_scattered_match() {
    let entries = [
        entry("wifi.desktop", "Wifi Config", None, &[], "/usr/bin/wifi"),
        entry("firefox.desktop", "Firefox", None, &[], "/usr/bin/firefox"),
    ];
    assert_eq!(
        rank_names("fi", &entries, &Frecency::empty(), 0),
        ["Firefox", "Wifi Config"]
    );
}

#[test]
fn a_habit_never_beats_a_much_better_match() {
    let now = 1_800_000_000;
    let mut history = Frecency::empty();
    // 200 launches yesterday: the maximum realistic love.
    history.insert_for_tests("state-art-museum.desktop", 200, now - 1_000);
    let entries = [
        entry(
            "state-art-museum.desktop",
            "State Art Museum",
            None,
            &[],
            "/usr/bin/museum",
        ),
        entry("terminal.desktop", "Terminal", None, &[], "/usr/bin/term"),
    ];
    // "term" is a prefix of Terminal and a far scatter in State Art
    // Museum: the habit loses. (Drop the bonus cap and this fails.)
    assert_eq!(
        rank_names("term", &entries, &history, now),
        ["Terminal", "State Art Museum"]
    );
}

#[test]
fn a_habit_breaks_a_close_match() {
    let now = 1_800_000_000;
    let mut history = Frecency::empty();
    for _ in 0..5 {
        history.insert_for_tests("firewall.desktop", 5, now - 1_000);
    }
    let entries = [
        entry("firefox.desktop", "Firefox", None, &[], "/usr/bin/firefox"),
        entry(
            "firewall.desktop",
            "Firewall",
            None,
            &[],
            "/usr/bin/firewall",
        ),
    ];
    // Both prefix-match "fire": the launched one wins.
    assert_eq!(
        rank_names("fire", &entries, &history, now),
        ["Firewall", "Firefox"]
    );
}

#[test]
fn unicode_and_case_fold_across_query_and_fields() {
    let entries = [
        entry(
            "maps.desktop",
            "Berliner Straße",
            None,
            &["karte"],
            "/usr/bin/maps",
        ),
        entry("calc.desktop", "Calculator", None, &[], "/usr/bin/calc"),
    ];
    for query in ["strasse", "STRASSE", "Straße"] {
        assert_eq!(
            rank_names(query, &entries, &Frecency::empty(), 0),
            ["Berliner Straße"],
            "{query}"
        );
    }
    assert_eq!(
        rank_names("KARTE", &entries, &Frecency::empty(), 0),
        ["Berliner Straße"]
    );
}

#[test]
fn empty_query_orders_by_frecency_then_name() {
    let now = 1_800_000_000;
    let mut history = Frecency::empty();
    history.insert_for_tests("zebra.desktop", 1, now - 100);
    let entries = [
        entry("zebra.desktop", "Zebra", None, &[], "/usr/bin/zebra"),
        entry("apple.desktop", "Apple", None, &[], "/usr/bin/apple"),
        entry("mango.desktop", "Mango", None, &[], "/usr/bin/mango"),
    ];
    // Loved first, then alphabetical — even over a name prefix.
    assert_eq!(
        rank_names("", &entries, &history, now),
        ["Zebra", "Apple", "Mango"]
    );
    assert_eq!(
        rank_names("", &entries, &Frecency::empty(), now),
        ["Apple", "Mango", "Zebra"]
    );
}

#[test]
fn one_char_queries_match() {
    let entries = [
        entry("terminal.desktop", "Terminal", None, &[], "/usr/bin/term"),
        entry("firefox.desktop", "Firefox", None, &[], "/usr/bin/firefox"),
    ];
    assert_eq!(
        rank_names("f", &entries, &Frecency::empty(), 0),
        ["Firefox"]
    );
}

#[test]
fn no_entries_rank_empty() {
    let rows = build_rows(&[]);
    let mut ranker = Ranker::new();
    assert!(ranker.rank("fire", &rows, &Frecency::empty(), 0).is_empty());
    assert!(ranker.rank("", &rows, &Frecency::empty(), 0).is_empty());
}

#[test]
fn one_entry_ranks_when_it_matches() {
    let entries = [entry(
        "firefox.desktop",
        "Firefox",
        None,
        &[],
        "/usr/bin/firefox",
    )];
    assert_eq!(
        rank_names("fire", &entries, &Frecency::empty(), 0),
        ["Firefox"]
    );
    assert!(rank_names("zzz", &entries, &Frecency::empty(), 0).is_empty());
}

#[test]
fn action_rows_surface_under_their_names() {
    let entries = [entry_with_actions(
        "firefox.desktop",
        "Firefox",
        None,
        &[],
        "/usr/bin/firefox",
        &[("private", "New Private Window")],
    )];
    let rows = build_rows(&entries);
    let mut ranker = Ranker::new();
    let order = ranker
        .rank("private", &rows, &Frecency::empty(), 0)
        .to_vec();
    // The action row matched; the entry prints once.
    assert_eq!(order.len(), 1);
    assert_eq!(rows[order[0] as usize].action, Some(0));
    let listed = ranked_entries(&entries, &rows, &order, false);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Firefox");
}

#[test]
fn empty_query_hides_action_rows() {
    let entries = [entry_with_actions(
        "firefox.desktop",
        "Firefox",
        None,
        &[],
        "/usr/bin/firefox",
        &[("private", "New Private Window")],
    )];
    let rows = build_rows(&entries);
    let mut ranker = Ranker::new();
    let order = ranker.rank("", &rows, &Frecency::empty(), 0).to_vec();
    assert_eq!(order.len(), 1);
    assert_eq!(rows[order[0] as usize].action, None);
}

#[test]
fn hidden_entries_stay_out_without_all() {
    let mut hidden_entry = entry("secret.desktop", "Secret", None, &[], "/usr/bin/secret");
    hidden_entry.hidden = Some(crate::index::filter::HiddenReason::NoDisplay);
    let entries = [
        hidden_entry,
        entry("shown.desktop", "Shown Secret", None, &[], "/usr/bin/shown"),
    ];
    let rows = build_rows(&entries);
    let mut ranker = Ranker::new();
    let order = ranker.rank("secret", &rows, &Frecency::empty(), 0).to_vec();
    let listed: Vec<&str> = ranked_entries(&entries, &rows, &order, false)
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(listed, ["Shown Secret"]);
    let listed_all: Vec<&str> = ranked_entries(&entries, &rows, &order, true)
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(listed_all, ["Secret", "Shown Secret"]);
}

#[test]
fn keywords_and_exec_basename_search_with_less_weight() {
    let entries = [
        entry(
            "a.desktop",
            "Alpha Tool",
            None,
            &["browser"],
            "/usr/bin/alpha",
        ),
        entry("b.desktop", "Firefox", None, &[], "/usr/bin/firefox"),
    ];
    // A name match beats a keyword match for the same query — and a
    // row with no match at all stays out, however the rest rank.
    assert_eq!(
        rank_names("fire", &entries, &Frecency::empty(), 0),
        ["Firefox"]
    );
    let entries = [
        entry(
            "a.desktop",
            "Alpha Tool",
            None,
            &["campfire"],
            "/usr/bin/alpha",
        ),
        entry("b.desktop", "Firefox", None, &[], "/usr/bin/firefox"),
    ];
    assert_eq!(
        rank_names("fire", &entries, &Frecency::empty(), 0),
        ["Firefox", "Alpha Tool"]
    );
    let entries = [
        entry("a.desktop", "Alpha", None, &[], "/usr/bin/firetool"),
        entry("b.desktop", "Beta", None, &[], "/usr/bin/beta"),
    ];
    assert_eq!(
        rank_names("fire", &entries, &Frecency::empty(), 0),
        ["Alpha"]
    );
}

fn three_thousand_entries() -> Vec<crate::index::Entry> {
    let mut entries = Vec::with_capacity(3000);
    entries.push(entry(
        "firefox.desktop",
        "Firefox",
        Some("Web Browser"),
        &["web", "browser"],
        "/usr/bin/firefox",
    ));
    entries.push(entry(
        "terminal.desktop",
        "Terminal",
        None,
        &["shell"],
        "/usr/bin/foot",
    ));
    for i in 2..3000 {
        entries.push(entry(
            &format!("app-{i:04}.desktop"),
            &format!("Application {i:04} Tool"),
            Some("Utility"),
            &["tool", "utility"],
            &format!("/usr/bin/app-{i:04}"),
        ));
    }
    entries
}

#[test]
fn three_thousand_entries_rank_sanely() {
    let entries = three_thousand_entries();
    assert_eq!(
        rank_names("fire", &entries, &Frecency::empty(), 0)[0],
        "Firefox"
    );
    assert_eq!(
        rank_names("term", &entries, &Frecency::empty(), 0)[0],
        "Terminal"
    );
    // A query nothing holds ranks empty.
    assert!(rank_names("zzzqqq", &entries, &Frecency::empty(), 0).is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn a_warm_query_allocates_nothing() {
    let entries = three_thousand_entries();
    let rows = build_rows(&entries);
    let history = Frecency::empty();
    let mut ranker = Ranker::new();
    // Warm-up: size every buffer past the longest query below. The
    // long query alone is not enough: it matches nothing, so the
    // score table and the alignment rows never grow. The one-char
    // query matches nearly everything and sizes them.
    let _ = ranker.rank("a much longer warmed-up query string", &rows, &history, 0);
    let _ = ranker.rank("a", &rows, &history, 0);
    let _ = ranker.rank("", &rows, &history, 0);
    for query in ["fire", "f", "", "terminal emulator", "zzzqqq"] {
        let (_, allocations) = scootbg_mem::count_allocations(|| {
            ranker.rank(query, &rows, &history, 0);
        });
        assert_eq!(allocations, 0, "query {query:?}");
    }
}

#[test]
fn keystroke_latency_report() {
    let entries = three_thousand_entries();
    let rows = build_rows(&entries);
    let history = Frecency::empty();
    let mut ranker = Ranker::new();
    let _ = ranker.rank("warmed up", &rows, &history, 0);
    // Keystrokes of one typed query: the per-keystroke cost the
    // design caps at 1 ms for 3,000 entries on the M2.
    let mut typed = String::new();
    for key in "firefox".chars() {
        typed.push(key);
        let start = std::time::Instant::now();
        let order = ranker.rank(&typed, &rows, &history, 0).to_vec();
        let elapsed = start.elapsed();
        eprintln!("query {typed:?}: {} rows in {elapsed:?}", order.len());
    }
}
