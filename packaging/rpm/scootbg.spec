# scootbg: the wallpaper daemon for the scoot compositor.
#
# Built from the same trio tag as `scoot` (`scoot-v%{version}`): the two move
# in lockstep (docs/versioning.md). The daemon links nothing past the C
# library, so it carries no system-library dependencies. Its `[wallpaper]`
# section is documented in scoot's example config
# (/usr/share/doc/scoot/config.toml.example).
#
# No release is cut yet: see the Source0 note in scoot.spec, which applies
# here unchanged (same trio tarball, same provisional basename).

Name:           scootbg
Version:        0.1.0
Release:        1%{?dist}
Summary:        Wallpaper daemon for the scoot compositor
License:        MIT
URL:            https://github.com/scoot-sh/scoot
Source0:        https://github.com/scoot-sh/scoot/releases/download/scoot-v%{version}/scoot-%{version}-vendored.tar.gz
BuildRequires:  cargo
BuildRequires:  gcc

%description
scootbg is the wallpaper daemon for the scoot compositor. The compositor
drives it through `scootbg apply-config` with the `[wallpaper]` section of
its own config; run `scootbg --help` for the standalone commands.

%prep
# Same vendored-tarball layout as scoot.spec: ./source/, not ./Name-Version.
%setup -q -c -T
tar -xzf %{SOURCE0}

%build
# Offline: every dependency is in vendor/ through .cargo/config.toml.
export CARGO_NET_OFFLINE=true
cd source
cargo build --release --locked --offline -p scootbg

%install
cd source
install -Dm755 target/release/scootbg %{buildroot}%{_bindir}/scootbg

%check
# Headless-safe: the binary starts and reports its version.
cd source
./target/release/scootbg --version

%files
%{_bindir}/scootbg
%license source/LICENSE source/NOTICE source/THIRD-PARTY-LICENSES

%changelog
* Wed Oct 08 2026 scoot-sh <packages@scoot.sh> - 0.1.0-1
- First RPM: wallpaper daemon, from the vendored tarball.
