# scoot: the scrolling-tiling Wayland compositor.
#
# Built from the vendored-sources release tarball (scripts/vendor-release.sh:
# the tagged tree plus vendor/, .cargo/config.toml and THIRD-PARTY-LICENSES),
# the default cargo feature set: the GPU-free compositor that runs on a box
# with no GPU stack installed. The `gpu-scanout` and `xwayland` tiers are
# compile-time features; rebuilding with them is documented in
# packaging/rpm/README.md.
#
# The versioned `scootbg` requirement is the coupled pair from
# docs/backlog/packaging/rpm-package.md: scoot and scootbg move in lockstep
# (docs/versioning.md), so the pair installs at one version.
#
# No release is cut yet, so Source0 points at the release asset the release
# job will publish; until it exists CI builds this exact file against a
# staged tarball of HEAD placed under the same basename (see
# .github/workflows/rpm.yml). The basename below is provisional until the
# first release names the asset; only that line changes then.

Name:           scoot
Version:        0.1.0
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: the vendored crate sources
# carry their upstream modes (executable .rs files, a `#!` attribute
# line that is not a shebang), which trips brp-mangle-shebangs over
# /usr/src/debug. Revisit with a mode-normalizing %prep if distro-grade
# inclusion ever wants debuginfo (packaging/rpm/README.md).
%global         debug_package %{nil}
Summary:        Scrolling-tiling Wayland compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/scoot-v%{version}/scoot-%{version}-vendored.tar.gz
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
BuildRequires:  desktop-file-utils
BuildRequires:  systemd-rpm-macros
%else
BuildRequires:  libinput-devel
BuildRequires:  libseat-devel
BuildRequires:  systemd-devel
BuildRequires:  pixman-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  desktop-file-utils
BuildRequires:  systemd-rpm-macros
%endif
# The coupled pair installs at one version (docs/versioning.md).
Requires:       scootbg = %{version}-%{release}

%description
scoot is a scrolling-tiling Wayland compositor that renders on the CPU
by default, so it runs with no GPU stack installed. This package is the
compositor binary plus the greeter launcher: pick "scoot" on the login
screen, or run `scoot --headless` for a headless session. Copy
/usr/share/doc/scoot/config.toml.example to ~/.config/scoot/config.toml
to start from the built-in defaults.

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
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES
%doc source/config.toml.example

%changelog
* Thu Oct 08 2026 scoot-sh <packages@scoot.sh> - 0.1.0-1
- First RPM: compositor plus greeter launcher, from the vendored tarball.
