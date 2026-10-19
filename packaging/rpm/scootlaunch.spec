# scootlaunch: the keyboard-first application launcher for Wayland.
#
# Built from the same release tag as every spec here (`v%%{version}`): one
# tag, one version for every binary (docs/versioning.md). The panel links
# libxkbcommon for its keyboard (the reference XKB implementation); the
# headless list and query commands need nothing past the C library.
# Runtime requirements for that soname are auto-generated from the binary
# at build time, so only the `-devel` name below is written here. It
# speaks standard Wayland protocols, so it needs no versioned dependency
# on any compositor package.
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here unchanged (the same release tarball, under the same name).

Name:           scootlaunch
Version:        0.2.1
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: see scoot.spec.
%global         debug_package %{nil}
Summary:        Keyboard-first application launcher for the scoot compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/v%{version}/scoot-%{version}-vendored.tar.gz
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  libxkbcommon-devel

%description
scootlaunch is the keyboard-first application launcher for the scoot
compositor, and runs standalone under any compositor speaking standard
Wayland protocols. It ranks desktop entries as you type, launches them
with an activation token in their own systemd scope, and also drives
dmenu-style picks and headless `--list --json` queries. A missing
config file is the built-in defaults: check one with `scootlaunch
--check`.

%prep
# Same vendored-tarball layout as scoot.spec: ./source/, not ./Name-Version.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scootlaunch

%install
cd source
install -Dm755 target/release/scootlaunch %{buildroot}%{_bindir}/scootlaunch

# Stripped explicitly: see scoot.spec (no find-debuginfo run strips them).
%{__strip} %{buildroot}%{_bindir}/scootlaunch

%check
# Headless-safe: the binary starts, and --check on the built-in defaults
# (no config file) exits 0.
cd source
./target/release/scootlaunch --version
./target/release/scootlaunch --check

%files
%{_bindir}/scootlaunch
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES

%changelog
* Mon Oct 19 2026 scoot-sh <packages@scoot.sh> - 0.2.1-1
- First RPM: launcher plus its keyboard library, from the vendored tarball.
