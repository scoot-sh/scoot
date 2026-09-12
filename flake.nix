{
  description = "flexwm: a scrolling-tiling Wayland window manager that runs without a GPU";

  # Pinned to the same nixpkgs revision as vm/, so the dev shell and the VM
  # agree on every library and nothing is downloaded twice.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/8ce4ef6cb6f871616146b9fe26d2a5ae594e94fe";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forEach = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forEach (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = [
            pkgs.rustc
            pkgs.cargo
            pkgs.clippy
            pkgs.rustfmt
            pkgs.pkg-config
          ]
          # Not on Linux: the VM's system closure deliberately drops
          # rust-analyzer to keep erofs packing fast, and `nix develop` there
          # should stay a subset of that closure -- nothing new to fetch.
          ++ pkgs.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.rust-analyzer;
          # The compositor's C dependencies exist only on Linux; the core, the
          # IPC crate and `flexwm msg` build anywhere.
          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            import ./vm/compositor-deps.nix pkgs
          );

          # `nix develop`'s setup hooks already add -L for every buildInput, so
          # this shellHook only matters inside the dev shell itself: a couple
          # of the compositor's -sys crates (xkbcommon-sys, pixman-sys) probe
          # via pkg-config for cflags but link with a bare -lfoo, which has
          # nothing to find on NixOS without an explicit LIBRARY_PATH. (The
          # VM's system profile hits the same gap outside any dev shell --
          # that's fixed separately, in configuration.nix.)
          shellHook = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            export LIBRARY_PATH="''${LIBRARY_PATH:+$LIBRARY_PATH:}${pkgs.lib.makeLibraryPath (import ./vm/compositor-deps.nix pkgs)}"
          '';
        };
      });

      formatter = forEach (pkgs: pkgs.nixfmt-rfc-style);
    };
}
