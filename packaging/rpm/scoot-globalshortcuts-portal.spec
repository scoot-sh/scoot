# scoot-globalshortcuts-portal: the GlobalShortcuts portal backend for Wayland.
#
# Built from the same release tag as every spec here (`v%%{version}`): one
# tag, one version for every binary (docs/versioning.md). The backend links
# nothing past the C library, so it carries no system-library
# dependencies; it registers shortcuts with the scoot compositor over its
# IPC socket, so it needs no versioned dependency on any compositor
# package. The systemd user unit restarts it on failure
# (`Restart=on-failure`, plus `RestartPreventExitStatus=3` so a taken name
# stops the unit instead of restarting against its owner, the same policy
# as the Nix home-manager module in
# nix/modules/scoot-globalshortcuts-portal-home.nix).
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here unchanged (the same release tarball, under the same name).

Name:           scoot-globalshortcuts-portal
Version:        0.6.29
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: see scoot.spec.
%global         debug_package %{nil}
Summary:        GlobalShortcuts portal for the scoot compositor
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
scoot-globalshortcuts-portal lets apps ask for global shortcuts on the
scoot compositor through the GlobalShortcuts portal. It registers each
shortcut with the compositor and turns key presses and releases into the
portal's Activated, Deactivated and ShortcutsChanged signals. Shortcut
assignment lives in the compositor's [global_shortcuts] config.

%prep
# Same vendored-tarball layout as scoot.spec: ./source/, not ./Name-Version.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scoot-globalshortcuts-portal

%install
cd source
install -Dm755 target/release/scoot-globalshortcuts-portal %{buildroot}%{_bindir}/scoot-globalshortcuts-portal
install -Dm644 packaging/rpm/scoot-globalshortcuts-portal.service %{buildroot}%{_userunitdir}/scoot-globalshortcuts-portal.service
install -Dm644 packaging/rpm/org.freedesktop.impl.portal.desktop.scoot.service %{buildroot}%{_datadir}/dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service
install -Dm644 packaging/rpm/scoot.portal %{buildroot}%{_datadir}/xdg-desktop-portal/portals/scoot.portal

# Stripped explicitly: see scoot.spec (no find-debuginfo run strips them).
%{__strip} %{buildroot}%{_bindir}/scoot-globalshortcuts-portal

%check
# Headless-safe: the binary starts and prints its help and version
# (serving needs a session bus, so `%check` stops here).
cd source
./target/release/scoot-globalshortcuts-portal --version
./target/release/scoot-globalshortcuts-portal --help

%post
%systemd_user_post scoot-globalshortcuts-portal.service

%preun
%systemd_user_preun scoot-globalshortcuts-portal.service

%files
%{_bindir}/scoot-globalshortcuts-portal
%{_userunitdir}/scoot-globalshortcuts-portal.service
%{_datadir}/dbus-1/services/org.freedesktop.impl.portal.desktop.scoot.service
%{_datadir}/xdg-desktop-portal/portals/scoot.portal
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES

%changelog
* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.29-1
- Built from the v0.6.29 release tag.

* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.28-1
- Built from the v0.6.28 release tag.

* Sun Nov 01 2026 scoot-sh <packages@scoot.sh> - 0.6.27-1
- Built from the v0.6.27 release tag.

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
