//! The desktop entry index: a small, honest subset of the
//! [scootlaunch design](../../w9-launch/dev/design/scootlaunch.md#the-desktop-entry-index).
//! Enough for the gate: real ids, precedence, `Hidden` masking,
//! `NoDisplay`, `TryExec`, localized names, the Exec field codes the panel
//! exercises, and the 300 / 3,000-entry scale rows (synthetically padded,
//! said so wherever reported).

use std::collections::HashSet;
use std::env;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 4096;

pub struct Entry {
    pub id: String,
    pub name: String,
    pub generic: String,
    pub keywords: String,
    pub exec: String,
    pub icon: String,
    pub terminal: bool,
    pub hidden: Option<String>,
    pub path: PathBuf,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/share"));
    }
    let system = env::var_os("XDG_DATA_DIRS")
        .map(|s| {
            env::split_paths(&s)
                .map(|p| p.to_path_buf())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    dirs.extend(system);
    dirs
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(';') => {
                    cur.push(';');
                    chars.next();
                }
                _ => cur.push(c),
            }
        } else if c == ';' {
            if !cur.is_empty() || !items.is_empty() {
                items.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        items.push(cur);
    }
    items
}

fn locale_chain() -> Vec<String> {
    let raw = env::var("LC_ALL")
        .or_else(|_| env::var("LC_MESSAGES"))
        .or_else(|_| env::var("LANG"))
        .unwrap_or_default();
    let lang = raw.split('.').next().unwrap_or("").to_string();
    if lang.is_empty() {
        return Vec::new();
    }
    let mut chain = Vec::new();
    // lang_COUNTRY@MODIFIER, lang_COUNTRY, lang@MODIFIER, lang.
    if let Some(at) = lang.find('@') {
        let (base, modifier) = lang.split_at(at);
        if let Some(us) = base.find('_') {
            chain.push(format!("{base}{modifier}"));
            chain.push(base.to_string());
            let (l, _) = base.split_at(us);
            chain.push(format!("{l}{modifier}"));
            chain.push(l.to_string());
        } else {
            chain.push(lang.clone());
            chain.push(base.to_string());
        }
    } else if let Some(us) = lang.find('_') {
        chain.push(lang.clone());
        chain.push(lang[..us].to_string());
    } else {
        chain.push(lang);
    }
    chain
}

struct Groups {
    entry: Vec<(String, String)>,
}

fn parse_file(bytes: &[u8]) -> Option<Groups> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut entry = Vec::new();
    let mut in_entry = false;
    let mut seen_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            if in_entry {
                seen_entry = true;
            }
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let (key, mut value) = line.split_at(eq);
        value = &value[1..];
        let key = key.trim();
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '[' || c == ']')
        {
            continue;
        }
        entry.push((key.to_string(), value.trim().to_string()));
    }
    if !seen_entry {
        return None;
    }
    Some(Groups { entry })
}

fn localized(entry: &[(String, String)], key: &str, chain: &[String]) -> Option<String> {
    for locale in chain {
        let full = format!("{key}[{locale}]");
        if let Some((_, v)) = entry.iter().find(|(k, _)| k == &full) {
            return Some(unescape(v));
        }
    }
    entry
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| unescape(v))
}

fn raw(entry: &[(String, String)], key: &str) -> Option<String> {
    entry
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn which(name: &str) -> bool {
    if name.contains('/') {
        return fs::metadata(name).map(|m| m.is_file()).unwrap_or(false);
    }
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| {
        fs::metadata(dir.join(name))
            .map(|m| m.is_file())
            .unwrap_or(false)
    })
}

