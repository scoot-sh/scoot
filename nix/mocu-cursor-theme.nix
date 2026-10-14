# Mocu as a real XCursor theme (`packages.<system>.mocu-cursor-theme`).
#
# The desktop profile's default cursor theme, so compositor-drawn, GTK,
# Qt, XWayland and greeter cursors all agree on one family. Built FROM
# THE VENDORED SVGs (`crates/scoot/src/compositor/cursor/shapes/mocu/`,
# Mocu release 1.1, CC0 -- see that directory's README.md): the same
# single source of truth scoot's built-in cursors rasterise, so the
# theme files and the built-in fallback are the same art. Nothing is
# fetched upstream.
#
# How it builds: each SVG is recolored (the fill `#fafbfc` becomes the
# variant's fill, the stroke `#1a1b1c` its pairing -- the shadow
# `#0a0b0c` and the spinner mask's greys stay as authored), the
# `<circle id="hot">` marker is removed (xcursorgen takes the hotspot
# separately; leaving it would bake a red dot into the cursor), and the
# result is rendered with rsvg-convert at 24, 32, 48 and 64 px. The
# hotspot scales with the size (`round(authored * size / 24)`), and
# xcursorgen packs the four sizes into one Xcursor file per cursor, so
# libXcursor loads the image nearest the physical size -- the same
# nearest-size behavior the compositor's own theme path relies on.
#
# `wait`/`progress` are static first frames, not animations: Mocu's
# frame definitions are not vendored (only the SVGs are), and the
# static first frame is what the built-in draws too (`mocu.rs`: "the
# animation's first frame is what is drawn"), so theme and fallback
# agree.
#
# Two outputs: `out` carries the white theme `Mocu` (fill `#fcfcfc` /
# stroke `#222222`, the pairing the built-in uses by default) and
# `black` the black theme `Mocu-Black` (fill `#111111` / stroke
# `#f9f9f9`, Mocu's own black theme). Same loop, so the second is
# nearly free.
#
# A package expression, not a module: takes `pkgs`, returns the
# derivation. Both `cursor.package` defaults (home-manager and NixOS)
# and `nix/tests.nix` import it, so the three never drift apart --
# the same shape as `notifications-mako.nix` / `clipboard-cliphist.nix`.
# Never imported off Linux: the desktop profile is Linux-only there,
# so callers guard with `stdenv.hostPlatform.isLinux` first and this
# file has no such guard of its own.
{ pkgs }:

let
  lib = pkgs.lib;
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "mocu-cursor-theme";
  # Mocu's own release, matching the vendored sources (see
  # `crates/scoot/src/compositor/cursor/shapes/mocu/README.md`).
  version = "1.1";

  outputs = [
    "out"
    "black"
  ];

  src =
    # Only the SVGs: the directory also carries Mocu's LICENSE.txt and
    # the README, and macOS tarballs of a checkout add `._*`
    # AppleDouble sidecars -- none of which is cursor art. (The build
    # script also skips dotfiles, so a stray sidecar fails loudly at
    # the alias check rather than parsing as XML.)
    builtins.path {
      path = ../crates/scoot/src/compositor/cursor/shapes/mocu;
      filter =
        path: type:
        type != "directory"
        && lib.hasSuffix ".svg" (baseNameOf path)
        && lib.substring 0 2 (baseNameOf path) != "._";
    };

  nativeBuildInputs = [
    pkgs.librsvg
    pkgs.python3
    pkgs.xcursorgen
  ];

  # Data only: PNGs packed into Xcursor files plus symlinks and two
  # `index.theme` files. No binaries, nothing to strip or patch.
  dontFixup = true;

  buildPhase = ''
    runHook preBuild

    python3 - "$src" "$out" "$black" <<'EOF'
    import os
    import re
    import subprocess
    import sys
    import xml.etree.ElementTree as ET

    src_dir, out_white, out_black = sys.argv[1], sys.argv[2], sys.argv[3]
    sizes = (24, 32, 48, 64)
    grid = 24

    # The two paints Mocu authors every cursor with. Only these two
    # spellings are recolored: the shadow (`#0a0b0c`), the hotspot
    # marker (`#f00`, removed below) and the spinner mask's greys stay
    # exactly as authored.
    FILL = "#fafbfc"
    STROKE = "#1a1b1c"
    variants = {
        "white": (out_white, "Mocu", "#fcfcfc", "#222222"),
        "black": (out_black, "Mocu-Black", "#111111", "#f9f9f9"),
    }

    # Alias table: each key is a vendored base name (one `<name>.svg`),
    # each value the extra Xcursor names pointing at it. Covers the
    # freedesktop/CSS name set the compositor looks up (see
    # `crates/scoot/src/compositor/cursor/theme.rs`, which tries each
    # icon's name then its `alt_names`) plus the common legacy X11
    # names (`left_ptr`, `hand2`, `xterm`, `watch`, `fleur`,
    # `sb_h_double_arrow`, the `top_side`/`top_left_corner` sides and
    # corners, ...). `move` draws `grabbing` and `all-resize` draws
    # `all-scroll`, the way Mocu aliases them and the built-in maps
    # them (`Shape::for_icon`).
    aliases = {
        "default": ["left_ptr", "arrow", "top_left_arrow", "X_cursor"],
        "pointer": ["hand2", "hand1", "hand", "pointing_hand"],
        "text": ["xterm", "ibeam"],
        "wait": ["watch"],
        "progress": ["left_ptr_watch"],
        "crosshair": ["cross", "cross_reverse", "diamond_cross", "tcross"],
        "not-allowed": ["crossed_circle", "circle"],
        "grab": ["openhand"],
        "grabbing": ["closedhand", "move", "fleur"],
        "all-scroll": ["all-resize"],
        "col-resize": ["split_h"],
        "row-resize": ["split_v"],
        "ew-resize": ["h_double_arrow", "sb_h_double_arrow", "size_hor"],
        "ns-resize": ["v_double_arrow", "sb_v_double_arrow", "size_ver"],
        "nesw-resize": ["size_bdiag"],
        "nwse-resize": ["size_fdiag"],
        "e-resize": ["right_side", "sb_right_arrow"],
        "w-resize": ["left_side", "sb_left_arrow"],
        "n-resize": ["top_side", "sb_up_arrow"],
        "s-resize": ["bottom_side", "sb_down_arrow"],
        "ne-resize": ["top_right_corner"],
        "nw-resize": ["top_left_corner"],
        "se-resize": ["bottom_right_corner"],
        "sw-resize": ["bottom_left_corner"],
        "help": ["whats_this", "left_ptr_help", "question_arrow"],
        "alias": ["dnd-link", "link"],
        "copy": ["dnd-copy"],
        "no-drop": ["dnd-no-drop", "nodrop"],
        "cell": ["plus"],
    }

    ET.register_namespace("", "http://www.w3.org/2000/svg")
    ET.register_namespace("xlink", "http://www.w3.org/1999/xlink")
    svg_ns = "{http://www.w3.org/2000/svg}"

    bases = sorted(
        f[:-4]
        for f in os.listdir(src_dir)
        if f.endswith(".svg") and not f.startswith(".") and not f.startswith("._")
    )
    assert bases, f"no SVGs in {src_dir}"
    unknown = set(aliases) - set(bases)
    assert not unknown, f"aliases for missing art: {sorted(unknown)}"

    work = os.environ.get("TMPDIR", "/tmp") + "/mocu-pngs"
    os.makedirs(work, exist_ok=True)

    def hotspot(svg_path):
        tree = ET.parse(svg_path)
        for el in tree.getroot().iter(svg_ns + "circle"):
            if el.get("id") == "hot":
                return (float(el.get("cx")), float(el.get("cy")))
        raise ValueError(f"no <circle id=\"hot\"> in {svg_path}")

    def recolored(svg_path, fill, stroke):
        # Element-wise, so only the paints change: the shadow, the
        # spinner mask and the hotspot marker are untouched (the marker
        # is dropped below instead of painted over).
        tree = ET.parse(svg_path)
        count = {"fill": 0, "stroke": 0}
        for el in tree.getroot().iter():
            if el.tag == svg_ns + "circle" and el.get("id") == "hot":
                continue
            style = el.get("style")
            if style:
                decls = []
                for decl in style.split(";"):
                    if ":" not in decl:
                        decls.append(decl)
                        continue
                    k, v = decl.split(":", 1)
                    if v.strip().lower() == FILL and k.strip() == "fill":
                        v = fill
                        count["fill"] += 1
                    elif v.strip().lower() == STROKE and (
                        k.strip() in ("fill", "stroke")
                    ):
                        v = stroke
                        count["stroke"] += 1
                    decls.append(f"{k}:{v}")
                el.set("style", ";".join(decls))
        # The hotspot marker must not render: drop it (xcursorgen takes
        # the hotspot from the config below).
        root = tree.getroot()
        dropped = False
        for parent in list(root.iter()):
            for child in list(parent):
                if child.tag == svg_ns + "circle" and child.get("id") == "hot":
                    parent.remove(child)
                    dropped = True
        assert dropped, f"hotspot marker not removed in {svg_path}"
        return (tree, count)

    for variant, (out, theme, fill, stroke) in variants.items():
        cursor_dir = os.path.join(out, "share", "icons", theme, "cursors")
        os.makedirs(cursor_dir, exist_ok=True)
        with open(
            os.path.join(out, "share", "icons", theme, "index.theme"), "w"
        ) as f:
            f.write(
                "[Icon Theme]\n"
                f"Name={theme}\n"
                f"Comment=Mocu {variant} cursors "
                "(built from the vendored SVGs scoot uses as its built-in cursors)\n"
            )
        for base in bases:
            svg_path = os.path.join(src_dir, base + ".svg")
            hx, hy = hotspot(svg_path)
            tree, count = recolored(svg_path, fill, stroke)
            assert count["fill"] >= 1, f"{base}.svg: no {FILL} fill found"
            assert count["stroke"] >= 1, f"{base}.svg: no {STROKE} paint found"
            staged = os.path.join(work, f"{variant}-{base}.svg")
            tree.write(staged, encoding="unicode", xml_declaration=True)
            conf_lines = []
            for size in sizes:
                png = os.path.join(work, f"{variant}-{base}-{size}.png")
                subprocess.run(
                    ["rsvg-convert", "-w", str(size), "-h", str(size), staged, "-o", png],
                    check=True,
                )
                xhot = int(hx * size / grid + 0.5)
                yhot = int(hy * size / grid + 0.5)
                conf_lines.append(f"{size} {xhot} {yhot} {png}\n")
            conf = os.path.join(work, f"{variant}-{base}.conf")
            with open(conf, "w") as f:
                f.writelines(conf_lines)
            subprocess.run(
                ["xcursorgen", conf, os.path.join(cursor_dir, base)], check=True
            )
        for base, names in aliases.items():
            for name in names:
                target = os.path.join(cursor_dir, base)
                link = os.path.join(cursor_dir, name)
                assert os.path.isfile(target), f"alias target missing: {base}"
                assert not os.path.lexists(link), f"alias collision: {name}"
                os.symlink(base, link)
    EOF

    runHook postBuild
  '';

  # No `installPhase` copy: the build writes straight into `$out` and
  # `$black`.
  installPhase = "runHook preInstall; runHook postInstall;";

  meta = with lib; {
    description = "Mocu white and black XCursor themes, built from scoot's vendored SVGs";
    homepage = "https://github.com/sevmeyer/mocu-xcursor";
    license = licenses.cc0;
    platforms = platforms.linux;
  };
}
