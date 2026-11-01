# scoot: the scrolling-tiling Wayland compositor.
#
# Built from the vendored-sources release tarball (scripts/vendor-release.sh:
# the tagged tree plus vendor/, .cargo/config.toml and THIRD-PARTY-LICENSES),
# the default cargo feature set: the single build, which runs on a box with
# no GPU stack installed and scans out from the GPU where there is one (the
# `gpu-scanout` tier is on by default and reaches libgbm at run time, so no
# libgbm link dependency). The `xwayland` tier stays a compile-time feature;
# rebuilding with it is documented in packaging/rpm/README.md. The default
# build compiles a small C stub (`crates/scoot/gbm-stub/`), hence
# `BuildRequires: gcc` below.
#
# The versioned `scootbg` requirement is the coupled pair from
# docs/backlog/resolved/rpm-package-done.md: one tag (`v%%{version}`) releases
# every binary at the repository's one version (docs/versioning.md), so
# the pair installs at that version.
#
# Source0 is the vendored tarball every release publishes under this exact
# name: release.yml runs `scripts/vendor-release.sh --label <version>`, so
# the asset is `scoot-<version>-vendored.tar.gz` (one tarball for all
# four specs). No release is cut yet, so the URL 404s; until it exists CI
# builds this exact file against a staged tarball of HEAD placed under the
# same name (see .github/workflows/rpm.yml).

Name:           scoot
Version:        0.6.26
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: the vendored crate sources
# carry their upstream modes (executable .rs files, a `#!` attribute
# line that is not a shebang), which trips brp-mangle-shebangs over
# /usr/src/debug. Revisit with a mode-normalizing %%prep if distro-grade
# inclusion ever wants debuginfo (packaging/rpm/README.md).
%global         debug_package %{nil}
Summary:        Scrolling-tiling Wayland compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/v%{version}/scoot-%{version}-vendored.tar.gz
BuildRequires:  cargo
BuildRequires:  gcc
%if 0%{?suse_version}
BuildRequires:  libinput-devel
BuildRequires:  seatd-devel
BuildRequires:  systemd-devel
BuildRequires:  libpixman-1-0-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  systemd-rpm-macros
%else
BuildRequires:  libinput-devel
BuildRequires:  libseat-devel
BuildRequires:  systemd-devel
BuildRequires:  pixman-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  systemd-rpm-macros
%endif
# The coupled pair installs at the one version (docs/versioning.md).
Requires:       scootbg = %{version}-%{release}
# The GPU scanout tier is compiled in and reached at run time (no libgbm
# link dependency): these are auto-detected, never required. Names from
# `dnf repoquery --whatprovides` on Fedora 43 (packaging/rpm/README.md);
# the openSUSE names are not probed, so that side stays a comment.
%if 0%{?suse_version}
# openSUSE runtime providers of libgbm.so.1 / libdrm.so.2: see README.md.
%else
Recommends:     mesa-libgbm
Recommends:     libdrm
%endif

%description
scoot is a scrolling-tiling Wayland compositor that picks CPU or GPU drawing
at run time, so the same binary runs with no GPU stack installed and scans
out from the GPU on real hardware. This package is the
compositor binary plus the greeter launcher and the systemd user units
it starts (scoot.service, scoot-session.target, scoot-shutdown.target)
with the portal selection: pick "scoot" on the login
screen, or run `scoot --headless` for a headless session. Copy the
shipped example into ~/.config/scoot/ to start from the built-in
defaults (its `[wallpaper]` section drives the daemon, so that package
ships no separate example).

%prep
# The vendored tarball unpacks to ./source/ (scripts/vendor-release.sh),
# not ./Name-Version, so unpack into a private dir and work from source/.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml, so
# a build that reaches the network fails instead of fetching.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scoot
./target/release/scoot --print-default-config > config.toml.example

%install
cd source
install -Dm755 target/release/scoot %{buildroot}%{_bindir}/scoot
install -Dm755 resources/scoot-session %{buildroot}%{_bindir}/scoot-session
install -Dm644 packaging/rpm/scoot.desktop %{buildroot}%{_datadir}/wayland-sessions/scoot.desktop
# The session units scoot-session starts (single-sourced from
# resources/systemd/user/, the same files the NixOS module installs):
# the service names the packaged binary, the two targets name no
# binary. Plus the portal selection for the vendor slot (see
# resources/scoot-portals.conf).
install -Dm644 resources/systemd/user/scoot-session.target %{buildroot}%{_userunitdir}/scoot-session.target
install -Dm644 resources/systemd/user/scoot-shutdown.target %{buildroot}%{_userunitdir}/scoot-shutdown.target
sed 's|@SCOOT_BIN@|%{_bindir}/scoot|g' resources/systemd/user/scoot.service > %{buildroot}%{_userunitdir}/scoot.service
chmod 644 %{buildroot}%{_userunitdir}/scoot.service
install -Dm644 resources/scoot-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/scoot-portals.conf

# Stripped explicitly: with `debug_package %%{nil}` above there is no
# find-debuginfo run, which is what normally strips the binaries, so an
# unstripped build would ship its symtab (the workspace already builds
# `strip = true`; this covers the RPM-added flags' debuginfo). Only the
# ELF binary: scoot-session beside it is a shell script.
%{__strip} %{buildroot}%{_bindir}/scoot

