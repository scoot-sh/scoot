//! Argument parsing. Deliberately small: scoot's surface is one compositor to
//! start, plus the `msg` client (parsed and run through the `scootctl`
//! library crate, which owns that surface -- see that crate's docs).

use std::fmt;
use std::path::PathBuf;

use scoot_ipc::Request;

use scootctl::Error;

pub const BACKEND_USAGE: &str = "\
    scoot --headless [OPTIONS] [-- COMMAND...]
    scoot --nested [OPTIONS] [-- COMMAND...]
    scoot --tty [OPTIONS] [-- COMMAND...]
    scoot --print-default-config [--write]
    scoot --version
    scoot msg REQUEST
    scoot msg --help [--json]
    scoot msg help [TOPIC|VERB|--json]
    scoot --help [--json]
    scoot help [TOPIC|--json]

OPTIONS (backends in brackets: all means --headless, --nested and --tty):
    --width 1-65535      output width in pixels (default 1600) [headless, nested]
    --height 1-65535     output height in pixels (default 1000) [headless, nested]
    --outputs 1-8        headless outputs, left to right (default 1) [headless]
    --gpu PATH           DRM device, when the automatic choice is wrong [tty]
    --mode WxH           display mode, when the preferred one is wrong [tty]
    --renderer cpu|gpu|auto  which renderer composites each frame [all]
    --xwayland           run an XWayland server inside the session [all]
    --socket PATH        where the IPC socket lives [all]
    --config PATH        which config file to read [all]
    -- COMMAND...        run once the compositor is up, with WAYLAND_DISPLAY set
";

/// The config-file topics `scoot help config` summarizes: every section
/// `site/src/content/docs/scoot/configure.md` documents, with the one command that emits them
/// all. The file itself stays the reference; this is the map to it.
const CONFIG_BODY: &str = "\
CONFIG:
    The config file ($XDG_CONFIG_HOME/scoot/config.toml) holds every option;
    a flag given replaces the file's value for its own option. Sections:

    [layout] [appearance] [output] [[outputs]] [renderer] [tty] [xwayland]
    [binds] [autostart] [floating] [[window_rule]] [wallpaper]

    `scoot --print-default-config` prints a starting file generated from the
    compositor's own live defaults; with `--write` it places the file (never
    overwriting). `scoot msg reload` re-applies what can be re-applied live.
";

/// The config-file map: [`CONFIG_BODY`] plus the reference URLs, rendered
/// from [`scoot_ipc::DOCS_URL`] -- the one copy of the docs domain, so a
/// move is one edit. A function rather than a `const` so the text can name
/// that constant; a cold path (one process per `--help`), so the one small
/// allocation costs nothing.
pub fn config_help() -> String {
    let mut text = String::with_capacity(CONFIG_BODY.len() + 128);
    text.push_str(CONFIG_BODY);
    text.push_str(&scoot_ipc::docs_tail("scoot/configure.md"));
    text
}

/// One `scoot help` page.
#[derive(Debug, PartialEq)]
pub enum HelpPage {
    /// The full text: [`usage`].
    Main,
    /// The machine-readable form: [`json`].
    Json,
    /// The config-file map: [`config_help`].
    Config,
    /// The full client help (`scoot msg --help`), rendered for `scoot msg`.
    Client,
    /// One client topic (`scoot msg help requests`, ...), rendered
    /// for `scoot msg`.
    ClientTopic(scootctl::help::Topic),
    /// One client verb's row (`help screenshot`), rendered for `scoot msg`.
    ClientVerb { verb: String },
    /// The client document (`scoot msg --help --json`).
    ClientJson,
}

impl HelpPage {
    /// Renders the page to stdout text (the JSON page renders the document).
    pub fn text(&self) -> String {
        match self {
            Self::Main => usage(),
            Self::Json => json(),
            Self::Config => config_help(),
            Self::Client => scootctl::help::usage(
                "scoot msg",
                scootctl::cli::REQUESTS_HELP,
                scootctl::cli::ACTIONS_HELP,
                false,
            ),
            Self::ClientTopic(topic) => scootctl::help::topic_text(*topic, "scoot msg"),
            Self::ClientVerb { verb } => {
                scootctl::help::verb_text(verb, "scoot msg").unwrap_or_else(usage)
            }
            Self::ClientJson => scootctl::help::json("scoot msg", false),
        }
    }
}

/// The full `--help` text: the compositor's own surface plus the client
/// surface single-sourced from the client library (its prose blocks and its
/// table-rendered sections), so `scoot --help` and `scoot msg --help` cannot
/// drift. A function rather than a `const` so the two forms share code.
pub fn usage() -> String {
    let mut text = String::from("scoot -- a scrolling-tiling Wayland compositor\n\nUSAGE:\n    ");
    text.push_str(BACKEND_USAGE);
    text.push_str("\nREQUESTS:\n    ");
    text.push_str(scootctl::cli::REQUESTS_HELP);
    text.push_str("\nACTIONS:\n    ");
    text.push_str(scootctl::cli::ACTIONS_HELP);
    text.push_str(
        "\nEXAMPLES:\n\
        \x20   scoot --headless --outputs 2 -- foot\n\
        \x20   scoot --print-default-config > ~/.config/scoot/config.toml\n\
        \x20   scoot msg windows\n\
        \x20   scoot msg action focus-workspace-index 2\n\
        \x20   scoot msg screenshot --out /tmp/shot.png\n\
        ",
    );
    text.push_str("\nEXIT CODES:\n");
    for (code, meaning) in scootctl::help::EXIT_CODES {
        text.push_str(&format!("    {code}  {meaning}\n"));
    }
    text.push_str("\nENVIRONMENT:\n");
    for (name, description) in ENVIRONMENT {
        text.push_str(&format!("    {name}  {description}\n"));
    }
    text.push_str(&format!(
        "\nSEE ALSO:\n\
        \x20   `scoot help config`, `scoot msg help requests`, `scoot msg help actions`\n\
        \x20   docs: {0}/scoot/configure.md and {0}/msg/\n\
        \x20   agents start at {0}/llms.txt\n",
        scoot_ipc::DOCS_URL
    ));
    text
}

/// Environment the compositor reads, beyond what the client reads.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    (
        "WAYLAND_DISPLAY",
        "the caller's display: --nested presents there as a window",
    ),
    (
        "SCOOT_RENDERER",
        "if --renderer is absent: cpu, gpu or auto; beats [renderer] backend",
    ),
    (
        "SCOOT_SOCKET",
        "the IPC socket's path; the default is $XDG_RUNTIME_DIR/scoot.sock",
    ),
    (
        "XDG_RUNTIME_DIR",
        "where the default socket lives; missing is a one-line startup error",
    ),
    (
        "XDG_CONFIG_HOME",
        "where the config file lives (~/.config/scoot/config.toml by default)",
    ),
];

/// One compositor backend, for the JSON document: its name and the flags it
/// takes, each with what follows it and its default. Mirrors `compositor()`
/// below -- the drift test parses every flag here.
pub struct BackendDoc {
    pub name: &'static str,
    pub flags: &'static [FlagDoc],
}

pub struct FlagDoc {
    pub flag: &'static str,
    pub takes: &'static str,
    pub default: &'static str,
}

