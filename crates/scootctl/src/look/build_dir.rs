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
    ("apps", &["helix.toml", "btop.theme", "starship.toml"]),
    ("fcitx5", &["theme.conf"]),
    ("gtk", &["gtk-3.0.css", "gtk-4.0.css"]),
    ("qt", &["scoot-look.conf"]),
];

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