%check
# Headless-safe: version and the default-config render (which covers the
# [wallpaper] handoff); nothing that opens a display.
cd source
./target/release/scoot --version
./target/release/scoot --print-default-config > /dev/null

%files
%{_bindir}/scoot
%{_bindir}/scoot-session
%{_datadir}/wayland-sessions/scoot.desktop
%{_userunitdir}/scoot.service
%{_userunitdir}/scoot-session.target
%{_userunitdir}/scoot-shutdown.target
%{_datadir}/xdg-desktop-portal/scoot-portals.conf
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES
%doc source/config.toml.example

%changelog
* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.26-1
- Built from the v0.6.26 release tag.

* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.25-1
- Built from the v0.6.25 release tag.

* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.24-1
- Built from the v0.6.24 release tag.

* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.23-1
- Built from the v0.6.23 release tag.

* Sat Oct 31 2026 scoot-sh <packages@scoot.sh> - 0.6.22-1
- Built from the v0.6.22 release tag.

* Sat Oct 31 2026 scoot-sh <packages@scoot.sh> - 0.6.21-1
- Built from the v0.6.21 release tag.

* Sat Oct 31 2026 scoot-sh <packages@scoot.sh> - 0.6.20-1
- Built from the v0.6.20 release tag.

* Sat Oct 31 2026 scoot-sh <packages@scoot.sh> - 0.6.19-1
- Built from the v0.6.19 release tag.

* Sat Oct 31 2026 scoot-sh <packages@scoot.sh> - 0.6.18-1
- Built from the v0.6.18 release tag.

* Fri Oct 30 2026 scoot-sh <packages@scoot.sh> - 0.6.17-1
- Built from the v0.6.17 release tag.

* Fri Oct 30 2026 scoot-sh <packages@scoot.sh> - 0.6.16-1
- Built from the v0.6.16 release tag.

* Fri Oct 30 2026 scoot-sh <packages@scoot.sh> - 0.6.15-1
- Built from the v0.6.15 release tag.

* Thu Oct 29 2026 scoot-sh <packages@scoot.sh> - 0.6.14-1
- Built from the v0.6.14 release tag.

* Thu Oct 29 2026 scoot-sh <packages@scoot.sh> - 0.6.13-1
- Built from the v0.6.13 release tag.

* Thu Oct 29 2026 scoot-sh <packages@scoot.sh> - 0.6.12-1
- Built from the v0.6.12 release tag.

* Wed Oct 28 2026 scoot-sh <packages@scoot.sh> - 0.6.11-1
- Built from the v0.6.11 release tag.

* Wed Oct 28 2026 scoot-sh <packages@scoot.sh> - 0.6.10-1
- Built from the v0.6.10 release tag.

* Tue Oct 27 2026 scoot-sh <packages@scoot.sh> - 0.6.9-1
- Built from the v0.6.9 release tag.

* Tue Oct 27 2026 scoot-sh <packages@scoot.sh> - 0.6.8-1
- Built from the v0.6.8 release tag.

* Tue Oct 27 2026 scoot-sh <packages@scoot.sh> - 0.6.7-1
- Built from the v0.6.7 release tag.

* Tue Oct 27 2026 scoot-sh <packages@scoot.sh> - 0.6.6-1
- Built from the v0.6.6 release tag.

* Sun Oct 25 2026 scoot-sh <packages@scoot.sh> - 0.6.5-1
- Built from the v0.6.5 release tag.

* Sun Oct 25 2026 scoot-sh <packages@scoot.sh> - 0.6.4-1
- Built from the v0.6.4 release tag.

* Sun Oct 25 2026 scoot-sh <packages@scoot.sh> - 0.6.3-1
- Built from the v0.6.3 release tag.

* Sat Oct 24 2026 scoot-sh <packages@scoot.sh> - 0.6.2-1
- Built from the v0.6.2 release tag.

* Sat Oct 24 2026 scoot-sh <packages@scoot.sh> - 0.6.1-1
- Built from the v0.6.1 release tag.

* Fri Oct 23 2026 scoot-sh <packages@scoot.sh> - 0.6.0-1
- Built from the v0.6.0 release tag.

* Wed Oct 21 2026 scoot-sh <packages@scoot.sh> - 0.5.0-1
- Built from the v0.5.0 release tag.

* Tue Oct 20 2026 scoot-sh <packages@scoot.sh> - 0.4.1-1
- Built from the v0.4.1 release tag.

* Tue Oct 20 2026 scoot-sh <packages@scoot.sh> - 0.4.0-1
- Built from the v0.4.0 release tag.

* Wed Oct 14 2026 scoot-sh <packages@scoot.sh> - 0.2.1-1
- Built from the v0.2.1 release tag.

* Tue Oct 13 2026 scoot-sh <packages@scoot.sh> - 0.2.0-1
- One version for every binary: built from the v0.2.0 release tag.

* Thu Oct 08 2026 scoot-sh <packages@scoot.sh> - 0.1.0-1
- First RPM: compositor plus greeter launcher, from the vendored tarball.
