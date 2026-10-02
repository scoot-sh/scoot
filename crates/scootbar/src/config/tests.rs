//! The config file: what it takes, and how it refuses.

use std::path::PathBuf;

use super::{Config, Error, MAX_FILE, load_startup, reload};
#[cfg(all(feature = "clock", feature = "workspaces"))]
use crate::bar::Edge;
use crate::bar::Margin;
#[cfg(any(feature = "clock", feature = "workspaces"))]
use crate::layout::Layout;

/// A scratch directory, removed on drop. The tests run one per process
/// (nextest), so a pid-suffixed directory cannot collide; no new
/// dependency for what `std` already does.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(format!(
            "/tmp/opencode/scootbar-config-test-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn file(&self, text: &str) -> PathBuf {
        let path = self.path.join("bar.toml");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(super) fn read(text: &str) -> Result<Config, Error> {
    let scratch = Scratch::new();
    super::read_file(&scratch.file(text))
}

/// A module id this build has, for the layout tests (every feature
/// combination has at least the shape tested: each module alone, both, or
/// neither).
#[cfg(feature = "clock")]
pub(super) const MODULE: &str = "clock";
#[cfg(all(not(feature = "clock"), feature = "workspaces"))]
pub(super) const MODULE: &str = "workspaces";

fn missing_path() -> PathBuf {
    PathBuf::from("/tmp/opencode/scootbar-config-test-no-such-dir/bar.toml")
}

#[test]
fn an_empty_file_is_the_defaults() {
    assert_eq!(read("").unwrap(), Config::default());
    assert_eq!(read("# only a comment\n").unwrap(), Config::default());
}

#[test]
fn an_unset_hover_follows_a_custom_accent() {
    let config = read("[colors]\naccent = \"#89b4fa\"\n").unwrap();
    assert_eq!(config.theme.accent.to_string(), "#89b4fa");
    assert_eq!(
        config.theme.hover.to_string(),
        "#89b4fa",
        "an unset hover follows the accent"
    );
    let config = read("[colors]\naccent = \"#89b4fa\"\nhover = \"#000000\"\n").unwrap();
    assert_eq!(
        config.theme.hover.to_string(),
        "#000000",
        "a set hover wins over the accent"
    );
}

#[test]
#[cfg(all(feature = "clock", feature = "workspaces"))]
fn a_full_file_is_read_whole() {
    let config = read(
        r##"
left = ["workspaces"]
center = ["clock"]
right = []

[bar]
edge = "bottom"
layer = "overlay"
exclusive = false
height = 40
margin = "8,4"
font = "/fonts/DejaVuSans.ttf"
font-size = 13
padding = 4
spacing = 2

[colors]
background = "#101014"
foreground = "#e0e0e0"
accent = "#f9e2af"
hover = "#89b4fa"
dim = "#6c7086"
urgent = "#f38ba8"

[clock]
format = "%H:%M"

[workspaces]
"##,
    )
    .unwrap();
    assert_eq!(config.bar.edge, Edge::Bottom);
    assert_eq!(config.bar.layer, crate::bar::Layer::Overlay);
    assert!(!config.bar.exclusive);
    assert_eq!(config.bar.height, 40);
    assert_eq!(
        config.bar.margin,
        Margin {
            top: 8,
            right: 4,
            bottom: 8,
            left: 4
        }
    );
    assert_eq!(config.font, Some(PathBuf::from("/fonts/DejaVuSans.ttf")));
    assert_eq!(config.font_size, 13);
    assert_eq!(config.layout.padding, 4);
    assert_eq!(config.layout.spacing, 2);
    assert_eq!(config.layout.left, ["workspaces"]);
    assert_eq!(config.layout.center, ["clock"]);
    assert!(config.layout.right.is_empty());
    assert_eq!(config.theme.background.to_string(), "#101014");
    assert_eq!(config.theme.foreground.to_string(), "#e0e0e0");
    assert_eq!(config.theme.accent.to_string(), "#f9e2af");
    assert_eq!(config.theme.hover.to_string(), "#89b4fa");
    assert_eq!(config.theme.dim.to_string(), "#6c7086");
    assert_eq!(config.theme.urgent.to_string(), "#f38ba8");
    #[cfg(feature = "clock")]
    assert_eq!(
        config.modules.clock.format,
        crate::modules::clock::format::Format::parse("%H:%M").unwrap()
    );
}