/// Minimal Exec expansion for the launch rows: quote-aware split, the
/// `%f/%F/%u/%U` and deprecated-code removal, `%i`/`%c`/`%k`/`%%`.
/// Returns `None` for entries the spec calls invalid (unknown field codes,
/// `=` in the executable).
pub fn expand_exec(exec: &str, name: &str, icon: &str, path: &Path) -> Option<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = exec.chars().peekable();
    let mut closed = true;
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_quotes => {
                in_quotes = true;
                closed = false;
            }
            '"' => {
                in_quotes = false;
                closed = true;
            }
            ' ' | '\t' if !in_quotes => {
                if !cur.is_empty() || !closed {
                    args.push(std::mem::take(&mut cur));
                    closed = true;
                }
            }
            '\\' if in_quotes => match chars.next() {
                Some('`') => cur.push('`'),
                Some('$') => cur.push('$'),
                Some('"') => cur.push('"'),
                Some('\\') => cur.push('\\'),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            other => cur.push(other),
        }
    }
    if in_quotes {
        return None;
    }
    if !cur.is_empty() || !closed {
        args.push(cur);
    }
    if args.is_empty() {
        return None;
    }
    if args[0].contains('=') {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        if i == 0 {
            out.push(arg.clone());
            continue;
        }
        // Field codes expand outside quotes only; a quoted arg keeps `%%`.
        let quoted = exec.contains(&format!("\"{arg}\""));
        let mut expanded = String::new();
        let mut cs = arg.chars().peekable();
        let mut ok = true;
        while let Some(c) = cs.next() {
            if c == '%' {
                match cs.next() {
                    Some('%') => expanded.push('%'),
                    Some('f') | Some('F') | Some('u') | Some('U') | Some('d') | Some('D')
                    | Some('n') | Some('N') | Some('v') | Some('m') => {}
                    Some('i') if !quoted => {
                        if !icon.is_empty() {
                            out.push("--icon".to_string());
                            out.push(icon.to_string());
                        }
                    }
                    Some('c') if !quoted => expanded.push_str(name),
                    Some('k') if !quoted => expanded.push_str(&path.to_string_lossy()),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            } else {
                expanded.push(c);
            }
        }
        if !ok {
            return None;
        }
        // `%i` outside quotes appends its own argv; the remainder stays.
        let ends_icon = arg == "%i" && !quoted;
        if !expanded.is_empty() || !ends_icon {
            out.push(expanded);
        }
    }
    Some(out)
}

fn scan_one_file(path: &Path, id: &str, chain: &[String], out: &mut Vec<Entry>) {
    let Ok(meta) = fs::metadata(path) else { return };
    if !meta.is_file() || meta.len() > MAX_FILE {
        return;
    }
    let Ok(bytes) = fs::read(path) else { return };
    let Some(groups) = parse_file(&bytes) else {
        return;
    };
    let entry = &groups.entry;
    if raw(entry, "Type").as_deref() != Some("Application") {
        return;
    }
    if raw(entry, "Hidden").as_deref() == Some("true") {
        return; // strictly equivalent to not existing
    }
    let name = localized(entry, "Name", chain).unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.is_empty() {
        let show: Vec<String> = raw(entry, "OnlyShowIn")
            .map(|s| s.split(';').map(str::to_string).collect())
            .unwrap_or_default();
        if !show.is_empty() && !desktop.split(':').any(|d| show.iter().any(|s| s == d)) {
            out.push(hidden_entry(id, &name, path, "OnlyShowIn"));
            return;
        }
        let hide: Vec<String> = raw(entry, "NotShowIn")
            .map(|s| s.split(';').map(str::to_string).collect())
            .unwrap_or_default();
        if desktop.split(':').any(|d| hide.iter().any(|s| s == d)) {
            out.push(hidden_entry(id, &name, path, "NotShowIn"));
            return;
        }
    }
    if let Some(try_exec) = raw(entry, "TryExec") {
        let exe = try_exec.split_whitespace().next().unwrap_or("");
        if !which(exe) {
            out.push(hidden_entry(id, &name, path, "TryExec missing"));
            return;
        }
    }
    let nodisplay = raw(entry, "NoDisplay").as_deref() == Some("true");
    let exec_raw = raw(entry, "Exec").unwrap_or_default();
    let icon = raw(entry, "Icon").unwrap_or_default();
    out.push(Entry {
        id: id.to_string(),
        name,
        generic: localized(entry, "GenericName", chain).unwrap_or_default(),
        keywords: raw(entry, "Keywords")
            .map(|k| split_list(&k).join(" "))
            .unwrap_or_default(),
        exec: exec_raw,
        icon,
        terminal: raw(entry, "Terminal").as_deref() == Some("true"),
        hidden: nodisplay.then(|| "NoDisplay".to_string()),
        path: path.to_path_buf(),
    });
}