/// Every backend and the flags it takes, in usage order.
pub const BACKENDS: &[BackendDoc] = &[
    BackendDoc {
        name: "--headless",
        flags: &[
            FlagDoc {
                flag: "--width",
                takes: "1-65535",
                default: "1600",
            },
            FlagDoc {
                flag: "--height",
                takes: "1-65535",
                default: "1000",
            },
            FlagDoc {
                flag: "--outputs",
                takes: "1-8",
                default: "1",
            },
            FlagDoc {
                flag: "--renderer",
                takes: "cpu|gpu|auto",
                default: "cpu (or $SCOOT_RENDERER, or the config file)",
            },
            FlagDoc {
                flag: "--xwayland",
                takes: "(none)",
                default: "off",
            },
            FlagDoc {
                flag: "--socket",
                takes: "PATH",
                default: "$SCOOT_SOCKET or $XDG_RUNTIME_DIR/scoot.sock",
            },
            FlagDoc {
                flag: "--config",
                takes: "PATH",
                default: "$XDG_CONFIG_HOME/scoot/config.toml",
            },
        ],
    },
    BackendDoc {
        name: "--nested",
        flags: &[
            FlagDoc {
                flag: "--width",
                takes: "1-65535",
                default: "1600",
            },
            FlagDoc {
                flag: "--height",
                takes: "1-65535",
                default: "1000",
            },
            FlagDoc {
                flag: "--renderer",
                takes: "cpu|gpu|auto",
                default: "cpu (or $SCOOT_RENDERER, or the config file)",
            },
            FlagDoc {
                flag: "--xwayland",
                takes: "(none)",
                default: "off",
            },
            FlagDoc {
                flag: "--socket",
                takes: "PATH",
                default: "$SCOOT_SOCKET or $XDG_RUNTIME_DIR/scoot.sock",
            },
            FlagDoc {
                flag: "--config",
                takes: "PATH",
                default: "$XDG_CONFIG_HOME/scoot/config.toml",
            },
        ],
    },
    BackendDoc {
        name: "--tty",
        flags: &[
            FlagDoc {
                flag: "--gpu",
                takes: "PATH",
                default: "automatic",
            },
            FlagDoc {
                flag: "--mode",
                takes: "WxH",
                default: "the connector's preferred mode",
            },
            FlagDoc {
                flag: "--renderer",
                takes: "cpu|gpu|auto",
                default: "cpu (or $SCOOT_RENDERER, or the config file)",
            },
            FlagDoc {
                flag: "--xwayland",
                takes: "(none)",
                default: "off",
            },
            FlagDoc {
                flag: "--socket",
                takes: "PATH",
                default: "$SCOOT_SOCKET or $XDG_RUNTIME_DIR/scoot.sock",
            },
            FlagDoc {
                flag: "--config",
                takes: "PATH",
                default: "$XDG_CONFIG_HOME/scoot/config.toml",
            },
        ],
    },
];

/// The `--help --json` document: the compositor surface plus the client
/// surface embedded from the client library, from the same tables as the text.
pub fn json() -> String {
    serde_json::to_string_pretty(&json_value()).unwrap_or_else(|_| "{}".into())
}

fn json_value() -> serde_json::Value {
    let mut root = serde_json::Map::new();
    root.insert(
        "schema_version".into(),
        serde_json::Value::from(scootctl::help::SCHEMA_VERSION),
    );
    root.insert("binary".into(), serde_json::Value::from("scoot"));
    root.insert(
        "about".into(),
        serde_json::Value::from("a scrolling-tiling Wayland compositor"),
    );
    root.insert(
        "backends".into(),
        serde_json::Value::from(
            BACKENDS
                .iter()
                .map(|backend| {
                    let mut entry = serde_json::Map::new();
                    entry.insert("name".into(), serde_json::Value::from(backend.name));
                    entry.insert(
                        "flags".into(),
                        serde_json::Value::from(
                            backend
                                .flags
                                .iter()
                                .map(|flag| {
                                    let mut field = serde_json::Map::new();
                                    field.insert("flag".into(), serde_json::Value::from(flag.flag));
                                    field.insert(
                                        "takes".into(),
                                        serde_json::Value::from(flag.takes),
                                    );
                                    field.insert(
                                        "default".into(),
                                        serde_json::Value::from(flag.default),
                                    );
                                    serde_json::Value::Object(field)
                                })
                                .collect::<Vec<_>>(),
                        ),
                    );
                    serde_json::Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "client".into(),
        scootctl::help::json_value("scoot msg", false),
    );
    root.insert(
        "config_sections".into(),
        serde_json::Value::from(vec![
            serde_json::Value::from("[layout]"),
            serde_json::Value::from("[appearance]"),
            serde_json::Value::from("[output]"),
            serde_json::Value::from("[[outputs]]"),
            serde_json::Value::from("[renderer]"),
            serde_json::Value::from("[tty]"),
            serde_json::Value::from("[xwayland]"),
            serde_json::Value::from("[binds]"),
            serde_json::Value::from("[autostart]"),
            serde_json::Value::from("[floating]"),
            serde_json::Value::from("[[window_rule]]"),
            serde_json::Value::from("[wallpaper]"),
        ]),
    );
    serde_json::Value::Object(root)
}

/// The largest `--width`/`--height` a `--headless`/`--nested` output may ask
/// for, per axis.
///
/// Two reasons for this exact number, both about what a mode can report
/// rather than taste:
///
/// - **Nothing real can use more.** DRM reports each mode axis in a `u16`
///   (`drm_mode_modeinfo`'s `hdisplay`/`vdisplay` in the kernel's uAPI
///   headers), so no connector can list anything past 65535 -- and neither
///   can `--mode WxH`, which parses as `(u16, u16)` in this same file. Real
///   hardware sits far below that (8K is 7680 wide; the widest 16K
///   prototype is 15360), so the bound has room to spare with margin.
/// - **It keeps every output-derived sum in the layout far from overflow.**
///   The largest one, `available + gap` in `scoot_core`'s `column_width`,
///   tops out at `ceil(65535 / MIN_SCALE) + Config::MAX_GAP` -- 131070 +
///   10,000 at the `[output] scale` floor of 0.5 -- over four orders of
///   magnitude inside `i32`.
///
/// [`CompositorOptions::width`]/[`CompositorOptions::height`] below carry
/// the range into the type docs; `dimension` enforces it at parse.
pub const MAX_OUTPUT_DIMENSION: i32 = 65535;

/// The most outputs `--headless --outputs N` will create.
///
/// Small on purpose. The flag exists so multi-output behaviour is testable
/// with no second monitor in the building, and nothing tests more screens than
/// a desk holds -- an unbounded count would only buy a way to ask for a
/// million `wl_output` globals.
///
/// It also keeps the one sum the flag introduces far from overflow. Outputs are
/// laid out left to right, so the right edge of the last one is at most
/// `MAX_OUTPUTS * ceil(MAX_OUTPUT_DIMENSION / MIN_SCALE)` -- 8 * 131070, about
/// 1.05 million, four orders of magnitude inside `i32` and inside the same
/// margin [`MAX_OUTPUT_DIMENSION`] claims for the layout's own sums.
/// `headless::add_output` saturates in any case.
pub const MAX_OUTPUTS: i32 = 8;

/// Which renderer composites each frame: the *resolved* tier.
///
/// Lives here rather than beside the renderers themselves because
/// [`CompositorOptions`] has to exist on every platform -- the client
/// The client (`scoot msg`) builds anywhere, while `compositor::render`
/// is Linux-only -- and because the config file parses the same names
/// (`[renderer] backend`). The user-facing spellings are `cpu` (pixman)
/// and `gpu` (GLES); the variant names stay as they were so internal
/// matches read unchanged. One name list, one parser, the way the client library's
/// [`scootctl::action`] is shared with `[binds]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// The resolved tier is read only by the Linux compositor; the macOS client
// parses requests but never resolves them.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum RendererKind {
    /// CPU compositing with pixman: the default, and the only renderer that
    /// needs no graphics device at all. Spelled `cpu` to the user.
    #[default]
    Pixman,
    /// GLES on an EGL device. Opt-in; `--headless`/`--nested` honour it in
    /// every build (the offscreen pipeline). Under `--tty` it needs the
    /// `gpu-scanout` Cargo feature, where the frame is composited straight
    /// into the buffer the CRTC scans out; without the feature `--tty`
    /// warns and keeps the CPU renderer, since the offscreen pipeline would only copy
    /// every frame back to the CPU (see `compositor::render::resolve`, and
    /// `tty::init`'s fallback when the device cannot drive the tier).
    /// Spelled `gpu` to the user.
    Gles,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl RendererKind {
    /// The spelling a user writes, in both the flag and the config file --    /// and what every log line and warning names it by, so the two cannot
    /// drift. `gpu` means the GLES renderer (scanout on `--tty`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pixman => "cpu",
            Self::Gles => "gpu",
        }
    }

    /// Exactly the two names [`RendererKind::as_str`] produces, and nothing
    /// else -- no aliases, no case folding, and the old `pixman`/`gles`
    /// spellings are not accepted. `None` is "not one of ours",
    /// which the flag refuses outright and the config file warns about and
    /// ignores (see `compositor::config`).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cpu" => Some(Self::Pixman),
            "gpu" => Some(Self::Gles),
            _ => None,
        }
    }
}

