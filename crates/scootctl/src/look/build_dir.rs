//! The `build --out` directory guard: only our own files may be written.
//!
//! Split out of `apply.rs` (which was already large before the daemon
//! leaves grew it): the foreign-entry walk lives here, so that file
//! shrinks instead of growing.

use std::path::Path;

/// Our output files, the only ones `build --out` may write: the two
/// first-party fragments, the toolkit fragments, foot's `foot.ini`, the
/// launcher and locker leaves, the greeter fragment (`greet.toml`: the
/// login screen reads it at system build time, never live), the launcher
/// fragment (`launch.toml`: the panel reads it at every open), the
/// notification fragment (`notify.toml`: the daemon reads it at start
/// and on re-theme), the
/// render record, and the daemon leaves. `foot.ini` joins them only when
/// the look has a `[terminal]` table, and `nightlight` only when the look
/// names a temperature; `fcitx5/theme.conf` lives under its directory,
/// the terminal static leaves under `apps/`.
/// `qt6ct.conf`, `settings.ini` and the fontconfig file stay
/// apply-rendered (the conf points through the state link, which `build`
/// does not know).
const BUILD_FILES: [&str; 15] = [
    "scoot.toml",
    "bar.toml",
    "foot.ini",
    "fuzzel.ini",
    "swaylock",
    "greet.toml",
    "launch.toml",
    "notify.toml",
    "render.json",
    "gtk/gtk-3.0.css",
    "gtk/gtk-4.0.css",
    "qt/scoot-look.conf",
    "mako",
    "wob.ini",
    "nightlight",
];
/// Output directories holding only their named files (never followed
/// through a symlink).
const BUILD_DIRS: [(&str, &[&str]); 4] = [
    (
        "apps",
        &["helix.toml", "btop.theme", "starship.toml", "nvim.lua"],
    ),
    ("fcitx5", &["theme.conf"]),
    ("gtk", &["gtk-3.0.css", "gtk-4.0.css"]),
    ("qt", &["scoot-look.conf"]),
];

/// Whether `rel` (a `/`-separated path under `out`) is one our build may
/// write: one of [`BUILD_FILES`], a file under a [`BUILD_DIRS`] entry, or
/// anything under `targets/` (user targets mirror their declared outputs
/// there: `targets/kitty/colors.conf`). Every segment is still a plain
/// relative name — no `..`, no absolute, no hidden — and symlinks refuse
/// below, so a hostile `targets/` entry cannot escape `--out`.
fn is_build_output(rel: &str) -> bool {
    if BUILD_FILES.contains(&rel) {
        return true;
    }
    let mut parts = rel.split('/');
    let Some(first) = parts.next() else {
        return false;
    };
    if first == "targets" {
        return rel
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    }
    match BUILD_DIRS.iter().find(|(dir, _)| *dir == first) {
        Some((_, allowed)) => {
            let rest: Vec<&str> = parts.collect();
            rest.len() == 1 && allowed.contains(&rest[0])
        }
        None => false,
    }
}

/// Refuses a symlinked `out` (writing through it would land outside the
/// named directory) and a non-empty foreign dir (only an absent dir, an
/// empty dir, or one holding only our own files is written into). That
/// keeps `--out .` from wiping the cwd. Creates an absent dir.
pub(crate) fn check_out_dir(out: &Path) -> Result<(), String> {
    // Refuse a symlinked `out`: writing through it would land outside the
    // named directory.
    if let Ok(meta) = std::fs::symlink_metadata(out)
        && meta.file_type().is_symlink()
    {
        return Err(format!(
            "refusing to build into {}: it is a symlink",
            out.display()
        ));
    }
    if out.exists() && !out.is_dir() {
        return Err(format!(
            "refusing to build into {}: not a directory",
            out.display()
        ));
    }
    if out.is_dir() {
        let entries = std::fs::read_dir(out)
            .map_err(|error| format!("cannot list {}: {error}", out.display()))?;
        let mut foreign: Vec<String> = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            // A symlinked output entry would be written through: refuse
            // it, whatever it is named.
            let fp = out.join(&entry_name);
            if let Ok(meta) = std::fs::symlink_metadata(&fp)
                && meta.file_type().is_symlink()
            {
                return Err(format!(
                    "refusing to build into {}: {} is a symlink",
                    out.display(),
                    fp.display()
                ));
            }
            if BUILD_FILES.contains(&entry_name.as_str()) {
                continue;
            }
            if entry_name == "targets" {
                // User targets mirror their declared outputs here: walk
                // the subtree and refuse anything this build could not
                // have written (foreign files, symlinks, `..` escapes —
                // the last cannot arrive through the renderer, but the
                // guard checks what is on disk, not how it got there).
                if !fp.is_dir() {
                    foreign.push(entry_name.clone());
                    continue;
                }
                let mut stack = vec![fp.clone()];
                while let Some(dir) = stack.pop() {
                    let Ok(inner) = std::fs::read_dir(&dir) else {
                        foreign.push(
                            dir.strip_prefix(out)
                                .map(|rel| rel.display().to_string())
                                .unwrap_or_else(|_| dir.display().to_string()),
                        );
                        continue;
                    };
                    for child in inner.filter_map(|e| e.ok()) {
                        let child_path = child.path();
                        let rel = child_path
                            .strip_prefix(out)
                            .map(|rel| rel.display().to_string())
                            .unwrap_or_else(|_| child_path.display().to_string());
                        let child_symlink = std::fs::symlink_metadata(&child_path)
                            .map(|meta| meta.file_type().is_symlink())
                            .unwrap_or(false);
                        if child_symlink || !is_build_output(&rel) {
                            foreign.push(rel);
                            continue;
                        }
                        if child_path.is_dir() {
                            stack.push(child_path);
                        }
                    }
                }
                continue;
            }
            match BUILD_DIRS.iter().find(|(dir, _)| *dir == entry_name) {
                Some((_, allowed)) => {
                    if !fp.is_dir() {
                        foreign.push(entry_name.clone());
                        continue;
                    }
                    let inner = std::fs::read_dir(&fp)
                        .map_err(|error| format!("cannot list {}: {error}", fp.display()))?;
                    for child in inner.filter_map(|e| e.ok()) {
                        let child_name = child.file_name().to_string_lossy().into_owned();
                        let child_path = fp.join(&child_name);
                        let child_symlink = std::fs::symlink_metadata(&child_path)
                            .map(|meta| meta.file_type().is_symlink())
                            .unwrap_or(false);
                        if !allowed.contains(&child_name.as_str()) || child_symlink {
                            foreign.push(format!("{entry_name}/{child_name}"));
                        }
                    }
                }
                None => foreign.push(entry_name.clone()),
            }
        }
        if !foreign.is_empty() {
            foreign.sort();
            return Err(format!(
                "refusing to build into {}: it holds files this build did not write ({})",
                out.display(),
                foreign.join(", ")
            ));
        }
    } else {
        std::fs::create_dir_all(out)
            .map_err(|error| format!("cannot create {}: {error}", out.display()))?;
    }
    Ok(())
}
