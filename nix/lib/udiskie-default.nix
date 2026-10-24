# The desktop profile's udiskie default, shared by the NixOS side
# (`nixos.nix`) and the home-manager side (`apps-home.nix`): udiskie
# with its flaky kernel-keyring tests off. Those tests
# (`test/test_keyutils.py`, `test_cache::test_is_valid`) need a
# working kernel keyring and fail without one (Docker seccomp EPERM;
# `ValueError: bytes length not a multiple of item size`), which
# failed the whole check on a package this repo only reads. Narrower
# than `doCheck = false`: the rest of the suite still runs.
#
# Takes `pkgs` (both sides have it; `desktop.nix` stays `lib`-only).
# Null where udiskie is absent, so the Linux guard in the callers
# keeps its fail-loud refusal and off-Linux stays null without
# forcing the attribute.
{ pkgs }:

if !(pkgs ? udiskie) then
  null
else
  pkgs.udiskie.overridePythonAttrs {
    disabledTestPaths = [ "test/test_keyutils.py" ];
    disabledTests = [ "test_is_valid" ];
  }