#[test]
fn a_margin_is_a_number_or_the_shorthand() {
    let margin = |text: &str| read(text).unwrap().bar.margin;
    assert_eq!(margin("[bar]\nmargin = 8\n").top, 8);
    assert_eq!(margin("[bar]\nmargin = 8\n").left, 8);
    assert_eq!(
        margin("[bar]\nmargin = \"8,4\"\n"),
        Margin {
            top: 8,
            right: 4,
            bottom: 8,
            left: 4
        }
    );
}

#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn giving_one_section_sets_the_whole_layout() {
    let layout = read(&format!("left = [\"{MODULE}\"]\n")).unwrap().layout;
    assert_eq!(layout.left, [MODULE]);
    assert!(layout.center.is_empty() && layout.right.is_empty());
    // Nothing given: the defaults.
    assert_eq!(read("").unwrap().layout, Layout::default());
}

#[test]
fn unknown_keys_are_a_loud_error_naming_them() {
    for (text, key) in [
        ("[bar]\nhieght = 28\n", "hieght"),
        ("[colours]\n", "colours"),
        ("[workspaces]\nanything = 1\n", "anything"),
        ("up = 1\n", "up"),
        ("[clock]\nfmt = \"%H\"\n", "fmt"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text}: {error}");
    }
}

#[test]
fn bad_values_name_their_key() {
    let cases = [
        ("[bar]\nedge = \"left\"\n", "bar.edge"),
        ("[bar]\nlayer = \"background\"\n", "bar.layer"),
        ("[bar]\nlayer = 1\n", "layer"),
        ("[bar]\nexclusive = \"yes\"\n", "exclusive"),
        ("[bar]\nheight = 0\n", "bar.height"),
        ("[bar]\nheight = 2048\n", "bar.height"),
        ("[bar]\nheight = \"tall\"\n", "height"),
        ("[bar]\nmargin = \"8,x\"\n", "bar.margin"),
        ("[bar]\nmargin = -1\n", "bar.margin"),
        ("[bar]\nmargin = [1]\n", "bar.margin"),
        ("[bar]\nfont-size = 0\n", "bar.font-size"),
        ("[bar]\npadding = 2048\n", "bar.padding"),
        ("[colors]\nbackground = \"red\"\n", "colors.background"),
        ("[colors]\nhover = \"blue\"\n", "colors.hover"),
        ("[colors]\nurgent = \"#12345\"\n", "colors.urgent"),
        ("left = [\"bluetooth\"]\n", "left"),
    ];
    for (text, key) in cases {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text}: {error}");
    }
}

/// A module placed twice, in one section or across two, is refused naming
/// the section.
#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn a_module_placed_twice_is_refused() {
    for text in [
        format!("left = [\"{MODULE}\", \"{MODULE}\"]\n"),
        format!("left = [\"{MODULE}\"]\nright = [\"{MODULE}\"]\n"),
    ] {
        let error = read(&text).unwrap_err().to_string();
        assert!(error.contains("placed twice"), "{text}: {error}");
    }
}

#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn too_many_modules_are_refused() {
    let mut text = String::from("left = [");
    for n in 0..40 {
        if n > 0 {
            text.push_str(", ");
        }
        text.push('"');
        text.push_str(MODULE);
        text.push('"');
    }
    text.push_str("]\n");
    let error = read(&text).unwrap_err().to_string();
    assert!(error.contains("left"), "{error}");
    assert!(error.contains("at most"), "{error}");
}

