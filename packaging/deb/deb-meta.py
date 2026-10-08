#!/usr/bin/env python3
"""Debian control metadata for the scoot .debs (docs/backlog/packaging/deb-package.md).

Renders the `DEBIAN/control` each .deb ships and checks the
`dpkg-shlibdeps` output the build feeds it against the `ldd`-derived
allow-list from the site's packaging page
(`site/src/content/docs/reference/packaging.md`). Stdlib only, like
`scripts/version` and `scripts/backlog`; tests in `test_deb.py`.

Dependency model (the ticket asks for `dpkg-shlibdeps` `$auto`, the
brief for the `ldd`-derived table): both. `build-deb.sh` runs
`dpkg-shlibdeps -O` over the staged binaries, so the `Depends` versions
are derived at build time, and this module refuses any shlibdeps package
name outside the table below -- a renamed or new system library fails
the build instead of shipping a wrong `Depends`. The coupled pair's
versioned dependency (`scoot` on its `scootbg`) is rendered from the
`scootbg` .deb this same run builds.
"""

import argparse
import re
import sys

MAINTAINER = "Steve Yackey <steveyackey@gmail.com>"
HOMEPAGE = "https://github.com/scoot-sh/scoot"
SECTION = "x11"
PRIORITY = "optional"
DEBIAN_REVISION = "1"

# Runtime sonames per binary, from `ldd` of a release build (the site
# packaging page's table). The Debian package names they resolve to on
# bookworm and newer (checked: bookworm, trixie, noble share all five):
# libc6 and libgcc-s1 arrive via dpkg-shlibdeps, not ldd (the C library
# is not listed as a DT_NEEDED soname of its own).
ALLOW = {
    # The compositor links five system libraries past libc; the bar and
    # the wallpaper daemon link nothing past it (measured: scootbg and
    # scootbar are libc-only on the release build).
    "scoot": {
        "libc6",
        "libgcc-s1",
        "libinput10",
        "libseat1",
        "libudev1",
        "libpixman-1-0",
        "libxkbcommon0",
    },
    "scootbg": {"libc6", "libgcc-s1"},
    "scootbar": {"libc6", "libgcc-s1"},
}

CRATE = {"scoot": "scoot", "scootbg": "scootbg", "scootbar": "scootbar"}

SHORT = {
    "scoot": "Scrolling-tiling Wayland compositor (CPU rendering by default, no GPU needed)",
    "scootbg": "Wallpaper daemon for the scoot compositor",
    "scootbar": "Status bar for the scoot compositor",
}

LONG = {
    "scoot": (
        " scoot is a lightweight Wayland compositor with scrolling columns,\n"
        " CPU rendering by default and full IPC for agents and scripts.\n"
        " .\n"
        " This package ships the compositor, the greeter session launcher\n"
        " (scoot-session) and its wayland-sessions entry, the default\n"
        " configuration text (with the [wallpaper] handoff), and the\n"
        " license inventory. It writes nothing to a user's home."
    ),
    "scootbg": (
        " The wallpaper daemon scoot drives through its [wallpaper] section:\n"
        " solid colors and images on one output or every output, restored\n"
        " on login.\n"
        " .\n"
        " This package ships the daemon and the license inventory. It\n"
        " writes nothing to a user's home."
    ),
    "scootbar": (
        " The status bar for the scoot compositor: workspaces, window\n"
        " title, clock, volume, network, battery and more, on standard\n"
        " Wayland protocols, so it runs against any compositor release.\n"
        " .\n"
        " This package ships the bar, its systemd user unit and a starting\n"
        " configuration. It writes nothing to a user's home."
    ),
}

# Extra control stanzas per package. `scoot`'s versioned `scootbg`
# dependency is the coupled pair from docs/versioning.md: the two move
# in lockstep, so the pair installs at one version. `{scootbg}` is
# filled with the scootbg .deb version this run builds.
SUGGESTS = {
    "scoot": None,
    "scootbg": None,
    # The bar draws text only with a font; without one it draws a plain
    # bar. DejaVu is what the shipped example config points at.
    "scootbar": "fonts-dejavu-core",
}


def crate_version(source, crate):
    """Read `version = "X.Y.Z"` from a crate manifest under source."""
    path = f"{source}/crates/{crate}/Cargo.toml"
    with open(path) as f:
        text = f.read()
    m = re.search(r'^version = "([^"]+)"$', text, re.M)
    if not m:
        raise ValueError(f"no version in {path}")
    return m.group(1)