impl fmt::Display for RendererKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the user *asked* for, before the session resolves it to a
/// [`RendererKind`]. Separate from the resolved tier on purpose: `auto`
/// is not a tier, and every site that reads "what was built" must keep
/// reading [`RendererKind`], never this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererRequest {
    /// Pick the best tier for this session: the GPU renderer on real
    /// hardware, the CPU renderer in VMs and headless. Never fails startup.
    Auto,
    /// The CPU renderer (pixman), everywhere. The default request.
    #[default]
    Cpu,
    /// The GPU renderer (GLES): the offscreen pipeline under
    /// `--headless`/`--nested`, scanout under `--tty` in a `gpu-scanout`
    /// build.
    Gpu,
}

/// The default request: the CPU tier. Every caller that defaults uses this
/// constant, so the stage that flips the default to `auto` moves one line
/// (and its pin test).
// Resolved only by the Linux compositor (see `RendererKind`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const DEFAULT_REQUEST: RendererRequest = RendererRequest::Cpu;

impl RendererRequest {
    /// The spelling a user writes: `cpu`, `gpu` or `auto`. Shared by the
    /// flag, `SCOOT_RENDERER` and the config file, so the three cannot
    /// drift. No aliases: the old `pixman`/`gles` spellings are rejected
    /// with a message naming the new value (see [`renamed_renderer`]).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }

    /// Exactly the three names [`RendererRequest::as_str`] produces, and
    /// nothing else -- no aliases, no case folding.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "gpu" => Some(Self::Gpu),
            _ => None,
        }
    }

    /// The tier an explicit (non-`auto`) request names, if it names one.
    // Resolved only by the Linux compositor (see `RendererKind`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn kind(self) -> Option<RendererKind> {
        match self {
            Self::Auto => None,
            Self::Cpu => Some(RendererKind::Pixman),
            Self::Gpu => Some(RendererKind::Gles),
        }
    }
}

impl fmt::Display for RendererRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a [`RendererRequest`] came from, for the `renderer chosen` log
/// line. Precedence is flag > `SCOOT_RENDERER` > config file > default.
// Produced only by the Linux compositor (see `RendererKind`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestSource {
    /// An explicit `--renderer`.
    Flag,
    /// The `SCOOT_RENDERER` environment variable.
    Env,
    /// The config file's `[renderer] backend`.
    Config,
    /// Neither flag, env nor file named one: [`DEFAULT_REQUEST`].
    Default,
}

impl fmt::Display for RequestSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::Config => "config",
            Self::Default => "default",
        })
    }
}

/// The old spelling's new name, for the rejection message. `None` when the
/// value was never ours under any spelling.
pub fn renamed_renderer(value: &str) -> Option<&'static str> {
    match value {
        "pixman" => Some("cpu"),
        "gles" => Some("gpu"),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help(HelpPage),
    /// `scoot --version`: identify this build without starting anything.
    /// Prints [`scootctl::version_string`] and exits. A first-arg flag like
    /// `--help` and `--print-default-config`, not a backend and not `msg`:
    /// it needs no running compositor, and the bare `version` word stays
    /// the IPC request's (answered by the compositor over the socket --
    /// giving one spelling two transports would make its failure mode
    /// depend on whether a session happens to be up).
    Version,
    /// `scoot --print-default-config [--write]`: emit a starting config file,
    /// generated from the compositor's own live defaults (see
    /// `compositor::config::default_config_toml`). Stdout by default, so it
    /// cannot clobber an existing config and it composes
    /// (`scoot --print-default-config > ~/.config/scoot/config.toml`).
    /// With `--write` it goes to the default config location instead (see
    /// `compositor::config::write_default_config`), which refuses loudly
    /// rather than overwriting anything already there. A first-arg flag like
    /// `--help`, not a backend and not `msg`: it needs no running
    /// compositor -- producing a file before a session is configured is the
    /// whole point.
    PrintDefaultConfig {
        /// Place the emission at the default config location instead of
        /// printing it, refusing when anything already exists there.
        write: bool,
    },
    Compositor(CompositorOptions),
    Msg {
        request: Request,
        out: Option<PathBuf>,
        /// Whether a `binds` reply renders as JSON rather than the human
        /// table (see `scootctl::Msg`): presentation only, never on the wire.
        json: bool,
    },
}

#[derive(Debug, PartialEq)]
pub struct CompositorOptions {
    /// The requested size. Under `--nested` it is only what scoot asks for:
    /// the host's first configure decides what the window comes up at, and
    /// every later one moves it (see `compositor::nested`). Under
    /// `--headless` it's authoritative, there being no host to negotiate
    /// with. Each axis is in `1..=MAX_OUTPUT_DIMENSION`; `parse` refuses
    /// anything else, and so does a host configure naming one.
    pub width: i32,
    pub height: i32,
    /// How many outputs `--headless` creates, each `width` by `height` and
    /// placed left to right with no gap. `1..=MAX_OUTPUTS`; `parse` refuses
    /// anything else.
    ///
    /// `--headless` only, and `compositor::run` warns and ignores it on the
    /// other two backends -- `--nested` presents one window in its host and
    /// `--tty` already drives every connected monitor, so a virtual output
    /// there would be a session quietly different from the one asked for. Every output gets a render target of its own, alongside its
    /// `wl_output` global, its geometry and its own scrolling strip in the
    /// core, which is what makes per-output rendering and protocol
    /// behaviour testable without a second monitor. Nothing is shown on a
    /// headless output in any case.
    pub outputs: i32,
    /// Where to listen for IPC; the default path when `None`.
    pub socket: Option<PathBuf>,
    /// Explicit config file path; the XDG default when `None`. See
    /// `compositor::config::load`'s doc for the resolution and fallback
    /// rules -- an explicit path here that can't be read is a hard startup
    /// error, unlike the default path.
    pub config: Option<PathBuf>,
    /// Launched once the compositor is up, with `WAYLAND_DISPLAY` set.
    pub command: Vec<String>,
    /// Present as a window inside the host compositor named by the *caller's*
    /// `WAYLAND_DISPLAY`, instead of running with no display at all.
    pub nested: bool,
    /// Present on a real DRM/KMS display via a Linux session (libseat) --
    /// the actual deployment target, rather than headless or nested inside
    /// another compositor. Mutually exclusive with `nested` (`parse` only
    /// ever sets one of the two). `width`/`height` are meaningless here and
    /// silently ignored: the connector's preferred mode (or `--mode`, see
    /// `mode` below) picks the output size, and unlike `--nested` there's no
    /// host to negotiate a different size with.
    pub tty: bool,
    /// `--tty`'s DRM device, when the automatic choice is wrong. `None`
    /// (the normal case) means `tty::gpu::candidates` picks: Smithay's
    /// `primary_gpu` first, then every other device on the seat as a
    /// fallback. A path here replaces that search entirely -- exactly one
    /// candidate, no fallback -- so a user on hardware both heuristics get
    /// wrong can name the right device instead. It also wins over the
    /// config file's `[tty] gpu` when both name one (an explicit flag beats
    /// a file, the way `--config` beats the default path). Meaningless outside
    /// `--tty`, where `compositor::run` ignores it *with a warning* --
    /// unlike `width`/`height` under `--tty`, which are dropped silently.
    /// The difference is deliberate: a size has a sensible reading on a
    /// backend that ignores it (the mode wins), whereas naming a DRM
    /// device on a backend with no DRM device at all means the user
    /// believes they are on `--tty` and is not.
    pub gpu: Option<PathBuf>,
    /// Which renderer composites each frame, when the command line says.
    /// `None` (the normal case) means the config file's `[renderer] backend`
    /// decides, and the CPU renderer when that is unset too -- so this is
    /// `Option<RendererRequest>` rather than a plain `RendererRequest` precisely so
    /// that an explicit `--renderer cpu` can *override* a config file
    /// asking for `gpu`, the way `--gpu` overrides `[tty] gpu`. Resolved
    /// once, in `compositor::render::policy::resolve_request` and
    /// `compositor::render::resolve`.
    pub renderer: Option<RendererRequest>,
    /// `--tty`'s display mode, as `WxH`, when the connector's own preferred
    /// mode is the wrong size. `None` (the normal case) takes the preferred
    /// mode, else the first one listed. `Some` picks the connector mode of
    /// exactly that size, and falls back to the preferred one *with a
    /// warning* when the connector offers no such mode -- the wrong size
    /// beats a black screen. Exists for hosts whose "preferred" size is an
    /// artefact rather than a monitor's: Apple's Virtualization framework
    /// (vfkit, UTM) hands the guest a mode sized from the host window in
    /// backing pixels, so it doubles or halves with the screen the window
    /// happened to open on, while the connector also lists the usual
    /// standard sizes. Meaningless outside `--tty`, where `compositor::run`
    /// ignores it with a warning, for the same reason as `gpu`.
    pub mode: Option<(u16, u16)>,
    /// Run an XWayland server inside the session, so X11-only applications
    /// get a `DISPLAY` to connect to. Opt-in and off by default: the server
    /// costs a whole extra process (~55 MB RSS idle, ~87 MB with three X
    /// clients mapped, measured) plus a hard `PATH`
    /// dependency on the `Xwayland` binary, and any X client can
    /// keylog/snoop by design (see `site/src/content/docs/scoot/protocols.md`'s trust note), so it
    /// is an explicit choice, never a default. The config-file form is
    /// `[xwayland] enabled`; either one turns it on (a flag can only say
    /// yes, so the two are OR-ed in `compositor::run`). X windows map into
    /// the layout like any other, behind a focus gate (see
    /// `compositor::xwayland`). Needs an `xwayland` Cargo-feature build;
    /// without one this warns and the session runs Wayland-only, the way
    /// `--renderer gpu` degrades without `gpu-scanout`.
    pub xwayland: bool,
}

