# scootbar: the status bar for the scoot compositor.
#
# Built from the same release tag as every spec here (`v%%{version}`): one
# tag, one version for every binary (docs/versioning.md). The daemon links
# nothing past the C library, so it carries no system-library dependencies;
# it speaks standard Wayland protocols, so it needs no versioned dependency
# on any compositor package. The systemd user unit restarts it on failure
# (`Restart=on-failure`, the same policy as the Nix home-manager module in
# nix/modules/scootbar-home.nix).
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here unchanged (the same release tarball, under the same name).

Name:           scootbar
Version:        0.6.16
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: see scoot.spec.
%global         debug_package %{nil}
Summary:        Status bar for the scoot compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/v%{version}/scoot-%{version}-vendored.tar.gz
BuildRequires:  cargo
BuildRequires:  gcc
%if 0%{?suse_version}
BuildRequires:  systemd-rpm-macros
%else
BuildRequires:  systemd-rpm-macros
%endif

%description
scootbar is the status bar for the scoot compositor, and runs standalone
under any compositor speaking standard Wayland protocols. Copy the
shipped example into ~/.config/scoot/ and check it with `scootbar
daemon --check` (prints OK).

%prep
# Same vendored-tarball layout as scoot.spec: ./source/, not ./Name-Version.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scootbar

%install
cd source
install -Dm755 target/release/scootbar %{buildroot}%{_bindir}/scootbar
install -Dm644 packaging/rpm/scootbar.service %{buildroot}%{_userunitdir}/scootbar.service

# Stripped explicitly: see scoot.spec (no find-debuginfo run strips them).
%{__strip} %{buildroot}%{_bindir}/scootbar

%check
# Headless-safe: the binary starts, and the shipped example passes --check.
cd source
./target/release/scootbar --version
./target/release/scootbar daemon --check --config packaging/rpm/bar.toml.example

%post
%systemd_user_post scootbar.service

%preun
%systemd_user_preun scootbar.service

%files
%{_bindir}/scootbar
%{_userunitdir}/scootbar.service
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES
%doc source/packaging/rpm/bar.toml.example

%changelog
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
- First RPM: status bar plus user unit, from the vendored tarball.
