"""The bars measured, each configured to show exactly the milestone's scope.

The resource ratchet (docs/scootbar/backlog/lightest.md, rule 2) compares
scootbar with the competitors *at the milestone's own scope*: a bar that
shows only a clock is compared with bars showing only a clock. So every bar
here shows ``SCOPES[scope]`` and nothing else, with the look M0's baselines
used (docs/scootbar/spikes/m0/results/bar-configs): a 26-pixel bar along
the top, DejaVu Sans at 14 pixels, ``#1e1e2e`` behind ``#cdd6f4``, the
clock on the right as ``%a %d %b %H:%M``, updated each minute where the bar
has the option (Waybar's ``interval``; yambar and scootbar follow the
format's finest field).

Competitors are yambar and Waybar, the two the ratchet names ("Clock and
workspaces are compared with yambar and waybar"); ironbar and ashell (M0's
"if cheap" pair) can be added the same way. Each runs from the flake's
pinned nixpkgs, as M0's did (the same store paths), and is started
directly, through nixpkgs' wrapper where it has one (which ``exec``s, so
the measured process is the bar).
"""

import json
import os

# What a milestone shows. M1 is the clock; M2 adds the workspaces.
SCOPES = {"clock": ("clock",)}

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

    def write_config(self, directory, font_file, scope):
        """Writes the bar's configuration under ``directory``; returns the
        command line that uses it."""
        raise NotImplementedError


class Scootbar(Bar):
    name = "scootbar"
    reference = True

    def executable(self):
        return self.binary

    def write_config(self, directory, font_file, scope):
        # No configuration file yet: every option is a flag
        # (docs/scootbar/cli.md).
        return [
            self.binary, "daemon",
            "--font", font_file,
            "--font-size", str(FONT_PX),
            "--height", str(HEIGHT),
            "--background", f"#{BACKGROUND}",
            "--foreground", f"#{FOREGROUND}",
            "--right", ",".join(SCOPES[scope]),
            "--clock-format", FORMAT,
        ]


class Yambar(Bar):
    name = "yambar"
    nix_attr = "yambar"

    def write_config(self, directory, font_file, scope):
        assert SCOPES[scope] == ("clock",), scope
        date, time = FORMAT.rsplit(" ", 1)
        path = os.path.join(directory, "yambar.yml")
        with open(path, "w") as f:
            f.write(
                "bar:\n"
                f"  height: {HEIGHT}\n"
                "  location: top\n"
                f"  background: {BACKGROUND}ff\n"
                f"  foreground: {FOREGROUND}ff\n"
                f"  font: DejaVu Sans:pixelsize={FONT_PX}\n"
                "  right:\n"
                "    - clock:\n"
                f'        date-format: "{date}"\n'
                f'        time-format: "{time}"\n'
                "        content:\n"
                '          - string: {text: "{date} {time}", right-margin: 8}\n'
            )
        return [self.executable(), "-c", path]


class Waybar(Bar):
    name = "waybar"
    nix_attr = "waybar"

    def write_config(self, directory, font_file, scope):
        assert SCOPES[scope] == ("clock",), scope
        config = os.path.join(directory, "waybar.jsonc")
        style = os.path.join(directory, "waybar.css")
        with open(config, "w") as f:
            json.dump(
                {
                    "layer": "top",
                    "position": "top",
                    "height": HEIGHT,
                    "modules-right": ["clock"],
                    "clock": {"format": "{:" + FORMAT + "}", "interval": 60, "tooltip": False},
                },
                f,
                indent=1,
            )
        with open(style, "w") as f:
            f.write(
                f'* {{ font-family: "DejaVu Sans"; font-size: {FONT_PX}px; }}\n'
                f"window#waybar {{ background: #{BACKGROUND}; color: #{FOREGROUND}; }}\n"
            )
        return [self.executable(), "-c", config, "-s", style]


ALL = [Scootbar, Yambar, Waybar]


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
