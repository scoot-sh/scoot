# The Nix side of the look catalog (design section 6.2), shared by the
# home-manager module (`nix/modules/home.nix`) and the NixOS module
# (`nix/modules/nixos.nix`).
#
# `looks` (names of shipped looks or paths to source look directories)
# normalizes to `{ name, src, manifest }` entries, first wins; each entry
# renders at build time through the `scoot-look` binary into
# `rendered/<name>`. No derivations run at evaluation (sources read with
# `builtins.fromTOML`, font carriers resolved with `lib.attrByPath`
# against `pkgs`), and no import-from-derivation: every derivation input
# is a source path or a package.
#
# Takes `{ lib, pkgs, scootLook, fallbackUiPackage, fallbackFont }`:
# `scootLook` is the `scoot-look` binary package (null skips nothing
# here — the caller gates on it); `fallbackUiPackage` is the theme's
# default bar-face package for looks without their own `[fonts.nix]`
# carrier (null off Linux); `fallbackFont` is the same face as a file
# for the font-file derivation's fallback (null off Linux).
{
  lib,
  pkgs,
  scootLook,
  fallbackUiPackage,
  fallbackFont,
}:

let
  # The shipped look names, from the registry (sorted, like every
  # directory scan in the catalog: order among declared entries is the
  # catalog's own, and the runtime scan sorts within a directory).
  shippedNames = builtins.attrNames (import ./modules/desktop.nix { inherit lib; }).looks;

  # Whether `value` is a look name (`[a-z0-9-]{1,32}`, the grammar
  # `scoot-look` enforces).
  isName = value: builtins.isString value && builtins.match "[a-z0-9-]{1,32}" value != null;

  # The manifest of a source directory: parsed at evaluation, so a
  # malformed `look.toml` fails the evaluation naming the file — before
  # anything builds, let alone starts a session. Stays a path value
  # (`src + "/look.toml"`, never string concatenation: a
  # string-concatenated absolute path is forbidden in pure evaluation).
  readManifest = src: builtins.fromTOML (builtins.readFile (src + "/look.toml"));

  # One `looks` entry to `{ name, src, manifest }`: a shipped name, a
  # source directory (paths, or an absolute store path as a string —
  # interpolation turns paths into strings), or a loud refusal. The
  # manifest's own `name` wins over the directory basename, which
  # carries a hash prefix in the store — the same rule the runtime
  # catalog scan follows.
  toEntry =
    item:
    let
      asPath =
        if builtins.isPath item then
          item
        else if builtins.isString item && lib.hasPrefix "/" item then
          item
        else
          null;
    in
    if builtins.isString item && !(lib.hasPrefix "/" item) then
      if builtins.elem item shippedNames then
        let
          src = ../looks/${item};
          manifest = readManifest src;
        in
        {
          name = item;
          inherit src manifest;
        }
      else
        throw "programs.scoot.looks: unknown shipped look `${item}` (shipped: ${lib.concatStringsSep ", " shippedNames}; a directory goes as a path, not a string)"
    else if asPath != null then
      let
        # A path value, never a string: only path values (plus `+`)
        # read in pure evaluation.
        src = if builtins.isPath asPath then asPath else /. + asPath;
        manifest = readManifest src;
        name = manifest.name or (throw "programs.scoot.looks: ${toString src}/look.toml names no `name`");
      in
      if !(isName name) then
        throw "programs.scoot.looks: ${toString src}/look.toml `name` is not a look name ([a-z0-9-]{1,32}): `${name}`"
      else
        { inherit name src manifest; }
    else
      throw "programs.scoot.looks takes shipped look names (strings) or source look directories (paths), got ${builtins.typeOf item}";

  # First wins, so a look listed twice resolves deterministically (the
  # runtime catalog's shadow rule, at evaluation time).
  dedupe =
    entries:
    builtins.foldl' (
      acc: entry: if builtins.elem entry.name (map (done: done.name) acc) then acc else acc ++ [ entry ]
    ) [ ] entries;

  mkEntries = looks: dedupe (map toEntry looks);

  # An entry by name, for `extends` walks below.
  findEntry = entries: name: lib.findFirst (entry: entry.name == name) null entries;

  # The first non-null `get entry` walking down the `extends` chain
  # from `entry` (the child wins, mirroring `resolve`), with the entry
  # that named it. Unknown bases and cycles stop the walk with null —
  # the build (`scoot-look build --catalog`) refuses those loudly, so
  # this never silently renders the wrong file.
  chainField =
    entries: entry: get:
    let
      walk =
        current: depth:
        let
          value = get current;
        in
        if value != null then
          {
            entry = current;
            inherit value;
          }
        else if depth >= 8 then
          null
        else
          let
            base = current.manifest.extends or null;
            next = if base == null then null else findEntry entries base;
          in
          if next == null then null else walk next (depth + 1);
    in
    walk entry 0;

  # The `[wallpaper]` image for the render, with the entry whose unit
  # names it: `[wallpaper]` swaps whole (a child table without an
  # image is color-only, never the base's image).
  chainWallpaper =
    entries: entry:
    let
      walk =
        current: depth:
        let
          table = current.manifest.wallpaper or null;
        in
        if table != null then
          {
            entry = current;
            image = table.image or null;
          }
        else if depth >= 8 then
          null
        else
          let
            base = current.manifest.extends or null;
            next = if base == null then null else findEntry entries base;
          in
          if next == null then null else walk next (depth + 1);
    in
    walk entry 0;

  # The bar's UI face as a file, found at build time in the carrier
  # package's `share/fonts`: the regular face whose file name is the
  # family name, or that name plus `-Regular` (the same rule the bar
  # module's `fontFile` uses, so the two never disagree). A look
  # bundling the face in `[fonts.files]` uses that file instead; without
  # either, the render carries no `--ui-font-file` and the runtime
  # resolves the family through fontconfig. Unresolvable falls back to
  # the fallback face with a build-time warning (in the log and in
  # `$out/warning`), never a failed build.
  uiFontFile =
    entries: entry:
    let
      family =
        (chainField entries entry (candidate: (candidate.manifest.fonts or { }).ui or null)).value or null;
      filesHit = chainField entries entry (
        candidate: ((candidate.manifest.fonts or { }).files or { }).ui or null
      );
      filesRel = if filesHit == null then null else filesHit.value;
      filesDir = if filesHit == null then null else filesHit.entry.src;
      bundled =
        if filesRel == null then
          null
        else
          let
            file = filesDir + "/${filesRel}";
          in
          if builtins.pathExists file then
            file
          else
            throw "programs.scoot.looks: ${entry.name} [fonts.files] `ui` is missing: ${filesRel}";
      nixHit = chainField entries entry (
        candidate: ((candidate.manifest.fonts or { }).nix or { }).ui or null
      );
      package =
        if bundled != null then
          null
        else if nixHit != null then
          let
            parts = lib.splitString "." nixHit.value;
          in
          lib.attrByPath parts
            (throw "programs.scoot.looks: [fonts.nix] `${nixHit.value}` resolves to nothing in pkgs")
            pkgs
        else
          fallbackUiPackage;
    in
    if family == null then
      null
    else if bundled != null then
      bundled
    else if package == null then
      null
    else
      pkgs.runCommand "scoot-look-font-${lib.strings.sanitizeDerivationName entry.name}"
        {
          nativeBuildInputs = [ pkgs.findutils ];
          inherit family;
          fontPackage = package;
          # Null off Linux (no fallback face there): the build refuses
          # instead of interpolating a null.
          fallback = if fallbackFont == null then "" else fallbackFont;
        }
        ''
          norm() { tr -d ' _-' | tr '[:upper:]' '[:lower:]'; }
          want=$(printf '%s' "$family" | norm)
          files=$(find -L "$fontPackage/share/fonts" -type f \( -iname '*.ttf' -o -iname '*.otf' \) 2>/dev/null | sort || true)
          pick=
          while IFS= read -r file; do
            [ -n "$file" ] || continue
            base=$(basename "$file"); base=''${base%.*}
            have=$(printf '%s' "$base" | norm)
            if [ "$have" = "$want" ] || [ "$have" = "''${want}regular" ]; then
              pick=$file
              break
            fi
          done <<<"$files"
          mkdir $out
          : > $out/warning
          if [ -z "$pick" ]; then
            if [ -n "$fallback" ]; then
              msg="scoot-look: no regular-face file named for \"$family\" in $fontPackage/share/fonts; using the fallback face."
              echo "warning: $msg" >&2
              echo "$msg" > $out/warning
              pick=$fallback
            else
              echo "error: scoot-look: no regular-face file named for \"$family\" and no fallback face" >&2
              exit 1
            fi
          fi
          ln -s "$pick" $out/font
        '';

  # The wallpaper's absolute build-time path, or null: a color-only look
  # needs none, and a `https://` image fetches at first swap (the design's
  # section 8.3), never in the sandbox. A relative image must exist in
  # the source whose unit names it, or the evaluation fails naming it.
  wallpaperAbs =
    entries: entry:
    let
      hit = chainWallpaper entries entry;
      dir = if hit == null then null else hit.entry.src;
      image = if hit == null then null else hit.image;
    in
    if image == null then
      null
    else if lib.hasPrefix "https://" image then
      null
    else
      let
        file = dir + "/${image}";
      in
      if builtins.pathExists file then
        file
      else
        throw "programs.scoot.looks: ${entry.name} wallpaper image is missing: ${image}";

  # Every source in one directory, so each render resolves `extends`
  # through the whole declared set (`scoot-look build --catalog`).
  catalogDir =
    entries:
    pkgs.runCommand "scoot-looks-sources" { } ''
      mkdir -p $out
      ${lib.concatMapStringsSep "\n" (
        entry: "ln -s \"${entry.src}\" $out/${lib.escapeShellArg entry.name}"
      ) entries}
    '';

  # One look rendered at build time: `scoot.toml`, `bar.toml` and
  # `render.json`, validated the same way `validate` validates (a parse
  # error, a forbidden key or a missing file fails the build before
  # anything changes; a contrast shortfall in a look the user's own
  # configuration chose only warns). The bar renders with its `popup`
  # keys: a bar without that feature drops them at sync time, against
  # the installed binary.
  renderLook =
    sources: entries: entry:
    let
      font = uiFontFile entries entry;
      wallpaper = wallpaperAbs entries entry;
    in
    pkgs.runCommand "scoot-look-${lib.strings.sanitizeDerivationName entry.name}-rendered"
      {
        nativeBuildInputs = [ scootLook ];
      }
      ''
        mkdir -p $out
        scoot-look build "${entry.src}" --out "$out" --name ${lib.escapeShellArg entry.name} --origin declared --catalog "${sources}" \
          ${lib.optionalString (wallpaper != null) "--wallpaper-abs \"${wallpaper}\""} \
          ${lib.optionalString (font != null) "--ui-font-file \"${font}/font\""}
      '';

  mkCatalog =
    looks:
    let
      entries = mkEntries looks;
    in
    if scootLook == null then
      # No render binary: the catalog side stays off (no sources, no
      # renders, no declared default, no activation) — see
      # `lookPackage`. Entries and wallpaper knowledge still evaluate,
      # so the wallpaper package follows catalog wallpapers regardless.
      {
        inherit entries;
        sources = null;
        rendered = { };
        names = map (entry: entry.name) entries;
        hasWallpaper = builtins.any (
          entry: ((entry.manifest.wallpaper or { }).image or null) != null
        ) entries;
      }
    else
      let
        sources = catalogDir entries;
      in
      {
        inherit entries sources;
        rendered = lib.listToAttrs (
          map (entry: {
            name = entry.name;
            value = renderLook sources entries entry;
          }) entries
        );
        names = map (entry: entry.name) entries;
        hasWallpaper = builtins.any (
          entry: ((entry.manifest.wallpaper or { }).image or null) != null
        ) entries;
      };
in
{
  inherit shippedNames mkEntries mkCatalog;
}