impl Default for CompositorOptions {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 1000,
            outputs: 1,
            socket: None,
            config: None,
            command: Vec::new(),
            nested: false,
            tty: false,
            gpu: None,
            renderer: None,
            mode: None,
            xwayland: false,
        }
    }
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None => Ok(Command::Help(HelpPage::Main)),
        Some("--help" | "-h") => help_args(args.collect()),
        Some("help") => help_args(args.collect()),
        Some("--version") => Ok(Command::Version),
        Some("--print-default-config") => print_default_config(args),
        Some("--headless") => compositor(args, false, false).map(Command::Compositor),
        Some("--nested") => compositor(args, true, false).map(Command::Compositor),
        Some("--tty") => compositor(args, false, true).map(Command::Compositor),
        Some("msg") => msg_alias(args.collect()),
        Some(other) => Err(hinted(
            "argument",
            other.to_owned(),
            &[
                "--headless",
                "--nested",
                "--tty",
                "--version",
                "--print-default-config",
                "msg",
                "--help",
                "help",
            ],
            "scoot --help",
        )),
    }
}

/// `scoot help` / `scoot --help`, optionally followed by one topic, client
/// verb, or `--json`. Anything else is refused with a guess -- `help` is
/// where a lost agent lands, so it teaches too.
fn help_args(args: Vec<String>) -> Result<Command, Error> {
    match args.as_slice() {
        [] => Ok(Command::Help(HelpPage::Main)),
        [only] if only == "--json" => Ok(Command::Help(HelpPage::Json)),
        [only] if only == "config" => Ok(Command::Help(HelpPage::Config)),
        [only] if only == "help" => Ok(Command::Help(HelpPage::Main)),
        [only] => {
            if let Some(topic) = scootctl::help::Topic::parse(only) {
                return Ok(Command::Help(HelpPage::ClientTopic(topic)));
            }
            if scootctl::help::verb_text(only, "scoot msg").is_some() {
                return Ok(Command::Help(HelpPage::ClientVerb { verb: only.clone() }));
            }
            let mut candidates: Vec<&str> = vec!["config", "--json"];
            candidates.extend(scootctl::help::Topic::names());
            candidates.extend(scootctl::help::REQUESTS.iter().map(|request| request.verb));
            Err(hinted(
                "help topic",
                only.clone(),
                &candidates,
                "scoot help",
            ))
        }
        [_, extra, ..] => Err(Error::Unknown(extra.clone())),
    }
}

/// `scoot msg ...`: the client alias. `--help` (or `help [TOPIC|VERB]`)
/// right after `msg` asks for the client help instead of a request -- every
/// other word parses through the client library, so `msg` cannot drift.
fn msg_alias(args: Vec<String>) -> Result<Command, Error> {
    match args.as_slice() {
        [] => Err(Error::Missing("a request")),
        [only] if only == "--help" || only == "-h" || only == "help" => {
            Ok(Command::Help(HelpPage::Client))
        }
        [first, rest @ ..] if first == "--help" || first == "-h" || first == "help" => {
            client_help_page(rest)
        }
        _ => scootctl::parse_msg(args).map(|msg| Command::Msg {
            request: msg.request,
            out: msg.out,
            json: msg.json,
        }),
    }
}

/// The client help page for `scoot msg help ...`, rendered for `scoot msg`.
fn client_help_page(args: &[String]) -> Result<Command, Error> {
    match args {
        [] => Ok(Command::Help(HelpPage::Client)),
        [only] if only == "--json" => Ok(Command::Help(HelpPage::ClientJson)),
        [only] => {
            if let Some(topic) = scootctl::help::Topic::parse(only) {
                return Ok(Command::Help(HelpPage::ClientTopic(topic)));
            }
            if scootctl::help::verb_text(only, "scoot msg").is_some() {
                return Ok(Command::Help(HelpPage::ClientVerb { verb: only.clone() }));
            }
            let mut candidates: Vec<&str> = scootctl::help::Topic::names().collect();
            candidates.extend(scootctl::help::REQUESTS.iter().map(|request| request.verb));
            candidates.push("--json");
            Err(hinted(
                "help topic",
                only.clone(),
                &candidates,
                "scoot msg help",
            ))
        }
        [_, extra, ..] => Err(Error::Unknown(extra.clone())),
    }
}

/// An unknown word with a guess attached (see the client library's `hinted`): the
/// closest candidate, or the bare [`Error::Unknown`] when nothing is close
/// enough to be a typo.
fn hinted(kind: &'static str, what: String, candidates: &[&str], topic: &'static str) -> Error {
    match scoot_ipc::suggest(&what, candidates.iter().copied()) {
        Some(suggestion) => Error::Hinted {
            kind,
            what,
            suggestion: suggestion.to_owned(),
            topic,
        },
        None => Error::Unknown(what),
    }
}

/// `scoot --print-default-config [--write]`: the only trailing argument the
/// flag takes is `--write`. Anything else is refused rather than ignored --
/// a typo'd `--wirte` silently emitting to stdout would be exactly the "it
/// ran, but not the way you asked" shape every other invalid flag in this
/// file refuses (see [`renderer`]).
fn print_default_config(mut args: impl Iterator<Item = String>) -> Result<Command, Error> {
    match args.next().as_deref() {
        None => Ok(Command::PrintDefaultConfig { write: false }),
        Some("--write") => match args.next().as_deref() {
            None => Ok(Command::PrintDefaultConfig { write: true }),
            Some(extra) => Err(hinted(
                "flag",
                extra.to_owned(),
                &["--write"],
                "scoot --help",
            )),
        },
        Some(other) => Err(hinted(
            "flag",
            other.to_owned(),
            &["--write"],
            "scoot --help",
        )),
    }
}