fn hidden_entry(id: &str, name: &str, path: &Path, reason: &str) -> Entry {
    Entry {
        id: id.to_string(),
        name: name.to_string(),
        generic: String::new(),
        keywords: String::new(),
        exec: String::new(),
        icon: String::new(),
        terminal: false,
        hidden: Some(reason.to_string()),
        path: path.to_path_buf(),
    }
}

fn walk(dir: &Path, apps: &Path, chain: &[String], seen: &mut HashSet<String>, out: &mut Vec<Entry>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut names: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    names.sort();
    for path in names {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let Ok(rel) = path.strip_prefix(apps) else { continue };
        let rel = rel.to_string_lossy().replace('/', "-");
        if path.is_dir() {
            if rel.matches('-').count() < 8 {
                walk(&path, apps, chain, seen, out);
            }
            continue;
        }
        if path.extension().map(|e| e.as_bytes()) != Some(b"desktop") {
            continue;
        }
        if !seen.insert(rel.clone()) {
            continue; // first dir in precedence wins
        }
        scan_one_file(&path, &rel, chain, out);
    }
}

pub fn scan(extra_dirs: &[PathBuf]) -> Vec<Entry> {
    let chain = locale_chain();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut dirs = data_dirs();
    for extra in extra_dirs {
        dirs.insert(0, extra.clone());
    }
    for dir in &dirs {
        let apps = dir.join("applications");
        walk(&apps, &apps, &chain, &mut seen, &mut out);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Pad the visible index with synthetic clones so the latency rows can run
/// at 300 and 3,000 even on a system with few real entries. Reported as
/// synthetic wherever it matters.
pub fn pad(entries: &mut Vec<Entry>, target: usize) {
    if entries.len() >= target {
        entries.truncate(target);
        return;
    }
    let real: Vec<Entry> = entries
        .iter()
        .filter(|e| e.hidden.is_none())
        .take(50)
        .map(clone_entry)
        .collect();
    let mut i = 0;
    while entries.len() < target && !real.is_empty() {
        let mut e = clone_entry(&real[i % real.len()]);
        e.id = format!("{}-pad{i}", e.id);
        e.name = format!("{} (pad {i})", real[i % real.len()].name);
        entries.push(e);
        i += 1;
    }
}

fn clone_entry(e: &Entry) -> Entry {
    Entry {
        id: e.id.clone(),
        name: e.name.clone(),
        generic: e.generic.clone(),
        keywords: e.keywords.clone(),
        exec: e.exec.clone(),
        icon: e.icon.clone(),
        terminal: e.terminal,
        hidden: e.hidden.clone(),
        path: e.path.clone(),
    }
}

pub struct Ranked {
    pub order: Vec<usize>,
}

/// Case-folded substring match: `Name` 0, `GenericName` 100, `Keywords`
/// 200, then Exec basename 300; ties by name. An empty query lists every
/// visible entry by name. No allocation past the returned order.
pub fn rank(entries: &[Entry], query: &str) -> Ranked {
    let q = query.to_lowercase();
    let mut scored: Vec<(u32, usize)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if e.hidden.is_some() {
            continue;
        }
        let score = if q.is_empty() {
            0
        } else if let Some(pos) = e.name.to_lowercase().find(&q) {
            pos as u32
        } else if let Some(pos) = e.generic.to_lowercase().find(&q) {
            100 + pos as u32
        } else if let Some(pos) = e.keywords.to_lowercase().find(&q) {
            200 + pos as u32
        } else if let Some(pos) = e.exec.to_lowercase().find(&q) {
            300 + pos as u32
        } else {
            continue;
        };
        scored.push((score, i));
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| entries[a.1].name.cmp(&entries[b.1].name)));
    Ranked {
        order: scored.into_iter().map(|(_, i)| i).collect(),
    }
}
