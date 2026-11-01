//! The look layer: precedence, the derived pair, refusals, the feature
//! drop, the scoped reload's snapshot, and the fixed layer.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::*;

/// A scratch directory, removed on drop. Tests run one per process under
/// nextest, so a pid-suffixed directory cannot collide; under `cargo
/// test` threads share the process, but each test takes its own
/// subdirectory. No new dependency for what `std` already does.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!(
            "/tmp/opencode/scootbar-look-test-{tag}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn value(text: &str) -> toml::Value {
    toml::from_str(text).expect("test TOML parses")
}

fn origin() -> Origin {
    Origin::State(PathBuf::from(
        "/tmp/opencode/scootbar-look-test-layer/bar.toml",
    ))
}

/// Commits a successful reload, as the daemon does after its apply
/// succeeds: the look functions only compute, they never move state. A
/// refused apply drops the pending value and the snapshot stays.
fn commit(look: &mut LookState, pending: Reloaded) -> (Config, Vec<String>) {
    let Reloaded {
        config,
        notes,
        look: next,
    } = pending;
    *look = next;
    (config, notes)
}

const FULL: Features = Features::all();

/// No state dir: fixed-layer tests prove the state link is never read.
const NO_STATE: Option<&Path> = None;

/// Any module id this build placed (config parsing is static: no
/// compositor needed). `None` in a build with no module at all.
fn any_module() -> Option<&'static str> {
    crate::modules::REGISTRY.first().map(|spec| spec.id)
}

#[test]
fn per_key_precedence() {
    let user = value("[colors]\nbackground = \"#111111\"\n[bar]\nradius = 3\n");
    let layer = value(
        "[colors]\nbackground = \"#222222\"\nforeground = \"#333333\"\naccent = \"#444444\"\n[bar]\nradius = 9\nopacity = 0.5\n",
    );
    let (merged, _, notes) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    assert!(notes.is_empty(), "notes: {notes:?}");
    assert_eq!(merged["colors"]["background"].as_str(), Some("#111111"));
    assert_eq!(merged["colors"]["foreground"].as_str(), Some("#333333"));
    assert_eq!(merged["colors"]["accent"].as_str(), Some("#444444"));
    assert_eq!(merged["bar"]["radius"].as_integer(), Some(3));
    assert_eq!(merged["bar"]["opacity"].as_float(), Some(0.5));
}

#[test]
fn user_file_wins_over_the_look_for_depth() {
    use scootui::effects::{FillStyle, ShadowStyle};
    // The user's file wins per key over the look layer for the depth
    // tokens together, and the look fills the keys the file leaves alone.
    let user = value("[bar]\nshadow = \"soft\"\n");
    let layer = value("[bar]\nshadow = \"deep\"\nfill = \"gradient\"\n");
    let (merged, _, notes) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    assert!(notes.is_empty(), "notes: {notes:?}");
    assert_eq!(merged["bar"]["shadow"].as_str(), Some("soft"));
    assert_eq!(merged["bar"]["fill"].as_str(), Some("gradient"));
    let config = config::config_from_value(merged, Path::new("bar.toml")).expect("merged parses");
    assert_eq!(config.shadow, ShadowStyle::Soft);
    assert_eq!(config.fill, FillStyle::Gradient);
    // The other way: the look's shadow stands where the file is silent,
    // while the file's fill still wins its own key.
    let user = value("[bar]\nfill = \"gradient\"\n");
    let layer = value("[bar]\nshadow = \"glow\"\nfill = \"solid\"\n");
    let (merged, _, _) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    let config = config::config_from_value(merged, Path::new("bar.toml")).expect("merged parses");
    assert_eq!(config.shadow, ShadowStyle::Glow);
    assert_eq!(config.fill, FillStyle::Gradient);
}