fn compositor(
    mut args: impl Iterator<Item = String>,
    nested: bool,
    tty: bool,
) -> Result<CompositorOptions, Error> {
    let mut options = CompositorOptions {
        nested,
        tty,
        ..CompositorOptions::default()
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => options.width = dimension("--width", args.next())?,
            "--height" => options.height = dimension("--height", args.next())?,
            "--outputs" => options.outputs = count("--outputs", args.next())?,
            "--socket" => {
                let path = args.next().ok_or(Error::Missing("a path after --socket"))?;
                options.socket = Some(PathBuf::from(path));
            }
            "--config" => {
                let path = args.next().ok_or(Error::Missing("a path after --config"))?;
                options.config = Some(PathBuf::from(path));
            }
            "--gpu" => {
                let path = args.next().ok_or(Error::Missing("a path after --gpu"))?;
                options.gpu = Some(PathBuf::from(path));
            }
            "--renderer" => options.renderer = Some(renderer("--renderer", args.next())?),
            "--mode" => options.mode = Some(mode("--mode", args.next())?),
            "--xwayland" => options.xwayland = true,
            "--" => {
                options.command = args.by_ref().collect();
                break;
            }
            other => {
                return Err(hinted(
                    "flag",
                    other.to_owned(),
                    &[
                        "--width",
                        "--height",
                        "--outputs",
                        "--socket",
                        "--config",
                        "--gpu",
                        "--renderer",
                        "--mode",
                        "--xwayland",
                        "--",
                    ],
                    "scoot --help",
                ));
            }
        }
    }
    Ok(options)
}

/// One of the three renderer requests, refused rather than defaulted: a typo'd
/// `--renderer gpuu` silently compositing on the CPU is exactly the kind of
/// "it ran, but not the way you asked" this project treats as a bug. The
/// config file's copy of the same key is the graceful half (warn, keep the
/// default) -- see `compositor::config` for why the two differ.
///
/// The old `pixman`/`gles` spellings are rejected with a message naming the
/// new value (`cpu`/`gpu`): breaking, no aliases.
fn renderer(what: &'static str, value: Option<String>) -> Result<RendererRequest, Error> {
    let value = value.ok_or(Error::Missing("cpu, gpu or auto after --renderer"))?;
    if let Some(request) = RendererRequest::parse(&value) {
        return Ok(request);
    }
    if let Some(new) = renamed_renderer(&value) {
        return Err(Error::Invalid {
            what,
            value: format!("{value} was renamed to {new}; use cpu, gpu or auto"),
        });
    }
    Err(Error::Invalid { what, value })
}

/// A display mode as `WxH` (`1920x1080`): two positive pixel counts around a
/// lowercase `x`. `u16` because that is what DRM's `Mode::size()` yields,
/// so the comparison in `tty::gpu` is exact rather than converted.
fn mode(what: &'static str, value: Option<String>) -> Result<(u16, u16), Error> {
    let value = value.ok_or(Error::Missing("a WxH size after --mode"))?;
    parse_mode(&value).ok_or(Error::Invalid { what, value })
}

/// The pure half of [`mode`]: `--mode`'s value, and an `[[outputs]]` entry's
/// `mode` (see `compositor::output_config`), so the flag and the file accept
/// exactly the same spellings. Each axis is a positive `u16`, so a parsed
/// size is never past [`MAX_OUTPUT_DIMENSION`]. `None` for anything else.
pub(crate) fn parse_mode(value: &str) -> Option<(u16, u16)> {
    let (width, height) = value.split_once('x')?;
    match (width.parse::<u16>(), height.parse::<u16>()) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Some((width, height)),
        _ => None,
    }
}

/// A `--width`/`--height` value: a positive pixel count no DRM mode could
/// exceed (see [`MAX_OUTPUT_DIMENSION`]). Refused rather than clamped --
/// every other invalid flag in this file is an `Error::Invalid`, and a
/// typo'd size silently running at a different size would be the worse
/// surprise. Unparsable input keeps the plain `Invalid` shape; a parsed
/// number outside the range echoes the range it was refused for.
fn dimension(what: &'static str, value: Option<String>) -> Result<i32, Error> {
    let raw = value.ok_or(Error::Missing(what))?;
    let size: i32 = raw.parse().map_err(|_| Error::Invalid {
        what,
        value: raw.clone(),
    })?;
    if (1..=MAX_OUTPUT_DIMENSION).contains(&size) {
        Ok(size)
    } else {
        Err(Error::OutOfRange {
            what,
            value: raw,
            min: 1,
            max: MAX_OUTPUT_DIMENSION,
        })
    }
}

