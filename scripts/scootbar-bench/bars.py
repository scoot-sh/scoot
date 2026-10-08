"""The bars measured, each configured to show exactly the milestone's scope.

The resource ratchet (docs/scootbar/backlog/lightest.md, rule 2) compares
scootbar with the competitors *at the milestone's own scope*: a bar that
shows only a clock is compared with bars showing only a clock. So every bar
here shows ``SCOPES[scope]`` and nothing else, with the look M0's baselines
used (dev/spikes/scootbar/m0/results/bar-configs): a 26-pixel bar along
the top, DejaVu Sans at 14 pixels, ``#1e1e2e`` behind ``#cdd6f4``, the
clock on the right as ``%a %d %b %H:%M``, updated each minute where the bar
has the option (Waybar's ``interval``; yambar and scootbar follow the
format's finest field).

Competitors are yambar and Waybar, the two the ratchet names ("Clock and
workspaces are compared with yambar and waybar"). ironbar and ashell (M0's
"if cheap" pair) run the same way but are **informational**: their columns
are measured and shown, and the gate (``tables.py``) neither counts them
nor fails on them, since the ratified rule does not name them (promoting
them into it is the maintainer's call). They are only ever run as the
binaries nixpkgs builds: ashell is GPL-3.0-or-later and ironbar MIT, and
nothing of theirs is copied; their configs here use only the keys that
give the look above. Each runs from the flake's
pinned nixpkgs, as M0's did (the same store paths), and is started
directly, through nixpkgs' wrapper where it has one (which ``exec``s, so
the measured process is the bar).
"""

import json
import os

# What a milestone shows, by part of the bar: the clock on the right, the
# workspaces on the left. M1 is the clock; M3's scope is both.
SCOPES = {
    "clock": {"right": ("clock",)},
    "clock-workspaces": {"left": ("workspaces",), "right": ("clock",)},
}


def shows(scope, module):
    return any(module in mods for mods in SCOPES[scope].values())

FORMAT = "%a %d %b %H:%M"
HEIGHT = 26
FONT_PX = 14
BACKGROUND = "1e1e2e"
FOREGROUND = "cdd6f4"


def is_elf(path):
    try:
        with open(path, "rb") as f:
            return f.read(4) == b"\x7fELF"
    except OSError:
        return False


def real_executables(bindir):
    """The ELF executables in ``bindir`` that are not launchers: nixpkgs'
    ``makeBinaryWrapper`` launcher is an ELF too (Waybar's is 16 KB), and
    it sits beside the real binary, ``.NAME-wrapped``."""
    names = set(os.listdir(bindir))
    return sorted(
        os.path.join(bindir, n)
        for n in names
        if f".{n}-wrapped" not in names and is_elf(os.path.join(bindir, n))
    )


class Bar:
    name = ""
    nix_attr = None
    # The one this benchmark is about: the others are gated against it.
    reference = False
    # Measured and shown, but not a competitor the ratchet names: never
    # counted by the gate (``tables.gate``).
    informational = False

    def __init__(self, store=None, binary=None, version=None):
        self.store = store
        self.binary = binary
        self.version = version

    def executable(self):
        """What to run: nixpkgs' wrapper where there is one."""
        return self.binary or os.path.join(self.store, "bin", self.nix_attr)

    def elf_binaries(self):
        """The ELF files that are the bar, for the Size row: the package's
        executables, its launcher scripts left out (nixpkgs puts the real
        binary beside the wrapper as ``.NAME-wrapped``)."""
        if self.binary:
            return [self.binary]
        return real_executables(os.path.join(self.store, "bin"))

    def cannot_show(self, scope, compositor):
        """Why this bar cannot show ``scope`` on ``compositor``, or ``None``
        when it can. The harness does not run a bar for a scope it cannot
        show, nor invent a number for it: the report names it instead."""
        return None

    def write_config(self, directory, font_file, scope, compositor):
        """Writes the bar's configuration under ``directory``; returns the
        command line that uses it."""
        raise NotImplementedError