#[test]
fn malformed_toml_says_what_and_where() {
    let scratch = Scratch::new();
    let path = scratch.file("[bar\n");
    let error = super::read_file(&path).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(matches!(error, Error::Parse { .. }));
}

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let path = missing_path();
    let error = super::read_file(&path).unwrap_err().to_string();
    assert!(error.contains("no-such-dir"), "{error}");
}

#[test]
fn a_file_past_the_bound_is_refused() {
    let scratch = Scratch::new();
    let path = scratch.path.join("bar.toml");
    std::fs::write(&path, vec![b'#'; MAX_FILE as usize + 1]).unwrap();
    let error = super::read_file(&path).unwrap_err();
    assert!(matches!(error, Error::TooLarge { .. }), "{error:?}");
}

#[test]
fn an_explicit_missing_file_is_a_refusal() {
    assert!(load_startup(Some(&missing_path())).is_err());
}

#[test]
fn reload_without_a_file_is_a_refusal() {
    assert!(reload(None).is_err());
}

#[test]
#[cfg(feature = "clock")]
fn clock_format_bounds_hold() {
    // Longer than the clock takes: refused naming the key.
    let long = "x".repeat(300);
    let error = read(&format!("[clock]\nformat = \"{long}\"\n"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("clock.format"), "{error}");
    let error = read("[clock]\nformat = \"%Q\"\n").unwrap_err().to_string();
    assert!(error.contains("clock.format"), "{error}");
}

#[test]
fn the_default_path_follows_its_homes() {
    // Whatever this machine's homes are, the path ends in the file.
    if let Some(path) = super::default_path() {
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("bar.toml")
        );
        assert!(path.parent().is_some_and(|dir| dir.ends_with("scoot")));
    }
}

#[test]
fn a_radius_and_an_opacity_default_to_square_and_opaque() {
    let config = read("").unwrap();
    assert_eq!((config.radius, config.opacity), (0, 255));
    let config = read("[bar]\nradius = 12\nopacity = 0.5\n").unwrap();
    assert_eq!((config.radius, config.opacity), (12, 128));
    let style = config.style();
    assert_eq!((style.radius, style.opacity), (12, 128));
    let opacity = |text: &str| read(&format!("[bar]\nopacity = {text}\n")).map(|c| c.opacity);
    assert_eq!(opacity("1").unwrap(), 255);
    assert_eq!(opacity("1.0").unwrap(), 255);
    assert_eq!(opacity("0").unwrap(), 0);
    assert_eq!(opacity("0.0").unwrap(), 0);
    assert_eq!(opacity("0.9").unwrap(), 230);
}

#[test]
fn a_radius_is_at_most_half_the_height() {
    // The default height is 28: 14 is a pill, 15 is refused.
    assert_eq!(read("[bar]\nradius = 14\n").unwrap().radius, 14);
    let error = read("[bar]\nradius = 15\n").unwrap_err().to_string();
    assert!(
        error.contains("'bar.radius'") && error.contains("(14)"),
        "{error}"
    );
    assert_eq!(
        read("[bar]\nheight = 40\nradius = 20\n").unwrap().radius,
        20
    );
    assert!(read("[bar]\nheight = 40\nradius = 21\n").is_err());
    // The most any bar has.
    assert!(read("[bar]\nheight = 1024\nradius = 512\n").is_ok());
    assert!(read("[bar]\nheight = 1024\nradius = 513\n").is_err());
}