#[test]
fn user_accent_alone_drives_hover() {
    // The user's `accent` if set, without their `hover`: the look's
    // `hover` is dropped so the bar derives hover from the user's accent.
    let user = value("[colors]\naccent = \"#111111\"\n");
    let layer = value("[colors]\naccent = \"#222222\"\nhover = \"#333333\"\n");
    let (merged, _, _) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    assert!(merged.get("colors").and_then(|c| c.get("hover")).is_none());
    let config = config::config_from_value(merged, Path::new("bar.toml")).expect("merged parses");
    assert_eq!(config.theme.accent.to_string(), "#111111");
    assert_eq!(
        config.theme.hover.to_string(),
        "#111111",
        "hover follows the user's accent, not the look's hover"
    );
}

#[test]
fn user_hover_alone_keeps_the_look_accent() {
    // The user's `hover` if set: the look's `accent` stays (never the
    // built-in yellow).
    let user = value("[colors]\nhover = \"#444444\"\n");
    let layer = value("[colors]\naccent = \"#222222\"\nhover = \"#333333\"\n");
    let (merged, _, _) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    let config = config::config_from_value(merged, Path::new("bar.toml")).expect("merged parses");
    assert_eq!(config.theme.hover.to_string(), "#444444");
    assert_eq!(
        config.theme.accent.to_string(),
        "#222222",
        "the look's accent stays, never the built-in yellow"
    );
}

#[test]
fn look_hover_stands_with_neither_user_key() {
    let user = value("[colors]\nbackground = \"#111111\"\n");
    let layer = value("[colors]\naccent = \"#222222\"\nhover = \"#333333\"\n");
    let (merged, _, _) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    let config = config::config_from_value(merged, Path::new("bar.toml")).expect("merged parses");
    assert_eq!(config.theme.hover.to_string(), "#333333");
}

#[test]
fn forbidden_keys_refuse_the_layer() {
    // Geometry, placement, module lists, commands and bindings: a look
    // never touches them, so any of them refuses the whole layer.
    for layer in [
        "[bar]\nedge = \"top\"\n",
        "[bar]\nheight = 40\n",
        "[bar]\ntooltip-delay = 100\n",
        "left = [\"clock\"]\n",
        "[exec.weather]\ncommand = [\"weather\"]\n",
        "[clock]\non-click = \"popup\"\n",
    ] {
        let error =
            merge(&value(""), &value(layer), &FULL, &origin()).expect_err("forbidden layer passes");
        assert!(
            error.contains("not a look key"),
            "refusal names the rule: {error}"
        );
    }
}

#[test]
fn bad_layer_values_refuse_the_layer() {
    for (key, layer) in [
        ("colors.accent", "[colors]\naccent = \"notacolor\"\n"),
        ("bar.radius", "[bar]\nradius = 99999\n"),
        (
            "workspaces.pill-shape",
            "[workspaces]\npill-shape = \"hexagon\"\n",
        ),
    ] {
        let error =
            merge(&value(""), &value(layer), &FULL, &origin()).expect_err("bad layer value passes");
        assert!(error.contains(key), "refusal names {key}: {error}");
    }
}

#[test]
fn gated_keys_drop_with_a_note() {
    // A key whose feature is not compiled in leaves the layer with a
    // note; the rest still applies.
    let user = value("");
    let layer = value("[colors]\naccent = \"#222222\"\n[bar]\npopup-radius = 6\n");
    let features = Features {
        popup: false,
        ..FULL
    };
    let (merged, _, notes) = merge(&user, &layer, &features, &origin()).expect("merges");
    assert_eq!(
        merged["colors"]["accent"].as_str(),
        Some("#222222"),
        "the gated key's neighbors still apply"
    );
    assert!(
        merged
            .get("bar")
            .and_then(|bar| bar.get("popup-radius"))
            .is_none(),
        "popup-radius applied without the popup feature"
    );
    assert_eq!(notes.len(), 1, "one note, naming the key: {notes:?}");
    assert!(
        notes[0].contains("bar.popup-radius") && notes[0].contains("popup"),
        "note names the key and the feature: {}",
        notes[0]
    );
    // The user's own file keeps `deny_unknown_fields`: no such mercy
    // there. (This parses through the crate's own strict path.)
    let scratch = Scratch::new("deny");
    let path = scratch.write("bar.toml", "[bar]\npopup-radius = 6\n");
    #[cfg(feature = "popup")]
    {
        let _ = config::read_value(&path).expect("reads");
    }
    #[cfg(not(feature = "popup"))]
    {
        let error = config::read_value(&path)
            .and_then(|v| config::config_from_value(v, &path))
            .expect_err("user popup-radius passes without the feature");
        assert!(error.to_string().contains("popup-radius"), "{error}");
    }
}

