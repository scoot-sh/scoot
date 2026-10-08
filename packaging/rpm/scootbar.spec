# scootbar: the status bar for the scoot compositor.
#
# Built from the bar's own tag (`scootbar-v%%{version}`): the bar versions
# independently of the compositor (docs/versioning.md). The daemon links
# nothing past the C library, so it carries no system-library dependencies;
# it speaks standard Wayland protocols, so it needs no versioned dependency
# on any compositor package. The systemd user unit restarts it on failure
# (`Restart=on-failure`, the same policy as the Nix home-manager module in
# nix/modules/scootbar-home.nix).
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here with the bar's own tag and tarball basename.

Name:           scootbar
Version:        0.1.0
Release:        1%{?dist}
Summary:        Status bar for the scoot compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/scootbar-v%{version}/scootbar-%{version}-vendored.tar.gz
BuildRequires:  cargo
BuildRequires:  gcc
%if 0%{?suse_version}
BuildRequires:  systemd-rpm-macros
%else
BuildRequires:  systemd-rpm-macros
%endif

%description
scootbar is the status bar for the scoot compositor, and runs standalone
under any compositor speaking standard Wayland protocols. Copy
/usr/share/doc/scootbar/bar.toml.example to ~/.config/scoot/bar.toml and
check it with `scootbar daemon --check` (prints `ok`).

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
%doc packaging/rpm/bar.toml.example

%changelog
* Thu Oct 08 2026 scoot-sh <packages@scoot.sh> - 0.1.0-1
- First RPM: status bar plus user unit, from the vendored tarball.