def deb_version(source, package, revision=DEBIAN_REVISION):
    """`<crate version>-<revision>` for a .deb built from source."""
    return f"{crate_version(source, CRATE[package])}-{revision}"


def check_depends(package, shlibdeps_output):
    """Every shlibdeps package name must be in the allow-list.

    Returns (depends, problems): the `Depends` value to render (the
    shlibdeps entries verbatim, derived not remembered) plus the list
    of names outside the table. A missing allow-list entry is fine (a
    feature dropped a library -- the install proof, `scoot --version`
    and the smoke test on the bare base, catches a dropped library
    instead); an extra shlibdeps name is a build failure with the
    README table to update.
    """
    names = parse_shlibdeps(shlibdeps_output)
    problems = [n for n, _ in names if n not in ALLOW[package]]
    depends = ", ".join(
        f"{n} ({c})" if c else n for n, c in names
    )
    return depends, problems


def parse_shlibdeps(output):
    """Parse `dpkg-shlibdeps -O` output into [(name, constraint)]."""
    m = re.search(r"^shlibs:Depends=(.*)$", output.strip(), re.M)
    if not m:
        raise ValueError(f"no shlibs:Depends line in: {output!r}")
    entries = []
    for part in m.group(1).split(","):
        part = part.strip()
        if not part:
            continue
        mm = re.match(r"^([A-Za-z0-9+.-]+)(?:\s*\(([^)]*)\))?$", part)
        if not mm:
            raise ValueError(f"cannot parse shlibdeps entry: {part!r}")
        entries.append((mm.group(1), mm.group(2) or ""))
    return entries


def render_control(package, source, arch, depends, scootbg_version=None,
                    revision=DEBIAN_REVISION):
    """Render the full DEBIAN/control text for one package."""
    # Note: no Standards-Version. That field lives in source
    # packaging (debian/control), not in an installed binary control:
    # bookworm's lintian flags it as `unknown-field` there.
    lines = [
        f"Package: {package}",
        f"Version: {deb_version(source, package, revision)}",
        f"Architecture: {arch}",
        f"Maintainer: {MAINTAINER}",
        f"Section: {SECTION}",
        f"Priority: {PRIORITY}",
        f"Homepage: {HOMEPAGE}",
    ]
    dep_list = [depends] if depends else []
    if package == "scoot":
        if scootbg_version is None:
            raise ValueError("scoot needs its scootbg version")
        dep_list.append(f"scootbg (= {scootbg_version})")
    if dep_list:
        lines.append(f"Depends: {', '.join(dep_list)}")
    if SUGGESTS[package]:
        lines.append(f"Suggests: {SUGGESTS[package]}")
    lines.append(f"Description: {SHORT[package]}")
    lines.append(LONG[package])
    return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("control", help="print DEBIAN/control for a package")
    c.add_argument("--package", choices=sorted(CRATE), required=True)
    c.add_argument("--source", required=True)
    c.add_argument("--arch", required=True, help="amd64 or arm64")
    c.add_argument("--depends", default="", help="shlibdeps-derived Depends value")
    c.add_argument("--scootbg-version", default=None)
    c.add_argument("--revision", default=DEBIAN_REVISION)

    k = sub.add_parser("check-shlibdeps", help="check shlibdeps output against the allow-list")
    k.add_argument("--package", choices=sorted(CRATE), required=True)
    k.add_argument("output", help="the dpkg-shlibdeps -O stdout")

    v = sub.add_parser("versions", help="print crate versions (CI gate input)")
    v.add_argument("--source", required=True)

    args = ap.parse_args(argv)
    if args.cmd == "control":
        sys.stdout.write(
            render_control(
                args.package, args.source, args.arch,
                args.depends, args.scootbg_version, args.revision,
            )
        )
    elif args.cmd == "check-shlibdeps":
        depends, problems = check_depends(args.package, args.output)
        if problems:
            print(
                f"unexpected runtime dependency for {args.package}: "
                + ", ".join(problems)
                + " (update ALLOW and the README table, from ldd -- not memory)",
                file=sys.stderr,
            )
            return 1
        print(depends)
    elif args.cmd == "versions":
        for package in sorted(CRATE):
            print(f"{package} {crate_version(source=args.source, crate=CRATE[package])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