#[test]
fn module_keys_gate_on_their_module() {
    let user = value("");
    let layer = value("[workspaces]\ninactive-color = \"#222222\"\n");
    let features = Features {
        workspaces: false,
        ..FULL
    };
    let (merged, _, notes) = merge(&user, &layer, &features, &origin()).expect("merges");
    assert!(
        merged
            .get("workspaces")
            .and_then(|w| w.get("inactive-color"))
            .is_none()
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("workspaces.inactive-color")),
        "{notes:?}"
    );
}

#[test]
fn scoped_reload_ignores_pending_edits() {
    let scratch = Scratch::new("scoped");
    // The fixed layer: the same merge as the state link, with no
    // environment in the path.
    let layer = scratch.write(
        "one.toml",
        "[colors]\naccent = \"#222222\"\nhover = \"#333333\"\n",
    );
    let user = scratch.write(
        "bar.toml",
        &format!(
            "[colors]\nbackground = \"#111111\"\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    );
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    assert_eq!(startup.config.theme.accent.to_string(), "#222222");
    let mut look = startup.look;
    // Pending edits: the user recolors the background and rewrites
    // the layer's accent meanwhile.
    std::fs::write(
        &user,
        format!(
            "[colors]\nbackground = \"#aaaaaa\"\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &layer,
        "[colors]\naccent = \"#bbbbbb\"\nhover = \"#cccccc\"\n",
    )
    .unwrap();
    // Scoped: the new layer under the snapshot. The background stays
    // the snapshot's; the accent follows the layer.
    let pending = reload_look(Some(&user), &look, &FULL, NO_STATE).expect("scoped reloads");
    let (config, _) = commit(&mut look, pending);
    assert_eq!(config.theme.background.to_string(), "#111111");
    assert_eq!(config.theme.accent.to_string(), "#bbbbbb");
    // Plain: the pending edits land.
    let pending = reload_user(Some(&user), &look, &FULL, NO_STATE).expect("plain reloads");
    let (config, _) = commit(&mut look, pending);
    assert_eq!(config.theme.background.to_string(), "#aaaaaa");
    assert_eq!(config.theme.accent.to_string(), "#bbbbbb");
}

#[test]
fn scoped_reload_keeps_a_pending_module_list() {
    let Some(module) = any_module() else {
        return;
    };
    let scratch = Scratch::new("scoped-list");
    let layer = scratch.write("one.toml", "[colors]\naccent = \"#222222\"\n");
    let user = scratch.write(
        "bar.toml",
        &format!(
            "left = [\"{module}\"]\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    );
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    assert_eq!(startup.config.layout.left, vec![module]);
    let mut look = startup.look;
    // The user empties the list meanwhile: a half-edited file.
    std::fs::write(
        &user,
        format!("left = []\n[look]\nlayer = \"{}\"\n", layer.display()),
    )
    .unwrap();
    let pending = reload_look(Some(&user), &look, &FULL, NO_STATE).expect("scoped reloads");
    let (config, _) = commit(&mut look, pending);
    assert_eq!(
        config.layout.left,
        vec![module],
        "the scoped reload applied a pending module-list edit"
    );
    let pending = reload_user(Some(&user), &look, &FULL, NO_STATE).expect("plain reloads");
    let (config, _) = commit(&mut look, pending);
    assert!(
        config.layout.left.is_empty(),
        "the plain reload did not pick the edit up"
    );
}

#[test]
fn plain_reload_without_a_file_is_a_loud_refusal() {
    // Same refusal as before the layer: no usable config home means no
    // reload, whatever the look state.
    let look = LookState::new(value(""), false);
    let error = reload_user(None, &look, &FULL, NO_STATE).expect_err("reloads without a file");
    assert!(error.contains("no config file to reload"), "{error}");
}

#[test]
fn fixed_layer_resolves_relative_to_the_file() {
    let scratch = Scratch::new("fixed-rel");
    scratch.write("looks/bar.toml", "[colors]\naccent = \"#222222\"\n");
    let user = scratch.write("bar.toml", "[look]\nlayer = \"looks/bar.toml\"\n");
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    assert_eq!(startup.config.theme.accent.to_string(), "#222222");
}

#[test]
fn state_link_is_the_live_choice() {
    let scratch = Scratch::new("state");
    let state = scratch.path.join("state");
    let look = state.join("look");
    std::fs::create_dir_all(&look).unwrap();
    std::fs::write(look.join("bar.toml"), "[colors]\naccent = \"#222222\"\n").unwrap();
    let user = scratch.write("bar.toml", "[colors]\nbackground = \"#111111\"\n");
    let startup = load_startup(Some(&user), Some(&state)).expect("starts");
    assert_eq!(startup.config.theme.background.to_string(), "#111111");
    assert_eq!(startup.config.theme.accent.to_string(), "#222222");
}

#[test]
fn missing_state_link_is_silent_without_a_layer() {
    let scratch = Scratch::new("no-link");
    let state = scratch.path.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let user = scratch.write("bar.toml", "[colors]\nbackground = \"#111111\"\n");
    let startup = load_startup(Some(&user), Some(&state)).expect("starts");
    assert_eq!(
        startup.config.theme.accent.to_string(),
        Config::default().theme.accent.to_string()
    );
    assert!(startup.notes.is_empty(), "{:?}", startup.notes);
}

#[test]
fn dangling_state_link_starts_without_a_layer_and_refuses_scoped() {
    let scratch = Scratch::new("dangling");
    let state = scratch.path.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let look = state.join("look");
    let target = scratch.path.join("render");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("bar.toml"), "[colors]\naccent = \"#222222\"\n").unwrap();
    std::os::unix::fs::symlink(&target, &look).unwrap();
    let user = scratch.write("bar.toml", "[colors]\nbackground = \"#111111\"\n");
    // Through the link: the layer applies.
    let startup = load_startup(Some(&user), Some(&state)).expect("starts");
    assert_eq!(startup.config.theme.accent.to_string(), "#222222");
    // The target goes away: the link dangles. A start still works,
    // without the layer and with a warning; either reload refuses.
    std::fs::remove_dir_all(&target).unwrap();
    let startup = load_startup(Some(&user), Some(&state)).expect("starts over dangling");
    assert!(
        startup.notes.iter().any(|note| note.contains("dangling")),
        "{:?}",
        startup.notes
    );
    assert_eq!(
        startup.config.theme.accent.to_string(),
        Config::default().theme.accent.to_string()
    );
    let look_state = startup.look;
    let error = reload_look(Some(&user), &look_state, &FULL, Some(&state))
        .expect_err("scoped over dangling");
    assert!(error.contains("dangling"), "{error}");
}

#[test]
fn layer_font_that_fails_drops_its_keys_at_start() {
    // A layer font that fails to load drops the layer's font keys at
    // start-up (the bar falls back to its well-known paths) instead of
    // refusing the start. A reload refuses instead, with the running bar
    // kept: that path is `Responder::stage`'s, already tested.
    let scratch = Scratch::new("font-probe");
    let layer = scratch.write(
        "one.toml",
        "[colors]\naccent = \"#222222\"\n[bar]\nfont = \"/nonexistent/DejaVuSans.ttf\"\n",
    );
    let placed = any_module().map(|module| format!("left = [\"{module}\"]\n"));
    let user = scratch.write(
        "bar.toml",
        &format!(
            "{}[look]\nlayer = \"{}\"\n",
            placed.as_deref().unwrap_or(""),
            layer.display()
        ),
    );
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    assert_eq!(startup.config.theme.accent.to_string(), "#222222");
    if any_module().is_some() {
        assert!(
            startup
                .notes
                .iter()
                .any(|note| note.contains("font fails to load")),
            "{:?}",
            startup.notes
        );
        assert_eq!(startup.config.font, None);
    } else {
        // Nothing placed: no font loads either way, so the key rides on.
        assert_eq!(
            startup.config.font,
            Some(PathBuf::from("/nonexistent/DejaVuSans.ttf"))
        );
    }
}

#[test]
fn malformed_layer_refuses_without_panic() {
    // Not TOML at all, and valid TOML that is hostile: refused by name,
    // never a panic, on either reload path.
    let scratch = Scratch::new("malformed");
    let layer = scratch.write("one.toml", "[colors]\naccent = \"#222222\"\n");
    let user = scratch.write(
        "bar.toml",
        &format!(
            "[colors]\nbackground = \"#111111\"\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    );
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    let look = startup.look;
    for garbage in ["[[[", "\x00", "x = ", "[colors]\naccent = 5\n"] {
        std::fs::write(&layer, garbage).unwrap();
        let error = reload_look(Some(&user), &look, &FULL, NO_STATE).expect_err("garbage passes");
        assert!(!error.is_empty(), "empty refusal for {garbage:?}");
        let error = reload_user(Some(&user), &look, &FULL, NO_STATE).expect_err("garbage passes");
        assert!(!error.is_empty(), "empty refusal for {garbage:?}");
    }
}

#[test]
fn broken_layer_refuses_and_keeps_the_snapshot() {
    let scratch = Scratch::new("broken");
    let layer = scratch.write("one.toml", "[colors]\naccent = \"#222222\"\n");
    let user = scratch.write(
        "bar.toml",
        &format!(
            "[colors]\nbackground = \"#111111\"\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    );
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    let mut look = startup.look;
    std::fs::write(&layer, "[bar]\nedge = \"top\"\n").unwrap();
    let error = reload_look(Some(&user), &look, &FULL, NO_STATE).expect_err("broken layer passes");
    assert!(error.contains("not a look key"), "{error}");
    // The snapshot still holds the old file: a fixed layer heals the
    // next scoped reload.
    std::fs::write(&layer, "[colors]\naccent = \"#444444\"\n").unwrap();
    let pending = reload_look(Some(&user), &look, &FULL, NO_STATE).expect("heals");
    let (config, _) = commit(&mut look, pending);
    assert_eq!(config.theme.background.to_string(), "#111111");
    assert_eq!(config.theme.accent.to_string(), "#444444");
}

#[test]
fn state_dir_resolution_prefers_the_session() {
    assert_eq!(
        state_dir_from(
            Some(OsStr::new("/tmp/opencode/session-state")),
            Some(OsStr::new("/tmp/opencode/xdg-state")),
            Some(OsStr::new("/tmp/opencode/home-state")),
        ),
        Some(PathBuf::from("/tmp/opencode/session-state"))
    );
    assert_eq!(
        state_dir_from(
            None,
            Some(OsStr::new("/tmp/opencode/xdg-state")),
            Some(OsStr::new("/tmp/opencode/home-state")),
        ),
        Some(PathBuf::from("/tmp/opencode/xdg-state/scoot"))
    );
    assert_eq!(
        state_dir_from(None, None, Some(OsStr::new("/tmp/opencode/home-state")),),
        Some(PathBuf::from("/tmp/opencode/home-state/.local/state/scoot"))
    );
    assert_eq!(state_dir_from(None, None, None), None);
    assert_eq!(
        state_dir_from(Some(OsStr::new("")), None, None),
        None,
        "empty is unset, as in libwayland"
    );
}

#[test]
fn empty_fixed_layer_is_a_loud_user_error() {
    let scratch = Scratch::new("empty-fixed");
    let user = scratch.write("bar.toml", "[look]\nlayer = \"\"\n");
    let error = load_startup(Some(&user), NO_STATE).expect_err("empty layer passes");
    assert!(error.to_string().contains("look.layer"), "{error}");
}

#[test]
fn renderer_font_rides_the_layer() {
    // `bar.font` is a fragment key, never an author override: the loader
    // takes the renderer's absolute path, and refuses a relative one.
    let user = value("");
    let layer = value("[bar]\nfont = \"/fonts/DejaVuSans.ttf\"\n");
    let (merged, _, _) = merge(&user, &layer, &FULL, &origin()).expect("merges");
    assert_eq!(
        merged["bar"]["font"].as_str(),
        Some("/fonts/DejaVuSans.ttf")
    );
    let layer = value("[bar]\nfont = \"relative.ttf\"\n");
    let error = merge(&user, &layer, &FULL, &origin()).expect_err("relative font passes");
    assert!(error.contains("bar.font"), "{error}");
}

#[test]
fn an_unapplied_plain_reload_leaves_the_snapshot() {
    // Valid at parse but refused at apply (a flag clash, a font that
    // vanished in `stage`): the snapshot must not move until the apply
    // succeeds, or a later scoped reload merges the new layer under edits
    // that never applied.
    let scratch = Scratch::new("stale-snapshot");
    let layer = scratch.write("one.toml", "[colors]\naccent = \"#222222\"\n");
    let wrote = |background: &str| {
        std::fs::write(
            scratch.path.join("bar.toml"),
            format!(
                "[colors]\nbackground = \"{background}\"\n[look]\nlayer = \"{}\"\n",
                layer.display()
            ),
        )
        .unwrap()
    };
    wrote("#111111");
    let user = scratch.path.join("bar.toml");
    let startup = load_startup(Some(&user), NO_STATE).expect("starts");
    let look = startup.look;
    // The edit parses (a scoped-apply refusal happens later, in the
    // daemon): the caller drops it, as a refused apply would, without
    // committing.
    wrote("#aaaaaa");
    let _ = reload_user(Some(&user), &look, &FULL, NO_STATE).expect("parses");
    assert_eq!(
        look.snapshot["colors"]["background"].as_str(),
        Some("#111111"),
        "computing a reload must not move the snapshot"
    );
    // The apply refused: the file is reverted.
    wrote("#111111");
    // The scoped reload must see the reverted file, not the edit that
    // never applied.
    let pending = reload_look(Some(&user), &look, &FULL, NO_STATE).expect("scoped reloads");
    assert_eq!(
        pending.config.theme.background.to_string(),
        "#111111",
        "the reverted file must win over the unapplied edit"
    );
}

#[test]
fn plain_reload_over_a_lost_layer_applies_the_edits() {
    // The state link is gone: a plain reload still applies the user's
    // edits, degrading to no layer with a loud note and clearing
    // `active`. Only the look-scoped reload stays a refusal.
    let scratch = Scratch::new("lost-applies");
    let state = scratch.path.join("state");
    let look = state.join("look");
    std::fs::create_dir_all(&look).unwrap();
    std::fs::write(look.join("bar.toml"), "[colors]\naccent = \"#222222\"\n").unwrap();
    let user = scratch.write("bar.toml", "[colors]\nbackground = \"#111111\"\n");
    let startup = load_startup(Some(&user), Some(&state)).expect("starts");
    let mut look_state = startup.look;
    assert!(look_state.active, "the layer applied at start");
    // The link goes away; the user edits meanwhile.
    std::fs::remove_file(look.join("bar.toml")).unwrap();
    std::fs::write(&user, "[colors]\nbackground = \"#aaaaaa\"\n").unwrap();
    let pending = reload_user(Some(&user), &look_state, &FULL, Some(&state))
        .expect("plain applies the edits");
    assert_eq!(pending.config.theme.background.to_string(), "#aaaaaa");
    assert_eq!(
        pending.config.theme.accent.to_string(),
        Config::default().theme.accent.to_string(),
        "no layer: the accent is the default, not the lost layer's"
    );
    assert!(
        pending
            .notes
            .iter()
            .any(|note| note.contains("without a look layer")),
        "the loss is loud: {:?}",
        pending.notes
    );
    commit(&mut look_state, pending);
    assert!(!look_state.active, "active is cleared");
}

#[test]
fn plain_reload_over_a_dangling_link_applies_the_edits() {
    // Same as above, but the link dangles (its target went away) while
    // the layer was active.
    let scratch = Scratch::new("dangling-active");
    let state = scratch.path.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let look = state.join("look");
    let target = scratch.path.join("render");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("bar.toml"), "[colors]\naccent = \"#222222\"\n").unwrap();
    std::os::unix::fs::symlink(&target, &look).unwrap();
    let user = scratch.write("bar.toml", "[colors]\nbackground = \"#111111\"\n");
    let startup = load_startup(Some(&user), Some(&state)).expect("starts");
    let mut look_state = startup.look;
    assert!(look_state.active, "the layer applied at start");
    std::fs::remove_dir_all(&target).unwrap();
    std::fs::write(&user, "[colors]\nbackground = \"#aaaaaa\"\n").unwrap();
    let pending = reload_user(Some(&user), &look_state, &FULL, Some(&state))
        .expect("plain applies the edits");
    assert_eq!(pending.config.theme.background.to_string(), "#aaaaaa");
    assert!(
        pending.notes.iter().any(|note| note.contains("dangling")),
        "the loss is loud: {:?}",
        pending.notes
    );
    commit(&mut look_state, pending);
    assert!(!look_state.active, "active is cleared");
}

#[test]
fn merged_value_errors_name_the_file_once() {
    // The layer's radius passes the allowlist (0-512) but not the user's
    // own height (at most half of it): the refusal names the file once,
    // not twice.
    let scratch = Scratch::new("double-path");
    let layer = scratch.write("one.toml", "[bar]\nradius = 500\n");
    let user = scratch.write(
        "bar.toml",
        &format!(
            "[bar]\nheight = 24\n[look]\nlayer = \"{}\"\n",
            layer.display()
        ),
    );
    // At startup the whole layer is refused (a note), and the bar starts
    // without it.
    let startup = load_startup(Some(&user), NO_STATE).expect("starts without the layer");
    assert_eq!(
        startup.config.radius, 0,
        "the layer's radius must not apply"
    );
    assert!(
        startup.notes.iter().any(|note| note.contains("bar.radius")),
        "{:?}",
        startup.notes
    );
    let look = startup.look;
    assert!(!look.active);
    // On a swap the whole reload is refused, naming the file once.
    std::fs::write(&layer, "[bar]\nradius = 500\n").unwrap();
    let error =
        reload_look(Some(&user), &look, &FULL, NO_STATE).expect_err("swap refuses the layer");
    assert!(error.contains("bar.radius"), "{error}");
    let needle = layer.display().to_string();
    assert_eq!(
        error.matches(needle.as_str()).count(),
        1,
        "the path is named twice: {error}"
    );
    let error =
        reload_user(Some(&user), &look, &FULL, NO_STATE).expect_err("plain refuses the layer");
    assert_eq!(
        error.matches(needle.as_str()).count(),
        1,
        "the path is named twice: {error}"
    );
}
