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
Version:        0.6.8
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
with an activation token in their own service scope, and also answers
menu-style picks from standard input and headless list queries. A
missing config file is the built-in defaults: check one with
`scootlaunch --check`.

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

* Mon Oct 19 2026 scoot-sh <packages@scoot.sh> - 0.2.1-1
- First RPM: launcher plus its keyboard library, from the vendored tarball.