#[test]
fn a_bad_radius_or_opacity_is_refused_naming_its_key() {
    for (text, key) in [
        ("[bar]\nradius = -1\n", "bar.radius"),
        ("[bar]\nradius = \"8\"\n", "bar.radius"),
        ("[bar]\nradius = 1.5\n", "bar.radius"),
        ("[bar]\nopacity = 1.1\n", "bar.opacity"),
        ("[bar]\nopacity = -0.1\n", "bar.opacity"),
        ("[bar]\nopacity = 2\n", "bar.opacity"),
        ("[bar]\nopacity = nan\n", "bar.opacity"),
        ("[bar]\nopacity = inf\n", "bar.opacity"),
        ("[bar]\nopacity = \"0.5\"\n", "bar.opacity"),
        ("[bar]\nopacity = true\n", "bar.opacity"),
    ] {
        let error = read(text).unwrap_err().to_string();
        // Type errors come from the TOML layer and name the key too.
        assert!(
            error.contains(key.strip_prefix("bar.").unwrap()),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn fallback_fonts_are_at_most_two_paths() {
    assert!(read("").unwrap().fallback_fonts.is_empty());
    let config = read("[bar]\nfallback-fonts = [\"/f/Symbols.ttf\", \"/f/Cjk.otf\"]\n").unwrap();
    assert_eq!(
        config.fallback_fonts,
        [PathBuf::from("/f/Symbols.ttf"), PathBuf::from("/f/Cjk.otf")]
    );
    let error = read("[bar]\nfallback-fonts = [\"a\", \"b\", \"c\"]\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("bar.fallback-fonts") && error.contains("at most 2"),
        "{error}"
    );
    // The wrong type is a parse refusal naming the key.
    let error = read("[bar]\nfallback-fonts = \"a\"\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("fallback-fonts"), "{error}");
}

#[test]
#[cfg(feature = "clock")]
fn a_clock_icon_is_exactly_one_character() {
    assert_eq!(read("").unwrap().modules.clock.icon, None);
    let config = read("[clock]\nicon = \"\u{f0e65}\"\n").unwrap();
    assert_eq!(
        config.modules.clock.icon,
        Some(crate::icon::Icon::Glyph('\u{f0e65}'))
    );
    for bad in ["", "ab", "\\n", "\\u0065\\u0301"] {
        let error = read(&format!("[clock]\nicon = \"{bad}\"\n"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("clock.icon"), "{bad:?}: {error}");
    }
}

#[test]
#[cfg(feature = "window-title")]
fn a_window_title_section_is_read_whole() {
    use crate::modules::window_title::DEFAULT_MAX_WIDTH;
    let modules = read("").unwrap().modules.window_title;
    assert!(!modules.show_app_id);
    assert_eq!(modules.max_width, DEFAULT_MAX_WIDTH);
    assert!(modules.placeholder.is_empty());
    assert!(!modules.allow_close);
    let modules = read(
        "left = [\"window-title\"]\n\
         [window-title]\n\
         show-app-id = true\n\
         max-width = 200\n\
         placeholder = \"empty\"\n\
         allow-close = true\n\
         on-click = \"activate\"\n\
         on-middle-click = \"close\"\n",
    )
    .unwrap()
    .modules
    .window_title;
    assert!(modules.show_app_id);
    assert_eq!(modules.max_width, 200);
    assert_eq!(modules.placeholder, "empty");
    assert!(modules.allow_close);
}

#[test]
#[cfg(feature = "window-title")]
fn a_window_title_section_is_refused_loudly() {
    for (text, key) in [
        ("[window-title]\nmax-width = 0\n", "window-title.max-width"),
        (
            "[window-title]\nmax-width = 4097\n",
            "window-title.max-width",
        ),
        ("[window-title]\nwhatever = 1\n", "whatever"),
        (
            "[window-title]\non-click = \"raise\"\n",
            "window-title.on-click",
        ),
        (
            "[window-title]\non-click = \"activate 3\"\n",
            "window-title.on-click",
        ),
        (
            "[window-title]\non-click = \"close\"\n",
            "window-title.on-click",
        ),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
    // Type errors come from the TOML layer and name the key without its
    // section.
    for (text, key) in [
        ("[window-title]\nmax-width = -1\n", "max-width"),
        ("[window-title]\nmax-width = \"200\"\n", "max-width"),
        ("[window-title]\nshow-app-id = \"yes\"\n", "show-app-id"),
        ("[window-title]\nallow-close = 1\n", "allow-close"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
    // A `close` binding with closing off is a click that would do nothing.
    let error = read("[window-title]\non-middle-click = \"close\"\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("window-title.on-middle-click") && error.contains("allow-close"),
        "{error}"
    );
    // ... and allowed with it on.
    assert!(read("[window-title]\nallow-close = true\non-middle-click = \"close\"\n").is_ok());
    // A placeholder past the view's bound is refused, not cut silently.
    let long = "x".repeat(crate::modules::MAX_TEXT + 1);
    let error = read(&format!("[window-title]\nplaceholder = \"{long}\"\n"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("window-title.placeholder"), "{error}");
}

#[test]
#[cfg(feature = "volume")]
fn a_volume_section_is_read_whole() {
    use crate::modules::volume::{DEFAULT_MAX_VOLUME, DEFAULT_STEP};
    let modules = read("").unwrap().modules.volume;
    assert_eq!(modules.step, DEFAULT_STEP);
    assert_eq!(modules.max_volume, DEFAULT_MAX_VOLUME);
    assert!(modules.icon.is_none());
    let modules = read(
        "left = [\"volume\"]\n\
         [volume]\n\
         step = 10\n\
         max-volume = 120\n\
         on-click = \"toggle-mute\"\n\
         on-scroll-up = \"raise\"\n\
         on-scroll-down = \"lower\"\n",
    )
    .unwrap()
    .modules
    .volume;
    assert_eq!(modules.step, 10);
    assert_eq!(modules.max_volume, 120);
}

#[test]
#[cfg(feature = "volume")]
fn a_volume_section_is_refused_loudly() {
    for (text, key) in [
        ("[volume]\nstep = 0\n", "volume.step"),
        ("[volume]\nstep = 51\n", "volume.step"),
        ("[volume]\nmax-volume = 99\n", "volume.max-volume"),
        ("[volume]\nmax-volume = 151\n", "volume.max-volume"),
        ("[volume]\nwhatever = 1\n", "whatever"),
        // Another module's action, and a number where none is taken.
        ("[volume]\non-click = \"activate\"\n", "volume.on-click"),
        ("[volume]\non-click = \"raise 3\"\n", "volume.on-click"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
}

#[test]
#[cfg(feature = "microphone")]
fn a_microphone_section_is_read_whole() {
    use crate::modules::volume::{DEFAULT_MAX_VOLUME, DEFAULT_STEP};
    let modules = read("").unwrap().modules.microphone;
    assert_eq!(modules.step, DEFAULT_STEP);
    assert_eq!(modules.max_volume, DEFAULT_MAX_VOLUME);
    let modules = read(
        "left = [\"microphone\"]\n\
         [microphone]\n\
         step = 2\n\
         on-click = \"toggle-mute\"\n",
    )
    .unwrap()
    .modules
    .microphone;
    assert_eq!(modules.step, 2);
}

#[test]
#[cfg(feature = "microphone")]
fn a_microphone_section_is_refused_loudly() {
    for (text, key) in [
        ("[microphone]\nstep = 0\n", "microphone.step"),
        ("[microphone]\nmax-volume = 151\n", "microphone.max-volume"),
        ("[microphone]\nwhatever = 1\n", "whatever"),
        (
            "[microphone]\non-click = \"activate\"\n",
            "microphone.on-click",
        ),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
}

#[test]
#[cfg(feature = "brightness")]
fn a_brightness_section_is_read_whole() {
    use crate::modules::brightness::DEFAULT_STEP;
    let modules = read("").unwrap().modules.brightness;
    assert!(modules.device.is_none());
    assert_eq!(modules.step, DEFAULT_STEP);
    let modules = read(
        "left = [\"brightness\"]\n\
         [brightness]\n\
         device = \"apple-panel-bl\"\n\
         step = 10\n\
         on-scroll-up = \"raise\"\n\
         on-click = { exec = [\"foot\", \"-e\", \"light\"] }\n",
    )
    .unwrap()
    .modules
    .brightness;
    assert_eq!(modules.device.as_deref(), Some("apple-panel-bl"));
    assert_eq!(modules.step, 10);
}

#[test]
#[cfg(feature = "network")]
fn a_network_section_is_read_whole() {
    let modules = read("").unwrap().modules.network;
    assert!(modules.interface.is_none());
    assert!(modules.show_ssid);
    assert!(modules.menu_command.is_empty());
    let modules = read(
        "left = [\"network\"]\n\
         [network]\n\
         interface = \"wlan0\"\n\
         show-ssid = false\n\
         menu-command = [\"fuzzel\", \"--dmenu\"]\n\
         on-click = \"menu\"\n",
    )
    .unwrap()
    .modules
    .network;
    assert_eq!(modules.interface.as_deref(), Some("wlan0"));
    assert!(!modules.show_ssid);
    assert_eq!(modules.menu_command, ["fuzzel", "--dmenu"]);
}

#[test]
#[cfg(feature = "network")]
fn a_network_section_is_refused_loudly() {
    for (text, key) in [
        ("[network]\ninterface = \"\"\n", "network.interface"),
        (
            "[network]\ninterface = \"0123456789abcdef\"\n",
            "network.interface",
        ),
        ("[network]\nmenu-command = [\"\"]\n", "network.menu-command"),
        ("[network]\nwhatever = 1\n", "whatever"),
        ("[network]\non-click = \"raise\"\n", "network.on-click"),
        ("[network]\non-click = \"menu 2\"\n", "network.on-click"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
    // Type errors come from the TOML layer and name the key without its
    // section.
    for (text, key) in [
        ("[network]\nshow-ssid = \"yes\"\n", "show-ssid"),
        ("[network]\nmenu-command = \"fuzzel\"\n", "menu-command"),
        ("[network]\ninterface = 3\n", "interface"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
}

#[test]
#[cfg(feature = "brightness")]
fn a_brightness_section_is_refused_loudly() {
    for (text, key) in [
        ("[brightness]\ndevice = \"\"\n", "brightness.device"),
        (
            "[brightness]\ndevice = \"../escape\"\n",
            "brightness.device",
        ),
        ("[brightness]\nstep = 0\n", "brightness.step"),
        ("[brightness]\nstep = 51\n", "brightness.step"),
        ("[brightness]\nwhatever = 1\n", "whatever"),
        // `raise` takes no number, `set` takes one.
        (
            "[brightness]\non-click = \"raise 2\"\n",
            "brightness.on-click",
        ),
        ("[brightness]\non-click = \"set\"\n", "brightness.on-click"),
        (
            "[brightness]\non-click = \"frobnicate\"\n",
            "brightness.on-click",
        ),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
    // Type errors come from the TOML layer and name the key without its
    // section.
    for (text, key) in [
        ("[brightness]\nstep = \"five\"\n", "step"),
        ("[brightness]\ndevice = 3\n", "device"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
}

#[test]
#[cfg(feature = "tray")]
fn a_tray_section_is_read_whole() {
    let modules = read("").unwrap().modules.tray;
    assert_eq!(modules, crate::modules::tray::Settings::default());
    let config = read(
        "left = [\"tray\"]\n\
         [tray]\n\
         on-click = \"activate 0\"\n\
         on-scroll-up = \"scroll-up 1\"\n",
    )
    .unwrap();
    assert!(config.layout.left.contains(&"tray"));
    let bindings = config.modules.bindings_of("tray");
    assert!(!bindings.is_empty());
}

#[test]
#[cfg(feature = "tray")]
fn a_tray_section_is_refused_loudly() {
    for (text, key) in [
        ("[tray]\nwhatever = 1\n", "whatever"),
        // `activate` takes a number.
        ("[tray]\non-click = \"activate\"\n", "tray.on-click"),
        ("[tray]\non-click = \"frobnicate 0\"\n", "tray.on-click"),
        ("[tray]\nmargin = 99999\n", "tray.margin"),
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains(key), "{text:?}: {error}");
    }
}