/// An `--outputs` value: at least one output, at most [`MAX_OUTPUTS`].
/// Refused rather than clamped, for the same reason [`dimension`] refuses --
/// silently running with a different number of screens than was asked for is
/// the surprise, not the error.
fn count(what: &'static str, value: Option<String>) -> Result<i32, Error> {
    let raw = value.ok_or(Error::Missing(what))?;
    let parsed: i32 = raw.parse().map_err(|_| Error::Invalid {
        what,
        value: raw.clone(),
    })?;
    if (1..=MAX_OUTPUTS).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(Error::OutOfRange {
            what,
            value: raw,
            min: 1,
            max: MAX_OUTPUTS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, Error> {
        parse(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn no_arguments_prints_help() {
        assert_eq!(parse_args(&[]), Ok(Command::Help(HelpPage::Main)));
    }

    #[test]
    fn version_parses_to_its_own_command() {
        // A first-arg flag like `--help`, not a backend and not `msg`: it
        // needs no running compositor, and trailing arguments are ignored
        // the way `--help foo` still prints help.
        assert_eq!(parse_args(&["--version"]), Ok(Command::Version));
        assert_eq!(
            parse_args(&["--version", "--headless", "foo"]),
            Ok(Command::Version)
        );
    }

    #[test]
    fn version_is_first_arg_only_like_help() {
        // `scoot --headless --version` is not `--version`, the way
        // `scoot --headless --help` is not help: backend flags consume the
        // rest of the line, and `--version` is not one of theirs.
        assert_eq!(
            parse_args(&["--headless", "--version"]),
            Err(Error::Unknown("--version".into()))
        );
    }

    #[test]
    fn usage_names_version_on_its_own_line() {
        // The `--help` surface for the new flag: its own usage line, so a
        // user reading `--help` can discover it without knowing the ticket.
        assert!(
            usage().lines().any(|line| line.trim() == "scoot --version"),
            "--help hides the version flag"
        );
    }

    #[test]
    fn version_line_names_this_binarys_version_and_the_protocol() {
        // `scoot --version` names *this* binary's `CARGO_PKG_VERSION` (the
        // same `env!` the IPC `version` reply reads) and the live
        // `PROTOCOL_VERSION`, so a protocol bump without the string fails
        // here rather than shipping a line that lies. It must not go
        // through `scootctl::version_string()`: that names scootctl's own
        // version, which agrees with this binary's only while the lockstep
        // trio holds (independent versioning) -- `main` passes this
        // crate's version explicitly, and this pins that the helper honors
        // what it is given rather than reading its own.
        let line = scootctl::version_string_for(env!("CARGO_PKG_VERSION"));
        assert!(
            line.contains(env!("CARGO_PKG_VERSION")),
            "version line does not name this build: {line}"
        );
        assert!(
            line.contains(&format!("(ipc protocol {})", scoot_ipc::PROTOCOL_VERSION)),
            "version line does not track PROTOCOL_VERSION: {line}"
        );
        // And the two agree today: the trio is lockstep, so scootctl's own
        // line still names this same version (scripts/version enforces it).
        assert_eq!(line, scootctl::version_string());
    }

    #[test]
    fn print_default_config_parses_to_its_own_command() {
        // A first-arg flag like `--help`, not a backend and not `msg`: it
        // needs no running compositor, and bare it emits to stdout.
        assert_eq!(
            parse_args(&["--print-default-config"]),
            Ok(Command::PrintDefaultConfig { write: false })
        );
    }

    #[test]
    fn print_default_config_write_parses_and_anything_else_is_refused() {
        // The one trailing argument the flag takes. A typo is an error, not
        // a silent stdout emission -- see `print_default_config`.
        assert_eq!(
            parse_args(&["--print-default-config", "--write"]),
            Ok(Command::PrintDefaultConfig { write: true })
        );
        assert_eq!(
            parse_args(&["--print-default-config", "--wirte"]),
            Err(Error::Hinted {
                kind: "flag",
                what: "--wirte".into(),
                suggestion: "--write".into(),
                topic: "scoot --help",
            })
        );
        assert_eq!(
            parse_args(&["--print-default-config", "--write", "--write"]),
            Err(Error::Unknown("--write".into()))
        );
        assert_eq!(
            parse_args(&["--print-default-config", "foo"]),
            Err(Error::Unknown("foo".into()))
        );
    }

    #[test]
    fn usage_names_print_default_config_on_its_own_line() {
        // The `--help` surface for the flag and its one argument: its own
        // usage line, so a user reading `--help` can discover it without
        // knowing the ticket.
        assert!(
            usage()
                .lines()
                .any(|line| line.trim() == "scoot --print-default-config [--write]"),
            "--help hides the default-config flag or its --write argument"
        );
    }

    #[test]
    fn compositor_options_have_defaults_and_overrides() {
        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options, CompositorOptions::default());

        let Ok(Command::Compositor(options)) = parse_args(&[
            "--headless",
            "--width",
            "800",
            "--height",
            "600",
            "--",
            "foot",
            "-e",
            "sh",
        ]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.width, 800);
        assert_eq!(options.height, 600);
        assert_eq!(options.command, vec!["foot", "-e", "sh"]);
        assert!(!options.nested);
    }

    #[test]
    fn width_and_height_refuse_what_no_mode_can_report() {
        // DRM reports each mode axis in a `u16` (`drm_mode_modeinfo`), so
        // nothing real is wider than 65535: anything past it is a typo or a
        // probe, refused the way `--mode` refuses its own bad input rather
        // than silently running at a different size. The refusal echoes the
        // range, so the operator sees the fix, not just the failure.
        for bad in ["0", "-1", "-1600", "65536", "2000000000"] {
            for flag in ["--width", "--height"] {
                let err = parse_args(&["--headless", flag, bad])
                    .expect_err("an absurd size should not parse");
                assert_eq!(
                    err,
                    Error::OutOfRange {
                        what: flag,
                        value: bad.to_owned(),
                        min: 1,
                        max: MAX_OUTPUT_DIMENSION,
                    },
                    "{flag} {bad}"
                );
                assert!(
                    err.to_string().contains("expected 1-65535"),
                    "refusal for {flag} {bad} did not echo the range: {err}"
                );
            }
        }
    }

    #[test]
    fn width_and_height_accept_the_whole_legal_range() {
        // One pixel, today's sizes, 8K/16K, and exactly the DRM maximum.
        for good in ["1", "800", "1920", "7680", "15360", "65535"] {
            let expected: i32 = good.parse().unwrap();
            let Ok(Command::Compositor(options)) =
                parse_args(&["--headless", "--width", good, "--height", good])
            else {
                panic!("expected compositor for {good}");
            };
            assert_eq!((options.width, options.height), (expected, expected));
        }
    }

    #[test]
    fn outputs_defaults_to_one_and_takes_the_whole_legal_range() {
        for mode in ["--headless", "--nested", "--tty"] {
            let Ok(Command::Compositor(options)) = parse_args(&[mode]) else {
                panic!("expected compositor");
            };
            assert_eq!(options.outputs, 1, "{mode}");
        }

        for good in ["1", "2", "3", "8"] {
            let expected: i32 = good.parse().unwrap();
            let Ok(Command::Compositor(options)) = parse_args(&["--headless", "--outputs", good])
            else {
                panic!("expected compositor for {good}");
            };
            assert_eq!(options.outputs, expected);
        }
    }

    #[test]
    fn outputs_refuses_zero_negatives_and_more_screens_than_a_desk_holds() {
        // Refused rather than clamped, and the refusal echoes the range --
        // the same shape `--width`/`--height` have, so a typo is a message
        // rather than a session with a surprising number of screens.
        for bad in ["0", "-1", "9", "100", "2000000000", "two", ""] {
            let err = parse_args(&["--headless", "--outputs", bad])
                .expect_err("an out-of-range output count should not parse");
            match bad.parse::<i32>() {
                Ok(_) => assert_eq!(
                    err,
                    Error::OutOfRange {
                        what: "--outputs",
                        value: bad.to_owned(),
                        min: 1,
                        max: MAX_OUTPUTS,
                    },
                    "{bad}"
                ),
                Err(_) => assert_eq!(
                    err,
                    Error::Invalid {
                        what: "--outputs",
                        value: bad.to_owned(),
                    },
                    "{bad}"
                ),
            }
        }
        assert_eq!(
            parse_args(&["--headless", "--outputs"]),
            Err(Error::Missing("--outputs"))
        );
    }

    #[test]
    fn tty_mode_is_none_by_default_parses_wxh_and_rejects_the_rest() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.mode, None);

        let Ok(Command::Compositor(options)) = parse_args(&["--tty", "--mode", "1920x1080"]) else {
            panic!("expected compositor");
        };
        assert!(options.tty);
        assert_eq!(options.mode, Some((1920, 1080)));

        // Anything that is not two positive numbers around an `x`.
        for bad in [
            "1920",
            "1920x",
            "x1080",
            "0x1080",
            "1920x0",
            "1920X1080",
            "wide",
        ] {
            assert_eq!(
                parse_args(&["--tty", "--mode", bad]),
                Err(Error::Invalid {
                    what: "--mode",
                    value: bad.to_owned(),
                }),
                "{bad}"
            );
        }
        assert_eq!(
            parse_args(&["--tty", "--mode"]),
            Err(Error::Missing("a WxH size after --mode"))
        );
    }

    #[test]
    fn config_and_socket_paths_are_none_by_default_and_settable() {
        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.config, None);
        assert_eq!(options.socket, None);

        let Ok(Command::Compositor(options)) = parse_args(&[
            "--headless",
            "--config",
            "/etc/scoot/config.toml",
            "--socket",
            "/tmp/scoot.sock",
        ]) else {
            panic!("expected compositor");
        };
        assert_eq!(
            options.config,
            Some(PathBuf::from("/etc/scoot/config.toml"))
        );
        assert_eq!(options.socket, Some(PathBuf::from("/tmp/scoot.sock")));
    }

    #[test]
    fn nested_sets_the_flag_headless_does_not() {
        let Ok(Command::Compositor(options)) = parse_args(&["--nested"]) else {
            panic!("expected compositor");
        };
        assert!(options.nested);

        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert!(!options.nested);
    }

    #[test]
    fn tty_sets_its_own_flag_and_nothing_else() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert!(options.tty);
        assert!(!options.nested);

        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert!(!options.tty);

        let Ok(Command::Compositor(options)) = parse_args(&["--nested"]) else {
            panic!("expected compositor");
        };
        assert!(!options.tty);
    }

    #[test]
    fn gpu_is_none_by_default_and_takes_a_path() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.gpu, None);

        let Ok(Command::Compositor(options)) = parse_args(&["--tty", "--gpu", "/dev/dri/card1"])
        else {
            panic!("expected compositor");
        };
        assert_eq!(options.gpu, Some(PathBuf::from("/dev/dri/card1")));
    }

    #[test]
    fn gpu_without_a_path_is_an_error() {
        assert_eq!(
            parse_args(&["--tty", "--gpu"]),
            Err(Error::Missing("a path after --gpu"))
        );
    }

    #[test]
    fn xwayland_is_off_by_default_and_on_with_the_flag() {
        // Opt-in, on every backend: the flag can only say yes, so the
        // default must be off (see `CompositorOptions::xwayland`).
        for mode in ["--headless", "--nested", "--tty"] {
            let Ok(Command::Compositor(options)) = parse_args(&[mode]) else {
                panic!("expected compositor for {mode}");
            };
            assert!(!options.xwayland, "{mode}");
            let Ok(Command::Compositor(options)) = parse_args(&[mode, "--xwayland"]) else {
                panic!("expected compositor for {mode} --xwayland");
            };
            assert!(options.xwayland, "{mode}");
        }
    }

    #[test]
    fn usage_documents_xwayland_for_every_backend() {
        // The `--help` surface for the flag: the backends table carries it
        // on all three backends, and the text documents it -- so a user
        // reading `--help` can discover it without knowing the ticket.
        for backend in BACKENDS {
            assert!(
                backend.flags.iter().any(|flag| flag.flag == "--xwayland"),
                "{} hides --xwayland",
                backend.name
            );
        }
        assert!(usage().contains("--xwayland"), "--help hides --xwayland");
    }

    #[test]
    fn renderer_is_unset_by_default_and_takes_either_name() {
        // Unset, not `Cpu`: the config file only gets a say when the
        // command line has not spoken, so "absent" and "explicitly cpu"
        // cannot be the same value (see `CompositorOptions::renderer`).
        for mode in ["--headless", "--nested", "--tty"] {
            let Ok(Command::Compositor(options)) = parse_args(&[mode]) else {
                panic!("expected compositor");
            };
            assert_eq!(options.renderer, None, "{mode}");
        }

        for (name, expected) in [
            ("cpu", RendererRequest::Cpu),
            ("gpu", RendererRequest::Gpu),
            ("auto", RendererRequest::Auto),
        ] {
            let Ok(Command::Compositor(options)) = parse_args(&["--headless", "--renderer", name])
            else {
                panic!("expected compositor for {name}");
            };
            assert_eq!(options.renderer, Some(expected));
        }
    }

    #[test]
    fn an_unknown_renderer_name_is_refused_not_defaulted() {
        // Including the near-misses a typo actually produces, and the empty
        // string: none of them may quietly composite with the other renderer.
        for bad in ["gpuu", "GPU", "opengl", "gl", "vulkan", "", "cpu "] {
            assert_eq!(
                parse_args(&["--headless", "--renderer", bad]),
                Err(Error::Invalid {
                    what: "--renderer",
                    value: bad.to_owned(),
                }),
                "{bad}"
            );
        }
        // The old spellings are rejected with a message naming the new
        // value: breaking, no aliases.
        for (old, new) in [("pixman", "cpu"), ("gles", "gpu")] {
            assert_eq!(
                parse_args(&["--headless", "--renderer", old]),
                Err(Error::Invalid {
                    what: "--renderer",
                    value: format!("{old} was renamed to {new}; use cpu, gpu or auto"),
                }),
                "{old}"
            );
        }
        assert_eq!(
            parse_args(&["--headless", "--renderer"]),
            Err(Error::Missing("cpu, gpu or auto after --renderer"))
        );
    }

    #[test]
    fn the_renderer_names_round_trip_through_their_own_spelling() {
        for kind in [RendererKind::Pixman, RendererKind::Gles] {
            assert_eq!(RendererKind::parse(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        for request in [
            RendererRequest::Auto,
            RendererRequest::Cpu,
            RendererRequest::Gpu,
        ] {
            assert_eq!(RendererRequest::parse(request.as_str()), Some(request));
            assert_eq!(request.to_string(), request.as_str());
        }
        assert_eq!(RendererKind::default(), RendererKind::Pixman);
        // The default request is the CPU tier (pinned here; the stage that
        // flips it to `auto` moves this test on purpose).
        assert_eq!(DEFAULT_REQUEST, RendererRequest::Cpu);
        assert_eq!(RendererRequest::default(), RendererRequest::Cpu);
        // The request-to-tier map behind the policy: explicit requests name
        // their tier, `auto` names none.
        assert_eq!(RendererRequest::Cpu.kind(), Some(RendererKind::Pixman));
        assert_eq!(RendererRequest::Gpu.kind(), Some(RendererKind::Gles));
        assert_eq!(RendererRequest::Auto.kind(), None);
    }

    #[test]
    fn request_sources_render_for_the_log_line() {
        // The `source=` field of the `renderer chosen` line.
        for (source, text) in [
            (RequestSource::Flag, "flag"),
            (RequestSource::Env, "env"),
            (RequestSource::Config, "config"),
            (RequestSource::Default, "default"),
        ] {
            assert_eq!(source.to_string(), text);
        }
    }

    #[test]
    fn bad_input_explains_itself() {
        assert_eq!(parse_args(&["fly"]), Err(Error::Unknown("fly".into())));
        assert_eq!(
            parse_args(&["--headless", "--width"]),
            Err(Error::Missing("--width"))
        );
        assert_eq!(
            parse_args(&["--headless", "--bogus"]),
            Err(Error::Unknown("--bogus".into()))
        );
    }

    #[test]
    fn usage_lists_renderer_on_every_backend_that_parses_it() {
        // Fail-first pin for the `--help` blind spot the README audit found:
        // the parser (`compositor`, one function for all three backends)
        // accepts `--renderer` everywhere, so the backends table and the
        // text must both name it everywhere. Found 2026-09-20 with the
        // `--tty` line missing it while `README.md`,
        // `site/src/content/docs/scoot/configure.md` and `site/src/content/docs/scoot/backends.md` all showed
        // `scoot --tty --renderer gpu`.
        let text = usage();
        for backend in BACKENDS {
            assert!(
                backend.flags.iter().any(|flag| flag.flag == "--renderer"),
                "{}'s table entry hides a flag its parse accepts",
                backend.name
            );
            assert!(
                text.contains("--renderer cpu|gpu|auto"),
                "help text hides --renderer"
            );
            for name in ["cpu", "gpu", "auto"] {
                let Ok(Command::Compositor(options)) =
                    parse_args(&[backend.name, "--renderer", name])
                else {
                    panic!("{} should parse --renderer {name}", backend.name);
                };
                assert_eq!(
                    options.renderer,
                    RendererRequest::parse(name),
                    "{} --renderer {name}",
                    backend.name
                );
            }
        }
    }

    #[test]
    fn every_tabulated_flag_parses_on_its_backend() {
        // The drift pin for the JSON table: a flag listed for a backend
        // must parse there, with a representative value.
        fn sample(flag: &str) -> Vec<&str> {
            match flag {
                "--width" => vec![flag, "800"],
                "--height" => vec![flag, "600"],
                "--outputs" => vec![flag, "2"],
                "--socket" => vec![flag, "/tmp/scoot-help-probe.sock"],
                "--config" => vec![flag, "/dev/null"],
                "--gpu" => vec![flag, "/dev/dri/card0"],
                "--renderer" => vec![flag, "cpu"],
                "--mode" => vec![flag, "1920x1080"],
                "--xwayland" => vec![flag],
                other => panic!("{other} has no sample value"),
            }
        }
        for backend in BACKENDS {
            for flag in backend.flags {
                let mut argv = vec![backend.name];
                argv.extend(sample(flag.flag));
                assert!(
                    matches!(parse_args(&argv), Ok(Command::Compositor(_))),
                    "{:?} does not parse",
                    argv
                );
            }
        }
    }

    #[test]
    fn help_pages_route_to_their_topic_or_json() {
        use scootctl::help::Topic;
        assert_eq!(parse_args(&["help"]), Ok(Command::Help(HelpPage::Main)));
        assert_eq!(parse_args(&["--help"]), Ok(Command::Help(HelpPage::Main)));
        assert_eq!(
            parse_args(&["help", "config"]),
            Ok(Command::Help(HelpPage::Config))
        );
        assert_eq!(
            parse_args(&["help", "--json"]),
            Ok(Command::Help(HelpPage::Json))
        );
        assert_eq!(
            parse_args(&["--help", "--json"]),
            Ok(Command::Help(HelpPage::Json))
        );
        assert_eq!(
            parse_args(&["help", "actions"]),
            Ok(Command::Help(HelpPage::ClientTopic(Topic::Actions)))
        );
        assert_eq!(
            parse_args(&["help", "screenshot"]),
            Ok(Command::Help(HelpPage::ClientVerb {
                verb: "screenshot".into()
            }))
        );
        assert_eq!(
            parse_args(&["msg", "--help"]),
            Ok(Command::Help(HelpPage::Client))
        );
        assert_eq!(
            parse_args(&["msg", "help"]),
            Ok(Command::Help(HelpPage::Client))
        );
        assert_eq!(
            parse_args(&["msg", "help", "requests"]),
            Ok(Command::Help(HelpPage::ClientTopic(Topic::Requests)))
        );
        assert_eq!(
            parse_args(&["msg", "--help", "--json"]),
            Ok(Command::Help(HelpPage::ClientJson))
        );
        assert_eq!(
            parse_args(&["msg", "help", "--json"]),
            Ok(Command::Help(HelpPage::ClientJson))
        );
        // A typo'd first word teaches, the way the client's does.
        assert_eq!(
            parse_args(&["--headles"]),
            Err(Error::Hinted {
                kind: "argument",
                what: "--headles".into(),
                suggestion: "--headless".into(),
                topic: "scoot --help",
            })
        );
    }

    #[test]
    fn the_msg_alias_help_names_no_version_flag() {
        // `scoot msg --version` would be a request verb, refused as one --
        // so neither the alias help nor its JSON may list it.
        let text = HelpPage::Client.text();
        assert!(
            !text
                .lines()
                .any(|line| line.trim() == "scoot msg --version"),
            "alias help lists a flag the alias refuses"
        );
        let document: serde_json::Value = serde_json::from_str(&json()).unwrap();
        let usages: Vec<&str> = document["client"]["usage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect();
        assert!(
            !usages.iter().any(|line| line.contains("--version")),
            "alias JSON lists a flag the alias refuses: {usages:?}"
        );
    }

    #[test]
    fn help_text_and_json_share_one_source() {
        // The contract: plain text (no color escapes), wrapped under 100
        // columns, sections in order -- and the JSON parses, versioned,
        // carrying both the backends and the embedded client surface.
        let text = usage();
        assert!(!text.contains('\x1b'), "help must not carry color escapes");
        for line in text.lines() {
            assert!(line.chars().count() < 100, "line over 99 columns: `{line}`");
        }
        let sections = [
            "USAGE:",
            "OPTIONS",
            "REQUESTS:",
            "ACTIONS:",
            "EXAMPLES:",
            "EXIT CODES:",
        ];
        let mut cursor = 0;
        for section in sections {
            let found = text[cursor..]
                .find(section)
                .unwrap_or_else(|| panic!("`{section}` missing or out of order"));
            cursor += found + section.len();
        }
        for tail in ["ENVIRONMENT:", "SEE ALSO:"] {
            assert!(
                text[cursor..].contains(tail),
                "`{tail}` missing or out of order"
            );
        }
        for page in [
            HelpPage::Main,
            HelpPage::Config,
            HelpPage::Client,
            HelpPage::ClientTopic(scootctl::help::Topic::Requests),
            HelpPage::ClientVerb {
                verb: "windows".into(),
            },
        ] {
            let rendered = page.text();
            assert!(!rendered.contains('\x1b'));
            for line in rendered.lines() {
                assert!(line.chars().count() < 100, "line over 99 columns: `{line}`");
            }
        }
        let document: serde_json::Value = serde_json::from_str(&json()).unwrap();
        assert_eq!(
            document["schema_version"],
            serde_json::Value::from(scootctl::help::SCHEMA_VERSION)
        );
        assert_eq!(document["binary"], serde_json::Value::from("scoot"));
        let backends = document["backends"].as_array().unwrap();
        assert_eq!(backends.len(), BACKENDS.len());
        for backend in BACKENDS {
            assert!(
                backends.iter().any(|entry| entry["name"] == backend.name),
                "{} missing from the JSON",
                backend.name
            );
        }
        assert_eq!(
            document["client"]["binary"],
            serde_json::Value::from("scoot msg")
        );
        assert!(document["config_sections"].as_array().unwrap().len() >= 10);
    }

    #[test]
    fn help_prints_the_shared_client_grammar() {
        // The single-source pin from this side: `scoot --help` embeds the
        // same two blocks `scoot msg --help` renders (the prose lives in the
        // client library), so the documented grammar cannot drift from the
        // client's.
        assert!(
            usage().contains(scootctl::cli::REQUESTS_HELP),
            "scoot --help lost the shared REQUESTS block"
        );
        assert!(
            usage().contains(scootctl::cli::ACTIONS_HELP),
            "scoot --help lost the shared ACTIONS block"
        );
    }

    #[test]
    fn the_msg_request_parses_through_the_client_library() {
        // The structural pin behind "msg cannot drift": every one of
        // these arg vectors must parse to the same request (or the same
        // error) through `scoot msg ...` as through the client library's
        // own parser directly.
        // Success cases pin the request *and* the `--out`/`--json` split; error cases
        // pin byte-identical failures.
        let cases: &[&[&str]] = &[
            &["version"],
            &["outputs"],
            &["windows"],
            &["reload"],
            &["action", "focus-column", "left"],
            &["action", "move-window-to-workspace", "down"],
            &["action", "focus-workspace-index", "2"],
            &["action", "move-window-to-workspace-index", "3"],
            &["action", "spawn", "foot", "-e", "htop"],
            &["action", "quit"],
            &["screenshot"],
            &["screenshot", "--output", "2", "--out", "/tmp/shot.png"],
            &["pointer", "move", "10", "20"],
            &["pointer", "click", "10", "20", "right"],
            &["pointer", "button", "left", "press"],
            &["pointer", "scroll", "1", "-2"],
            &["key", "super+h"],
            &["type", "hello", "there"],
            &["keyboard"],
            &["locked"],
            &["binds"],
            &["binds", "--json"],
            &["action", "show-keymap"],
            &["subscribe"],
            &["subscribe", "output", "keyboard"],
            &["subscribe", "workspace"],
            &["subscribe", "lock"],
            &["wait-idle"],
            &["wait-idle", "--quiet-ms", "100", "--timeout-ms", "500"],
            &[],
            &["frobnicate"],
            &["action"],
            &["action", "focus-column", "sideways"],
            &["action", "focus-workspace-index", "down"],
            &["action", "move-window-to-workspace-index", "down"],
            &["action", "spawn"],
            &["pointer", "move", "x", "1"],
            &["pointer", "button", "left", "hold"],
            &["screenshot", "--bogus"],
            &["key"],
        ];
        for case in cases {
            let mut aliased: Vec<&str> = vec!["msg"];
            aliased.extend(case.iter());
            let through_alias = parse_args(&aliased);
            let direct = scootctl::parse_msg(case.iter().map(|a| (*a).to_owned()));
            match (through_alias, direct) {
                (Ok(Command::Msg { request, out, json }), Ok(msg)) => {
                    assert_eq!(request, msg.request, "{case:?}");
                    assert_eq!(out, msg.out, "{case:?}");
                    assert_eq!(json, msg.json, "{case:?}");
                }
                (Err(left), Err(right)) => assert_eq!(left, right, "{case:?}"),
                (left, right) => panic!("msg/library diverged on {case:?}: {left:?} vs {right:?}"),
            }
        }
    }
}