class Scootbar(Bar):
    name = "scootbar"
    reference = True

    def executable(self):
        return self.binary

    def write_config(self, directory, font_file, scope, compositor):
        # No configuration file: every option is a flag (site/src/content/docs/scootbar/cli.md).
        # Giving any of --left/--center/--right sets the whole layout, so a
        # part the scope leaves empty is simply not given (M1's command line
        # for the clock scope is unchanged).
        layout = []
        for part in ("left", "center", "right"):
            if part in SCOPES[scope]:
                layout += [f"--{part}", ",".join(SCOPES[scope][part])]
        return [
            self.binary, "daemon",
            "--font", font_file,
            "--font-size", str(FONT_PX),
            "--height", str(HEIGHT),
            "--background", f"#{BACKGROUND}",
            "--foreground", f"#{FOREGROUND}",
            *layout,
            "--clock-format", FORMAT,
        ]


class Yambar(Bar):
    name = "yambar"
    nix_attr = "yambar"

    def cannot_show(self, scope, compositor):
        # yambar 1.11.0 has no ext-workspace-v1 module (its modules for
        # workspaces are i3/sway, river and dwl-style tags), so on scoot it
        # cannot show the workspaces; on sway its `i3` module (which speaks
        # sway's IPC) does.
        if shows(scope, "workspaces") and compositor != "sway":
            return ("yambar 1.11.0 has no ext-workspace-v1 module, so it cannot "
                    f"show workspaces on {compositor}")
        return None

    def write_config(self, directory, font_file, scope, compositor):
        assert not self.cannot_show(scope, compositor), (scope, compositor)
        date, time = FORMAT.rsplit(" ", 1)
        path = os.path.join(directory, "yambar.yml")
        parts = SCOPES[scope]
        text = (
            "bar:\n"
            f"  height: {HEIGHT}\n"
            "  location: top\n"
            f"  background: {BACKGROUND}ff\n"
            f"  foreground: {FOREGROUND}ff\n"
            f"  font: DejaVu Sans:pixelsize={FONT_PX}\n"
        )
        if "left" in parts:
            text += "  left:\n"
            for module in parts["left"]:
                text += self.module(module, date, time)
        if "right" in parts:
            text += "  right:\n"
            for module in parts["right"]:
                text += self.module(module, date, time)
        with open(path, "w") as f:
            f.write(text)
        return [self.executable(), "-c", path]

    @staticmethod
    def module(module, date, time):
        if module == "clock":
            return (
                "    - clock:\n"
                f'        date-format: "{date}"\n'
                f'        time-format: "{time}"\n'
                "        content:\n"
                '          - string: {text: "{date} {time}", right-margin: 8}\n'
            )
        assert module == "workspaces", module
        # The `i3` module: one string per workspace, its number. Sway's
        # workspaces are named by number here (`workspace number N`).
        return (
            "    - i3:\n"
            "        sort: native\n"
            "        content:\n"
            '          "":\n'
            '            string: {text: "{name}", margin: 4}\n'
        )


class Waybar(Bar):
    name = "waybar"
    nix_attr = "waybar"

    # Waybar's module for the workspaces, by compositor: ext-workspace-v1
    # on scoot, sway's own IPC on sway.
    WORKSPACES = {"scoot": "ext/workspaces", "sway": "sway/workspaces"}

    def cannot_show(self, scope, compositor):
        if shows(scope, "workspaces") and compositor not in self.WORKSPACES:
            return f"no workspaces module for {compositor}"
        return None

    def write_config(self, directory, font_file, scope, compositor):
        assert not self.cannot_show(scope, compositor), (scope, compositor)
        config = os.path.join(directory, "waybar.jsonc")
        style = os.path.join(directory, "waybar.css")
        ids = {"clock": "clock"}
        if shows(scope, "workspaces"):
            ids["workspaces"] = self.WORKSPACES[compositor]
        body = {
            "layer": "top",
            "position": "top",
            "height": HEIGHT,
            "clock": {"format": "{:" + FORMAT + "}", "interval": 60, "tooltip": False},
        }
        if "workspaces" in ids:
            body[ids["workspaces"]] = {"format": "{name}"}
        for part, mods in SCOPES[scope].items():
            body[f"modules-{part}"] = [ids[m] for m in mods]
        with open(config, "w") as f:
            json.dump(body, f, indent=1)
        with open(style, "w") as f:
            f.write(
                f'* {{ font-family: "DejaVu Sans"; font-size: {FONT_PX}px; }}\n'
                f"window#waybar {{ background: #{BACKGROUND}; color: #{FOREGROUND}; }}\n"
            )
        return [self.executable(), "-c", config, "-s", style]


