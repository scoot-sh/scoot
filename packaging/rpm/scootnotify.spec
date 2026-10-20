# scootnotify: the notification daemon for Wayland.
#
# Built from the same release tag as every spec here (`v%%{version}`): one
# tag, one version for every binary (docs/versioning.md). The daemon links
# nothing past the C library, so it carries no system-library
# dependencies; it speaks standard Wayland protocols, so it needs no
# versioned dependency on any compositor package. The systemd user unit
# restarts it on failure (`Restart=on-failure`, plus
# `RestartPreventExitStatus=3` so a taken name stops the unit instead of
# restarting against its owner, the same policy as the Nix home-manager
# module in nix/modules/scootnotify-home.nix).
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here unchanged (the same release tarball, under the same name).

Name:           scootnotify
Version:        0.4.1
Release:        1%{?dist}
# No -debuginfo/-debugsource subpackages: see scoot.spec.
%global         debug_package %{nil}
Summary:        Notification daemon for the scoot compositor
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
scootnotify is the notification daemon for the scoot compositor, and
runs standalone under any desktop with a session bus. It shows one card
per notification on an on-demand popup surface, answers replacements
and dismissals, keeps a history ring with do-not-disturb and
per-sender rules, and also answers status queries over its control
socket. A missing config file is the built-in defaults: check one with
`scootnotify daemon --check`.

%prep
# Same vendored-tarball layout as scoot.spec: ./source/, not ./Name-Version.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scootnotify

%install
cd source
install -Dm755 target/release/scootnotify %{buildroot}%{_bindir}/scootnotify
install -Dm644 packaging/rpm/scootnotify.service %{buildroot}%{_userunitdir}/scootnotify.service
install -Dm644 packaging/rpm/org.freedesktop.Notifications.service %{buildroot}%{_datadir}/dbus-1/services/org.freedesktop.Notifications.service

# Stripped explicitly: see scoot.spec (no find-debuginfo run strips them).
%{__strip} %{buildroot}%{_bindir}/scootnotify

%check
# Headless-safe: the binary starts, and the shipped example passes
# `daemon --check` (validating the config file needs no session bus).
cd source
./target/release/scootnotify --version
./target/release/scootnotify daemon --check
./target/release/scootnotify daemon --check --config packaging/rpm/notify.toml.example

%post
%systemd_user_post scootnotify.service

%preun
%systemd_user_preun scootnotify.service

%files
%{_bindir}/scootnotify
%{_userunitdir}/scootnotify.service
%{_datadir}/dbus-1/services/org.freedesktop.Notifications.service
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES
%doc source/packaging/rpm/notify.toml.example

%changelog
* Tue Oct 20 2026 scoot-sh <packages@scoot.sh> - 0.4.1-1
- Built from the v0.4.1 release tag.

* Tue Oct 20 2026 scoot-sh <packages@scoot.sh> - 0.4.0-1
- Built from the v0.4.0 release tag.