class Ironbar(Bar):
    """ironbar 0.19.0 (GTK, MIT). It speaks compositor IPCs, not
    ext-workspace-v1 (M0: its workspaces module failed to start on scoot), so
    on scoot it can show the clock only and the workspaces scope is not run
    there, as for yambar; on sway its sway/i3 support shows them."""

    name = "ironbar"
    nix_attr = "ironbar"
    informational = True

    def cannot_show(self, scope, compositor):
        if shows(scope, "workspaces") and compositor != "sway":
            return ("ironbar 0.19.0 has no ext-workspace-v1 support (it speaks "
                    f"compositor IPCs), so it cannot show workspaces on {compositor}")
        return None

    def write_config(self, directory, font_file, scope, compositor):
        assert not self.cannot_show(scope, compositor), (scope, compositor)
        config = os.path.join(directory, "ironbar.json")
        style = os.path.join(directory, "ironbar.css")
        modules = {
            "clock": {"type": "clock", "format": FORMAT},
            "workspaces": {"type": "workspaces"},
        }
        body = {"position": "top", "height": HEIGHT}
        for part, mods in SCOPES[scope].items():
            body[{"left": "start", "center": "center", "right": "end"}[part]] = [
                modules[m] for m in mods]
        with open(config, "w") as f:
            json.dump(body, f, indent=1)
        with open(style, "w") as f:
            f.write(
                f'* {{ font-family: "DejaVu Sans"; font-size: {FONT_PX}px; }}\n'
                f"#bar {{ background-color: #{BACKGROUND}; color: #{FOREGROUND}; }}\n"
            )
        return [self.executable(), "-c", config, "-t", style]


class Ashell(Bar):
    """ashell 0.10.0 (iced, GPL-3.0-or-later; only ever run as nixpkgs'
    binary). It shows workspaces on both compositors. Its bar height and
    font size are not options (iced theme tokens), so those two are the
    bar's own; the family, colors, clock format and modules are set."""

    name = "ashell"
    nix_attr = "ashell"
    informational = True
    MODULES = {"workspaces": "Workspaces", "clock": "Tempo"}

    def write_config(self, directory, font_file, scope, compositor):
        path = os.path.join(directory, "ashell.toml")
        # Its default layout has a window title in the middle and a system
        # group on the right; the scope has neither.
        lists = {part: ", ".join(f'"{self.MODULES[m]}"' for m in SCOPES[scope].get(part, ()))
                 for part in ("left", "center", "right")}
        text = (
            "[modules]\n"
            f"left = [{lists['left']}]\n"
            f"center = [{lists['center']}]\n"
            f"right = [{lists['right']}]\n"
            "\n"
            "[tempo]\n"
            f'clock_format = "{FORMAT}"\n'
            "\n"
            "[appearance]\n"
            'font_name = "DejaVu Sans"\n'
            f'text_color = "#{FOREGROUND}"\n'
            "\n"
            "[appearance.bar]\n"
            'surface = "solid"\n'
            "\n"
            "[appearance.background_color]\n"
            f'base = "#{BACKGROUND}"\n'
        )
        with open(path, "w") as f:
            f.write(text)
        return [self.executable(), "-c", path]


ALL = [Scootbar, Yambar, Waybar, Ironbar, Ashell]


def fontconfig(directory, fonts_dir):
    """A fontconfig file that sees DejaVu only (``fonts_dir``), with its
    cache in ``directory``, so every bar and the terminals find the same
    face and no run reads the machine's fonts."""
    path = os.path.join(directory, "fonts.conf")
    cache = os.path.join(directory, "fc-cache")
    os.makedirs(cache, exist_ok=True)
    with open(path, "w") as f:
        f.write(
            '<?xml version="1.0"?>\n'
            '<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">\n'
            "<fontconfig>\n"
            f"  <dir>{fonts_dir}</dir>\n"
            f"  <cachedir>{cache}</cachedir>\n"
            "  <alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>\n"
            "  <alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>\n"
            "</fontconfig>\n"
        )
    return path
