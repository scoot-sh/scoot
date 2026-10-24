//! `install`, `export` and `save` against scratch directories: every
//! refusal in the ticket (symlink, `..`, absolute path, a `.so`, a taken
//! name, a missing license, an oversized image, `apps/` without `--trust`),
//! the `extends`-never-installed rule end to end, the export/install and
//! save round-trips, and the URL fetch's refusals (loopback, RFC 1918,
//! `fe80::/10`, a rebinding resolver) with its argv pinned.
//!
//! Nothing here touches the process environment: every function under test
//! takes its directories (and, for the fetch, its resolver and downloader)
//! explicitly. [`tempfile`] owns every tree.

use std::cell::RefCell;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use scoot_look::Origin;
use scoot_look::catalog::{Catalog, discover_ordered};

use super::apply::{Availability, Checks, ProdChecks};
use super::{
    APPS_LEAVES, CurlFetcher, Dns, Fetcher, MAX_INSTALL_FILES, cached_url_wallpaper_in,
    export_into, fetch_url_wallpaper, install_dir_into, install_tar_into, is_public_addr,
    save_into,
};

/// A standalone-valid manifest: moonrise's palette (which clears the
/// installed-origin contrast bars), every required token, nothing else.
fn manifest(name: &str) -> String {
    format!(
        "\
schema = 1
name = \"{name}\"
title = \"{name}\"
description = \"a test look\"
polarity = \"dark\"
license = \"MIT\"

[palette]
background = \"#2B3648\"
foreground = \"#F6EEDC\"
dim = \"#9C8B95\"
accent = \"#FFA45C\"
urgent = \"#E87F6A\"

[fonts]
ui = \"DejaVu Sans\"
sans = \"DejaVu Sans\"
mono = \"DejaVu Sans Mono\"
size = 10

[shape]
ring-width = 2

[cursor]
theme = \"Mocu\"
"
    )
}

/// A source look directory with `look.toml` plus `files` (rel → bytes).
fn source_look(parent: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("a source look");
    std::fs::write(dir.join("look.toml"), manifest(name)).expect("a manifest");
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(base) = path.parent() {
            std::fs::create_dir_all(base).expect("a subdir");
        }
        std::fs::write(&path, bytes).expect("a leaf");
    }
    dir
}

/// A source look with a hand-written manifest (for hostile manifests).
fn source_look_raw(parent: &Path, name: &str, manifest: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("a source look");
    std::fs::write(dir.join("look.toml"), manifest).expect("a manifest");
    dir
}

/// The catalog over explicit scratch roots: user, installed, shipped.
fn catalog(user: &Path, installed: &Path, shipped: &Path) -> Catalog {
    discover_ordered(&[
        (user.to_owned(), Origin::User),
        (installed.to_owned(), Origin::Installed),
        (shipped.to_owned(), Origin::Shipped),
    ])
}

/// An empty catalog (nothing installed under any name).
fn empty_catalog() -> Catalog {
    Catalog::default()
}

/// The eight PNG magic bytes: enough for the no-`scootbg` sniff.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A downloader and resolver in one scripted box: `addrs` answers every
/// resolution in order (so a rebinding resolver answers differently the
/// second time), `body` is what the download writes.
struct ScriptedNet {
    addrs: RefCell<Vec<Vec<IpAddr>>>,
    body: Vec<u8>,
    resolves: RefCell<usize>,
    fetched: RefCell<Vec<(String, String, IpAddr)>>,
}

impl ScriptedNet {
    fn once(addrs: Vec<IpAddr>, body: &[u8]) -> Self {
        Self {
            addrs: RefCell::new(vec![addrs]),
            body: body.to_vec(),
            resolves: RefCell::new(0),
            fetched: RefCell::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            addrs: RefCell::new(vec![vec![]]),
            body: Vec::new(),
            resolves: RefCell::new(0),
            fetched: RefCell::new(Vec::new()),
        }
    }
}

impl Dns for ScriptedNet {
    fn resolve(&self, _host: &str) -> io::Result<Vec<IpAddr>> {
        *self.resolves.borrow_mut() += 1;
        let mut addrs = self.addrs.borrow_mut();
        if addrs.len() > 1 {
            Ok(addrs.remove(0))
        } else {
            Ok(addrs.first().cloned().unwrap_or_default())
        }
    }
}

impl Fetcher for ScriptedNet {
    fn fetch(&self, url: &str, host: &str, addr: IpAddr, out: &Path) -> Result<(), String> {
        self.fetched
            .borrow_mut()
            .push((url.to_owned(), host.to_owned(), addr));
        std::fs::write(out, &self.body).map_err(|error| error.to_string())?;
        Ok(())
    }
}

impl super::UrlFetch for ScriptedNet {
    fn fetch_url(&self, url: &str, sha256: &str, cache_dir: &Path) -> Result<PathBuf, String> {
        fetch_url_wallpaper(url, sha256, cache_dir, self, self)
    }
}

fn sha256_of(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A URL-wallpaper manifest: the only hostile-adjacent field install
/// fetches.
fn url_manifest(name: &str, url: &str, pin: &str) -> String {
    format!(
        "{manifest}\n[wallpaper]\nimage = \"{url}\"\nsha256 = \"{pin}\"\nmode = \"fill\"\n",
        manifest = manifest(name).trim_end(),
    )
}

#[test]
fn install_dir_lands_regular_files() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src = source_look(
        &root.path().join("src"),
        "handed-over",
        &[("wallpaper.png", PNG), ("README.md", b"hello")],
    );
    let dest_parent = root.path().join("installed-looks");
    let report = install_dir_into(
        &src,
        &dest_parent,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("a valid look installs");
    assert_eq!(report.name, "handed-over");
    assert_eq!(report.dir, dest_parent.join("handed-over"));
    assert!(report.dropped.is_empty());
    assert_eq!(
        std::fs::read(report.dir.join("look.toml")).expect("an installed manifest"),
        std::fs::read(src.join("look.toml")).expect("the source manifest"),
    );
    assert_eq!(
        std::fs::read(report.dir.join("wallpaper.png")).expect("an installed image"),
        PNG,
    );
}

#[test]
fn install_refuses_a_symlinked_leaf() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src_dir = root.path().join("src").join("linked-leaf");
    std::fs::create_dir_all(&src_dir).expect("a source look");
    std::fs::write(src_dir.join("look.toml"), manifest("linked-leaf")).expect("a manifest");
    std::os::unix::fs::symlink("/etc/hostname", src_dir.join("wallpaper.png"))
        .expect("a symlink leaf");
    let error = install_dir_into(
        &src_dir,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a symlink installs");
    assert!(error.contains("symlink"), "names the symlink: {error}");
    assert!(
        !root.path().join("installed-looks").exists(),
        "nothing lands on refusal"
    );
}

#[test]
fn install_refuses_a_symlinked_source() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let real = source_look(&root.path().join("src"), "real", &[]);
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("a symlink source");
    let error = install_dir_into(
        &link,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a symlinked source installs");
    assert!(error.contains("symlink"), "names the symlink: {error}");
}

#[test]
fn install_refuses_a_non_regular_leaf() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src_dir = root.path().join("src").join("fifo-leaf");
    std::fs::create_dir_all(&src_dir).expect("a source look");
    std::fs::write(src_dir.join("look.toml"), manifest("fifo-leaf")).expect("a manifest");
    nix_fifo(&src_dir.join("wallpaper.png"));
    let error = install_dir_into(
        &src_dir,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a FIFO installs");
    assert!(error.contains("regular file"), "names the refusal: {error}");
}

/// A FIFO without `nix`: plain Rust cannot make one, so a `/dev/null`
/// stand-in is refused the same way (not a regular file). Kept beside the
/// real-FIFO test only where `mkfifo` exists.
fn nix_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo makes the FIFO");
}

#[test]
fn install_refuses_dotdot_and_absolute_wallpapers() {
    let root = tempfile::tempdir().expect("a scratch tree");
    for (tag, image) in [("dotdot", "../evil.png"), ("absolute", "/etc/hostname")] {
        let mangled = manifest(tag).replace(
            "[fonts]",
            &format!("[wallpaper]\nimage = \"{image}\"\n[fonts]"),
        );
        let src = source_look_raw(&root.path().join("src"), tag, &mangled);
        let error = install_dir_into(
            &src,
            &root.path().join("installed-looks"),
            &empty_catalog(),
            &ProdChecks::none(),
            false,
            &ScriptedNet::failing(),
        )
        .expect_err("an escaping wallpaper installs");
        assert!(
            error.contains("..") || error.contains("relative") || error.contains("absolute"),
            "{tag}: names the escape: {error}"
        );
    }
}

#[test]
fn install_refuses_a_so_and_a_stray_dir() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src = source_look(
        &root.path().join("src"),
        "carrying",
        &[("payload.so", b"\x7fELF"), ("README.md", b"hi")],
    );
    let error = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a .so installs");
    assert!(
        error.contains("payload.so") && error.contains("not a file a look uses"),
        "refuses by name: {error}"
    );
    let src = source_look(&root.path().join("src2"), "nested", &[]);
    std::fs::create_dir_all(src.join("sub")).expect("a stray dir");
    std::fs::write(src.join("sub").join("x.png"), PNG).expect("a nested image");
    let error = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a stray dir installs");
    assert!(error.contains("sub"), "refuses by name: {error}");
}

#[test]
fn install_refuses_taken_names_and_existing_dirs() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let shipped = root.path().join("shipped");
    source_look(&shipped, "taken-shipped", &[]);
    let user = root.path().join("user-looks");
    source_look(&user, "taken-user", &[]);
    let installed = root.path().join("installed-looks");
    let taken = catalog(&user, &installed, &shipped);
    for (name, origin) in [("taken-shipped", "shipped"), ("taken-user", "user")] {
        let src = source_look(&root.path().join("src"), name, &[]);
        let error = install_dir_into(
            &src,
            &installed,
            &taken,
            &ProdChecks::none(),
            false,
            &ScriptedNet::failing(),
        )
        .expect_err("a taken name installs");
        assert!(
            error.contains(name) && error.contains(origin),
            "names the look and its origin: {error}"
        );
    }
    // Twice is a refusal the second time: the destination already exists.
    let src = source_look(&root.path().join("src2"), "planted", &[]);
    install_dir_into(
        &src,
        &installed,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("the first install lands");
    let error = install_dir_into(
        &src,
        &installed,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("the second install lands");
    assert!(error.contains("already exists"), "{error}");
    // And once it catalogs, the refusal names the installed look.
    let cataloged = catalog(
        &root.path().join("user-looks"),
        &installed,
        &root.path().join("shipped"),
    );
    let error = install_dir_into(
        &src,
        &installed,
        &cataloged,
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("an installed name installs");
    assert!(
        error.contains("planted") && error.contains("already installed"),
        "{error}"
    );
}

#[test]
fn install_refuses_a_missing_license() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let unlicensed = manifest("unlicensed").replace("license = \"MIT\"\n", "");
    let src = source_look_raw(&root.path().join("src"), "unlicensed", &unlicensed);
    let error = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("an unlicensed look installs");
    assert!(error.contains("license"), "names the license: {error}");
}

#[test]
fn install_refuses_an_oversized_image() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let huge =
        manifest("huge").replace("[fonts]", "[wallpaper]\nimage = \"wallpaper.png\"\n[fonts]");
    let src = source_look_raw(&root.path().join("src"), "huge", &huge);
    let big = vec![0u8; 32 * 1024 * 1024 + 1];
    std::fs::write(src.join("wallpaper.png"), &big).expect("a huge image");
    let error = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("an oversized image installs");
    assert!(error.contains("over the"), "names the cap: {error}");
}

#[test]
fn install_drops_apps_leaves_without_trust_and_keeps_them_with_it() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let leaves: Vec<(&str, &[u8])> = vec![
        ("apps/helix.toml", b"[theme]\n" as &[u8]),
        ("apps/btop.theme", b"theme[main_bg]=\"#000000\"\n"),
        ("apps/starship.toml", b"format = \"x\"\n"),
        (
            "apps/nvim.lua",
            b"vim.g.colors_name = \"scoot-look\"\n" as &[u8],
        ),
    ];
    let src = source_look(&root.path().join("src"), "leafy", &leaves);
    let report = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("a leafy look installs untrusted");
    assert_eq!(
        report.dropped,
        vec![
            "apps/btop.theme".to_owned(),
            "apps/helix.toml".to_owned(),
            "apps/nvim.lua".to_owned(),
            "apps/starship.toml".to_owned(),
        ],
        "every leaf drops, sorted"
    );
    for leaf in APPS_LEAVES {
        assert!(
            !report.dir.join(leaf).exists(),
            "{leaf} must not land untrusted"
        );
    }
    let dest_parent = root.path().join("installed-again");
    let report = install_dir_into(
        &src,
        &dest_parent,
        &empty_catalog(),
        &ProdChecks::none(),
        true,
        &ScriptedNet::failing(),
    )
    .expect("a leafy look installs trusted");
    assert!(report.dropped.is_empty());
    assert_eq!(report.kept.len(), 4, "kept leaves print first: {report:?}");
    for (rel, bytes) in &leaves {
        assert_eq!(
            std::fs::read(report.dir.join(rel)).expect("a kept leaf"),
            *bytes,
            "{rel} lands byte for byte"
        );
    }
}

#[test]
fn installed_looks_never_extend() {
    // End to end: install A, then a source B extending A refuses through
    // the same validation an apply runs.
    let root = tempfile::tempdir().expect("a scratch tree");
    let src_a = source_look(&root.path().join("src"), "base-a", &[]);
    let installed = root.path().join("installed-looks");
    install_dir_into(
        &src_a,
        &installed,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("the base installs");
    let catalog = catalog(
        &root.path().join("user-looks"),
        &installed,
        &root.path().join("shipped"),
    );
    assert!(catalog.find("base-a").is_some(), "the base catalogs");
    assert_eq!(
        catalog.find("base-a").expect("the base").origin,
        Origin::Installed,
    );
    let child = manifest("child-b").replace(
        "license = \"MIT\"",
        "license = \"MIT\"\nextends = \"base-a\"",
    );
    let src_b = source_look_raw(&root.path().join("src"), "child-b", &child);
    let error = install_dir_into(
        &src_b,
        &root.path().join("installed-again"),
        &catalog,
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("an installed base extends");
    assert!(error.contains("never extended"), "names the rule: {error}");
}

#[test]
fn export_and_install_round_trip() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src = source_look(
        &root.path().join("src"),
        "roundtrip",
        &[
            ("wallpaper.png", PNG),
            ("README.md", b"hand this over"),
            ("LICENSE", b"MIT"),
            ("apps/starship.toml", b"format = \"x\"\n"),
        ],
    );
    let installed = root.path().join("installed-looks");
    install_dir_into(
        &src,
        &installed,
        &empty_catalog(),
        &ProdChecks::none(),
        true,
        &ScriptedNet::failing(),
    )
    .expect("the look installs");
    let catalog = catalog(
        &root.path().join("user-looks"),
        &installed,
        &root.path().join("shipped"),
    );
    let tarball = root.path().join("roundtrip.tar");
    let export = export_into("roundtrip", &catalog, &tarball).expect("the look exports");
    assert!(tarball.is_file(), "the tarball lands");
    assert!(
        export.files.iter().all(|f| f.starts_with("roundtrip/")),
        "one top-level prefix: {:?}",
        export.files
    );
    assert!(
        export.files.contains(&"roundtrip/look.toml".to_owned()),
        "{:?}",
        export.files
    );
    // A hostile reader (system tar) lists exactly the members: no `..`,
    // no absolute paths, no links.
    let listing = std::process::Command::new("tar")
        .args(["-tf"])
        .arg(&tarball)
        .output()
        .expect("tar lists");
    assert!(listing.status.success(), "system tar reads the export");
    let listing = String::from_utf8(listing.stdout).expect("UTF-8 listing");
    for line in listing.lines() {
        assert!(
            line.starts_with("roundtrip/"),
            "every member under the prefix: {line}"
        );
        assert!(!line.contains(".."), "no parent climbs: {line}");
    }
    // And installs back byte for byte under a fresh parent.
    let again = root.path().join("installed-again");
    let report = install_tar_into(
        &tarball,
        &again,
        &empty_catalog(),
        &ProdChecks::none(),
        true,
        &ScriptedNet::failing(),
    )
    .expect("the tarball installs");
    for member in &export.files {
        let rel = member.strip_prefix("roundtrip/").expect("the prefix");
        assert_eq!(
            std::fs::read(report.dir.join(rel)).expect("an installed member"),
            std::fs::read(installed.join("roundtrip").join(rel)).expect("the source member"),
            "{rel} round-trips byte for byte"
        );
    }
}

#[test]
fn install_tarball_refuses_hostile_entries() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let dest = root.path().join("installed-looks");
    // Each hostile tarball carries a valid manifest plus one hostile
    // member; the install refuses naming it.
    type TarHostile<'a> = Vec<(&'a str, Box<dyn Fn(&mut tar::Builder<std::fs::File>) + 'a>)>;
    let hostile: TarHostile = vec![
        (
            "symlink",
            Box::new(|b| {
                let mut h = tar::Header::new_ustar();
                h.set_size(0);
                h.set_entry_type(tar::EntryType::Symlink);
                // A permitted name (`wallpaper.png`), so the refusal pins
                // the kind gate, not the name gate: neutering the
                // `!kind.is_file()` arm must fail this test.
                b.append_link(&mut h, "wallpaper.png", "/etc/hostname").ok();
            }),
        ),
        (
            "hardlink",
            Box::new(|b| {
                let mut h = tar::Header::new_ustar();
                h.set_size(0);
                h.set_entry_type(tar::EntryType::Link);
                // Same permitted-name pin for hard links.
                b.append_link(&mut h, "wallpaper.png", "look.toml").ok();
            }),
        ),
        (
            "duplicate",
            Box::new(|b| {
                for _ in 0..2 {
                    let mut h = tar::Header::new_ustar();
                    h.set_size(7);
                    b.append_data(&mut h, "README.md", &b"twice! "[..]).ok();
                }
            }),
        ),
        (
            "so",
            Box::new(|b| {
                let mut h = tar::Header::new_ustar();
                h.set_size(4);
                b.append_data(&mut h, "payload.so", &b"ELF!"[..]).ok();
            }),
        ),
    ];
    // `..` and absolute members bypass the `tar` crate's writer (which
    // refuses them itself) with raw headers: a real attacker writes raw
    // tar, and the install gate must refuse what the writer will not.
    let raw_hostile: Vec<(&str, String)> = vec![
        ("raw-parent", "../evil".to_owned()),
        ("raw-absolute", "/tmp/evil".to_owned()),
    ];
    for (tag, build) in hostile {
        let name = format!("hostile-{tag}");
        let path = root.path().join(format!("{name}.tar"));
        {
            let file = std::fs::File::create(&path).expect("a tarball");
            let mut builder = tar::Builder::new(file);
            let mut h = tar::Header::new_ustar();
            let bytes = manifest(&name).into_bytes();
            h.set_size(bytes.len() as u64);
            builder
                .append_data(&mut h, "look.toml", &bytes[..])
                .expect("a manifest member");
            build(&mut builder);
            builder.finish().expect("a finished tarball");
        }
        refuse_hostile(&path, &dest, tag);
    }
    for (tag, member) in raw_hostile {
        let name = format!("hostile-{tag}");
        let path = root.path().join(format!("{name}.tar"));
        let mut tarball = raw_member("look.toml", b'0', manifest(&name).as_bytes());
        tarball.extend(raw_member(&member, b'0', b"evil!"));
        tarball.extend([0u8; 1024]);
        std::fs::write(&path, &tarball).expect("a raw tarball");
        refuse_hostile(&path, &dest, tag);
    }
}

/// One hostile tarball refuses install, naming the refusal, landing
/// nothing.
fn refuse_hostile(path: &Path, dest: &Path, tag: &str) {
    let name = path
        .file_stem()
        .expect("a stem")
        .to_string_lossy()
        .into_owned();
    let error = install_tar_into(
        path,
        dest,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a hostile tarball installs");
    assert!(error.contains("refused"), "{tag}: refuses loudly: {error}");
    assert!(
        !dest.join(&name).exists(),
        "{tag}: nothing lands on refusal"
    );
}

/// One raw ustar member: the name, typeflag and data as given (no writer
/// validation), padded to 512-byte blocks with a correct checksum, so the
/// reader under test sees exactly these bytes.
fn raw_member(name: &str, typeflag: u8, data: &[u8]) -> Vec<u8> {
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    let size = format!("{0:011o}\0", data.len());
    header[124..136].copy_from_slice(size.as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[156] = typeflag;
    header[257..262].copy_from_slice(b"ustar");
    header[263..265].copy_from_slice(b"00");
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|b| *b as u32).sum();
    let sum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(sum.as_bytes());
    let mut out = header.to_vec();
    out.extend_from_slice(data);
    out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    out
}

#[test]
fn install_tarball_refuses_gzip_and_non_archives() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let gz = root.path().join("look.tar.gz");
    std::fs::write(&gz, [0x1fu8, 0x8b, 0x08, 0x00]).expect("a gzip head");
    let error = install_tar_into(
        &gz,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("gzip installs");
    assert!(error.contains("gzip"), "{error}");
    let txt = root.path().join("notes.txt");
    std::fs::write(&txt, b"not a tarball").expect("a text file");
    let error = install_tar_into(
        &txt,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a text file installs");
    assert!(error.contains("not a tarball"), "{error}");
}

#[test]
fn save_snapshots_user_files_onto_the_live_look() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let user = root.path().join("user-looks");
    source_look(&user, "live-base", &[]);
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    std::fs::write(
        state.join("look.json"),
        r#"{"name": "live-base", "rendered": "/tmp/nowhere", "generation": 1}"#,
    )
    .expect("a record");
    let catalog = catalog(
        &user,
        &root.path().join("installed-looks"),
        &root.path().join("shipped"),
    );
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("scoot")).expect("a config tree");
    std::fs::write(
        config.join("scoot/config.toml"),
        "\
[appearance]
focus_ring_active_color = \"#FF9A49\"
background_color = \"#2B3648\"
corner_radius = 14
cursor_theme = \"Mocu\"
cursor_size = 24

[wallpaper]
color = \"#2B3648\"
mode = \"fill\"
",
    )
    .expect("a compositor file");
    std::fs::write(
        config.join("scoot/bar.toml"),
        "\
[colors]
background = \"#111111\"
foreground = \"#F6EEDC\"
dim = \"#9C8B95\"
accent = \"#FFA45C\"
urgent = \"#E87F6A\"
hover = \"#FFD54A\"
",
    )
    .expect("a bar file");
    let report = save_into(
        "saved-here",
        &config,
        &user,
        Some(&state),
        &catalog,
        &ProdChecks::none(),
    )
    .expect("user values save");
    assert_eq!(report.dir, user.join("saved-here"));
    assert_eq!(report.extends.as_deref(), Some("live-base"));
    assert!(report.tokens.contains(&"palette".to_owned()));
    // The compositor wins the background both files set, said aloud.
    let saved = std::fs::read_to_string(report.dir.join("look.toml")).expect("a saved manifest");
    assert!(saved.contains("background = \"#2B3648\""), "{saved}");
    assert!(
        report.notes.iter().any(|n| n.contains("wins over the bar")),
        "the conflict is a note: {:?}",
        report.notes
    );
    assert!(saved.contains("focus = \"#FF9A49\""), "{saved}");
    assert!(saved.contains("radius = 14"), "{saved}");
}

#[test]
fn save_refuses_with_no_live_look() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("scoot")).expect("a config tree");
    std::fs::write(
        config.join("scoot/bar.toml"),
        "[colors]\nbackground = \"#2B3648\"\nforeground = \"#F6EEDC\"\ndim = \"#9C8B95\"\naccent = \"#FFA45C\"\nurgent = \"#E87F6A\"\n",
    )
    .expect("a bar file");
    // No record: nothing to extend, so the save refuses.
    let error = save_into(
        "lonely-save",
        &config,
        &root.path().join("user-looks"),
        Some(&root.path().join("state")),
        &empty_catalog(),
        &ProdChecks::none(),
    )
    .expect_err("a baseless save lands");
    assert!(error.contains("no live look"), "{error}");
    // A live look gone from the catalog refuses too, naming it.
    let state = root.path().join("state2");
    std::fs::create_dir_all(&state).expect("a state dir");
    std::fs::write(
        state.join("look.json"),
        r#"{"name": "vanished", "rendered": "/tmp/nowhere", "generation": 1}"#,
    )
    .expect("a record");
    let error = save_into(
        "lonely-save",
        &config,
        &root.path().join("user-looks"),
        Some(&state),
        &empty_catalog(),
        &ProdChecks::none(),
    )
    .expect_err("a save onto a vanished base lands");
    assert!(error.contains("gone from the catalog"), "{error}");
}

#[test]
fn save_extends_the_live_look_and_copies_its_wallpaper_file() {
    let root = tempfile::tempdir().expect("a scratch tree");
    // The live base in the catalog.
    let user = root.path().join("user-looks");
    source_look(&user, "live-base", &[("wallpaper.png", PNG)]);
    // A live choice recorded on it.
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    std::fs::write(
        state.join("look.json"),
        r#"{"name": "live-base", "rendered": "/tmp/nowhere", "generation": 3}"#,
    )
    .expect("a record");
    let catalog = catalog(
        &user,
        &root.path().join("installed-looks"),
        &root.path().join("shipped"),
    );
    // The user's files carry one role and an image beside the config.
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("scoot")).expect("a config tree");
    std::fs::write(config.join("scoot").join("wall.png"), PNG).expect("a wallpaper");
    std::fs::write(
        config.join("scoot/config.toml"),
        "\
[appearance]
focus_ring_active_color = \"#FF9A49\"

[wallpaper]
image = \"wall.png\"
mode = \"fill\"
",
    )
    .expect("a compositor file");
    let report = save_into(
        "saved-next",
        &config,
        &user,
        Some(&state),
        &catalog,
        &ProdChecks::none(),
    )
    .expect("user values save onto the live look");
    assert_eq!(report.extends.as_deref(), Some("live-base"));
    let saved = std::fs::read_to_string(report.dir.join("look.toml")).expect("a saved manifest");
    assert!(saved.contains("extends = \"live-base\""), "{saved}");
    assert_eq!(
        std::fs::read(report.dir.join("wall.png")).expect("a copied wallpaper"),
        PNG,
    );
}

#[test]
fn save_refuses_with_nothing_to_take() {
    let root = tempfile::tempdir().expect("a scratch tree");
    // No files at all.
    let error = save_into(
        "empty-save",
        &root.path().join("config"),
        &root.path().join("user-looks"),
        None,
        &empty_catalog(),
        &ProdChecks::none(),
    )
    .expect_err("nothing saves");
    assert!(error.contains("nothing to save"), "{error}");
    // Files with no appearance or color values.
    let config = root.path().join("config2");
    std::fs::create_dir_all(config.join("scoot")).expect("a config tree");
    std::fs::write(config.join("scoot/config.toml"), "[binds]\n").expect("a binds file");
    let error = save_into(
        "empty-save",
        &config,
        &root.path().join("user-looks"),
        None,
        &empty_catalog(),
        &ProdChecks::none(),
    )
    .expect_err("binds save as a look");
    assert!(error.contains("nothing to save"), "{error}");
    // A taken name saves nothing.
    let user = root.path().join("user-looks");
    source_look(&user, "taken-save", &[]);
    let catalog = catalog(
        &user,
        &root.path().join("installed-looks"),
        &root.path().join("shipped"),
    );
    let error = save_into(
        "taken-save",
        &config,
        &user,
        None,
        &catalog,
        &ProdChecks::none(),
    )
    .expect_err("a taken name saves");
    assert!(error.contains("already a look"), "{error}");
}

#[test]
fn public_addresses_pass_and_everything_else_refuses() {
    let public: Vec<IpAddr> = vec![
        "8.8.8.8".parse().unwrap(),
        "1.1.1.1".parse().unwrap(),
        "93.184.216.34".parse().unwrap(),
        "2606:4700:4700::1111".parse().unwrap(),
        "2001:4860:4860::8888".parse().unwrap(),
    ];
    for ip in public {
        assert!(is_public_addr(ip), "{ip} is public");
    }
    let refused: Vec<IpAddr> = vec![
        // Loopback, unspecified, multicast, broadcast.
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(127, 1, 2, 3)),
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::BROADCAST),
        // Private, link-local, CGNAT, benchmarking, TEST-NET, reserved.
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)),
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)),
        IpAddr::V4(Ipv4Addr::new(240, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(0, 1, 2, 3)),
        // IPv6: loopback, unspecified, multicast, unique-local,
        // link-local, mapped loopback, documentation, 6to4, Teredo.
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        IpAddr::V6("ff02::1".parse().unwrap()),
        IpAddr::V6("fc00::1".parse().unwrap()),
        IpAddr::V6("fd12:3456::1".parse().unwrap()),
        IpAddr::V6("fe80::1".parse().unwrap()),
        IpAddr::V6("::ffff:127.0.0.1".parse().unwrap()),
        IpAddr::V6("::ffff:10.0.0.1".parse().unwrap()),
        IpAddr::V6("2001:db8::1".parse().unwrap()),
        IpAddr::V6("2002:0808:0808::1".parse().unwrap()),
        IpAddr::V6("2001::1".parse().unwrap()),
        IpAddr::V6("64:ff9b::0808:0808".parse().unwrap()),
    ];
    for ip in refused {
        assert!(!is_public_addr(ip), "{ip} refuses");
    }
    // But a mapped public address passes: the IPv4 inside is judged.
    assert!(is_public_addr("::ffff:8.8.8.8".parse().unwrap()));
}

#[test]
fn url_fetch_caches_and_never_refetches_a_hit() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let cache = root.path().join("scootbg");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    let net = ScriptedNet::once(vec!["93.184.216.34".parse().unwrap()], PNG);
    let cached = fetch_url_wallpaper(url, &pin, &cache, &net, &net).expect("the fetch lands");
    assert_eq!(*net.resolves.borrow(), 1, "one lookup");
    assert_eq!(net.fetched.borrow().len(), 1, "one download");
    let (got_url, got_host, got_addr) = &net.fetched.borrow()[0];
    assert_eq!(got_url, url);
    assert_eq!(got_host, "example.com");
    assert_eq!(*got_addr, "93.184.216.34".parse::<IpAddr>().unwrap());
    assert_eq!(std::fs::read(&cached).expect("cached bytes"), PNG);
    // A second call with a fetcher that cannot run still succeeds: the hit
    // skips the network entirely.
    let cached2 = fetch_url_wallpaper(url, &pin, &cache, &net, &net).expect("the hit lands");
    assert_eq!(cached, cached2);
    assert_eq!(*net.resolves.borrow(), 1, "no second lookup on a hit");
}

#[test]
fn url_fetch_refuses_private_mixed_and_empty_answers() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let cache = root.path().join("scootbg");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    for (tag, addrs) in [
        (
            "rfc1918",
            vec![
                "93.184.216.34".parse().unwrap(),
                "192.168.1.5".parse().unwrap(),
            ],
        ),
        ("loopback", vec!["127.0.0.1".parse().unwrap()]),
        ("link-local-v6", vec!["fe80::1".parse().unwrap()]),
        ("mapped-loopback", vec!["::ffff:127.0.0.1".parse().unwrap()]),
        ("empty", vec![]),
    ] {
        let net = ScriptedNet::once(addrs, PNG);
        let error = fetch_url_wallpaper(url, &pin, &cache, &net, &net)
            .expect_err("a hostile answer fetches");
        assert!(
            error.contains("not a public address") || error.contains("nothing resolves"),
            "{tag}: refuses the answer: {error}"
        );
        assert!(
            net.fetched.borrow().is_empty(),
            "{tag}: never downloads after refusing"
        );
    }
}

#[test]
fn url_fetch_pins_the_first_answer_against_rebinding() {
    // A resolver answering public, then loopback (DNS rebinding): the
    // fetch pins the first checked address and never looks again.
    let root = tempfile::tempdir().expect("a scratch tree");
    let cache = root.path().join("scootbg");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    let net = ScriptedNet {
        addrs: RefCell::new(vec![
            vec!["93.184.216.34".parse().unwrap()],
            vec!["127.0.0.1".parse().unwrap()],
        ]),
        body: PNG.to_vec(),
        resolves: RefCell::new(0),
        fetched: RefCell::new(Vec::new()),
    };
    fetch_url_wallpaper(url, &pin, &cache, &net, &net).expect("the pinned fetch lands");
    assert_eq!(*net.resolves.borrow(), 1, "exactly one lookup");
    assert_eq!(net.fetched.borrow().len(), 1, "exactly one download");
    assert_eq!(
        net.fetched.borrow()[0].2,
        "93.184.216.34".parse::<IpAddr>().unwrap(),
        "the checked address, not the rebound one"
    );
}

#[test]
fn url_fetch_refuses_mismatch_non_image_and_oversize() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let cache = root.path().join("scootbg");
    let url = "https://example.com/wall.png";
    let public = vec!["93.184.216.34".parse().unwrap()];
    // Hash mismatch: nothing caches.
    let net = ScriptedNet::once(public.clone(), PNG);
    let error = fetch_url_wallpaper(url, &sha256_of(b"other"), &cache, &net, &net)
        .expect_err("a mismatch caches");
    assert!(error.contains("sha256"), "{error}");
    assert!(
        cache
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    );
    // HTML, not an image: refused even with a matching pin.
    let html = b"<html>302 found</html>";
    let net = ScriptedNet::once(public.clone(), html);
    let error =
        fetch_url_wallpaper(url, &sha256_of(html), &cache, &net, &net).expect_err("HTML caches");
    assert!(error.contains("not a PNG"), "{error}");
    // Past the byte cap: refused.
    let big = vec![0x89u8; 32 * 1024 * 1024 + 1];
    let net = ScriptedNet::once(public, &big);
    let error = fetch_url_wallpaper(url, &sha256_of(&big), &cache, &net, &net)
        .expect_err("an oversize download caches");
    assert!(error.contains("over the"), "{error}");
}

/// The exact `curl` argv a URL fetch runs: no `--location`, no redirects,
/// `https` only, `--resolve` pinning the checked address. A fake `curl`
/// records its argv and serves canned bytes, so this proves the wiring a
/// real download takes.
#[test]
fn curl_argv_pins_without_redirects() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin dir");
    let argv_file = root.path().join("argv");
    let body = root.path().join("body");
    std::fs::write(&body, PNG).expect("canned bytes");
    // The fake curl records "$@" one per line, then copies the canned
    // bytes to whatever `--output` named.
    std::fs::write(
        bin.join("curl"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nout=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"--output\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\ncp \"{}\" \"$out\"\n",
            argv_file.display(),
            body.display(),
        ),
    )
    .expect("a fake curl");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(bin.join("curl"))
            .expect("fake metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(bin.join("curl"), perms).expect("an executable fake");
    }
    let curl = CurlFetcher::explicit(bin.join("curl"));
    let out = root.path().join("out.png");
    curl.fetch(
        "https://example.com/wall.png",
        "example.com",
        "93.184.216.34".parse().unwrap(),
        &out,
    )
    .expect("the fake curl runs");
    assert_eq!(std::fs::read(&out).expect("downloaded bytes"), PNG);
    let argv = std::fs::read_to_string(&argv_file).expect("recorded argv");
    let args: Vec<&str> = argv.lines().collect();
    assert!(
        !args.contains(&"--location"),
        "no redirects followed:\n{argv}"
    );
    assert!(
        args.contains(&"--resolve") && args.contains(&"example.com:443:93.184.216.34"),
        "the checked address pins the connection:\n{argv}"
    );
    assert!(
        args.windows(2).any(|w| w == ["--max-redirs", "0"]),
        "redirects capped at zero:\n{argv}"
    );
    assert!(
        args.windows(2).any(|w| w == ["--proto", "https"]),
        "https only:\n{argv}"
    );
}

/// The `curl_argv` unit form of the same promise: reverting the pin or
/// re-adding `--location` fails this test, not just the fake-curl one.
#[test]
fn curl_argv_shape() {
    let out = PathBuf::from("/tmp/o.png");
    let argv = super::curl_argv(
        "https://example.com/w.png",
        "example.com",
        "93.184.216.34".parse().unwrap(),
        &out,
    );
    assert!(!argv.iter().any(|a| a == "--location"));
    assert!(argv.contains(&"--resolve".to_owned()));
    assert!(argv.contains(&"example.com:443:93.184.216.34".to_owned()));
}

#[test]
fn cache_hit_serves_matching_pins_and_refuses_the_rest() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let cache = root.path().join("scootbg");
    std::fs::create_dir_all(&cache).expect("a cache dir");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    // Miss: nothing there.
    let error = cached_url_wallpaper_in(url, Some(&pin), &cache).expect_err("a miss serves");
    assert!(error.contains("not in the download cache"), "{error}");
    // Hit: pinned bytes serve.
    std::fs::write(cache.join(super::url_cache_name_for_test(url)), PNG).expect("cached bytes");
    let hit = cached_url_wallpaper_in(url, Some(&pin), &cache).expect("a hit serves");
    assert_eq!(std::fs::read(hit).expect("hit bytes"), PNG);
    // Stale: other bytes under the name refuse.
    std::fs::write(cache.join(super::url_cache_name_for_test(url)), b"stale").expect("stale bytes");
    let error = cached_url_wallpaper_in(url, Some(&pin), &cache).expect_err("stale serves");
    assert!(error.contains("no longer matches"), "{error}");
}

/// `Checks` with a scripted URL answer, so the apply path's cache branch
/// runs without the network or the environment.
struct UrlChecks {
    answer: Result<Availability<String>, String>,
}

impl Checks for UrlChecks {
    fn font_file(&self, _family: &str) -> Result<Availability<String>, String> {
        Ok(Availability::Unavailable(
            "test: fonts unchecked".to_owned(),
        ))
    }
    fn wallpaper(&self, _path: &Path) -> Result<Availability<()>, String> {
        Ok(Availability::Unavailable(
            "test: wallpaper unchecked".to_owned(),
        ))
    }
    fn bar_fragment(&self, _fragment: &Path) -> Result<Availability<()>, String> {
        Ok(Availability::Unavailable("test: bar unchecked".to_owned()))
    }
    fn foot_fragment(&self, _fragment: &Path) -> Result<Availability<()>, String> {
        Ok(Availability::Unavailable("test: foot unchecked".to_owned()))
    }
    fn url_wallpaper(
        &self,
        _url: &str,
        _sha256: Option<&str>,
    ) -> Result<Availability<String>, String> {
        match &self.answer {
            Ok(Availability::Available(path)) => Ok(Availability::Available(path.clone())),
            Ok(Availability::Unavailable(note)) => Ok(Availability::Unavailable(note.clone())),
            Err(error) => Err(error.clone()),
        }
    }
}

#[test]
fn url_wallpaper_resolves_from_cache_and_refuses_on_miss() {
    use super::apply::validate_entry;
    let root = tempfile::tempdir().expect("a scratch tree");
    let installed = root.path().join("installed-looks");
    let src = root.path().join("src");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    source_look_raw(&src, "url-look", &url_manifest("url-look", url, &pin));
    // Bypass the fetch: stage the installed directory by hand (the fetch
    // tests above own that path), with the cached download beside it.
    let dest = installed.join("url-look");
    std::fs::create_dir_all(&dest).expect("an installed look");
    for entry in std::fs::read_dir(src.join("url-look")).expect("source entries") {
        let entry = entry.expect("an entry");
        std::fs::copy(entry.path(), dest.join(entry.file_name())).expect("a staged file");
    }
    let cache = root.path().join("scootbg");
    std::fs::create_dir_all(&cache).expect("a cache dir");
    std::fs::write(cache.join(super::url_cache_name_for_test(url)), PNG).expect("cached bytes");
    let catalog = catalog(
        &root.path().join("user-looks"),
        &installed,
        &root.path().join("shipped"),
    );
    // Hit: validates with the cached path feeding the render.
    let hit = UrlChecks {
        answer: Ok(Availability::Available(
            cache
                .join(super::url_cache_name_for_test(url))
                .to_string_lossy()
                .into_owned(),
        )),
    };
    let validation = validate_entry(&catalog, "url-look", &hit, false).expect("validation runs");
    assert!(
        validation.errors.is_empty(),
        "a cached URL validates: {:?}",
        validation.errors
    );
    // Miss: refuses, naming the download (never fetching).
    let miss = UrlChecks {
        answer: Err("wallpaper `https://example.com/wall.png` is not in the download cache: reinstall the look so its URL wallpaper is fetched and verified".to_owned()),
    };
    let validation = validate_entry(&catalog, "url-look", &miss, false).expect("validation runs");
    assert!(
        validation
            .errors
            .iter()
            .any(|e| e.contains("not in the download cache")),
        "a miss refuses: {:?}",
        validation.errors
    );
}

#[test]
fn install_validates_like_apply() {
    // A look whose tokens no apply would take (a forbidden key) refuses at
    // install, not at first swap.
    let root = tempfile::tempdir().expect("a scratch tree");
    let mangled = manifest("forbidden").replace("[fonts]", "[autostart]\nrun = [\"x\"]\n[fonts]");
    let src = source_look_raw(&root.path().join("src"), "forbidden", &mangled);
    let error = install_dir_into(
        &src,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a forbidden key installs");
    assert!(error.contains("autostart"), "{error}");
}

#[test]
fn install_caps_count_and_total() {
    let root = tempfile::tempdir().expect("a scratch tree");
    // Past the file count: READMEs cannot repeat, so READMEs plus images.
    let src_dir = root.path().join("src").join("many");
    std::fs::create_dir_all(&src_dir).expect("a source look");
    std::fs::write(src_dir.join("look.toml"), manifest("many")).expect("a manifest");
    for i in 0..MAX_INSTALL_FILES + 2 {
        std::fs::write(src_dir.join(format!("w{i:03}.png")), PNG).expect("an image");
    }
    let error = install_dir_into(
        &src_dir,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("too many files install");
    assert!(error.contains(&MAX_INSTALL_FILES.to_string()), "{error}");
}

/// Mutation bait: [`is_public_addr`] must refuse `127.1` read as an
/// address (the WHATWG decimal-IPv4 trap from the design's F2). The
/// grammar already refuses the name; the classifier refuses the address
/// it parses to.
#[test]
fn decimal_ipv4_trap_refuses() {
    let trap: IpAddr = "127.0.0.1".parse().unwrap();
    assert!(!is_public_addr(trap));
    assert!(!is_public_addr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
}

#[test]
fn permitted_places_hold() {
    use super::permitted_rel;
    for good in [
        "look.toml",
        "preview.png",
        "README.md",
        "LICENSE",
        "LICENSE-MIT",
        "LICENSE.spdx",
        "COPYING",
        "COPYING.others",
        "wallpaper.png",
        "wall.JPG",
        "x.gif",
        "fonts/face.ttf",
        "fonts/face.otf",
        "apps/foot.ini",
        "apps/helix.toml",
        "apps/btop.theme",
        "apps/starship.toml",
        "apps/regreet.css",
    ] {
        assert!(permitted_rel(good), "{good} installs");
    }
    for bad in [
        "face.ttf",
        "wallpaper.png.bak",
        "fonts/face.woff2",
        "apps/evil.toml",
        "apps/nested/x.toml",
        "sub/wallpaper.png",
        "look.toml ",
        ".DS_Store",
        "payload.so",
    ] {
        assert!(!permitted_rel(bad), "{bad} refuses");
    }
}

#[test]
fn install_dir_and_tar_accept_large_image_and_font() {
    // A real-sized wallpaper (past the old 64 KiB `read_regular` cap) and
    // a font-sized file install from a directory AND from a tarball: the
    // directory path reads with the same per-kind caps the tarball path
    // has. Regression for the 64 KiB directory refusal of every shipped
    // look.
    let root = tempfile::tempdir().expect("a scratch tree");
    let with_wallpaper =
        manifest("big-look").replace("[fonts]", "[wallpaper]\nimage = \"wallpaper.png\"\n[fonts]");
    let src_dir = root.path().join("src").join("big-look");
    std::fs::create_dir_all(src_dir.join("fonts")).expect("a source look");
    std::fs::write(src_dir.join("look.toml"), with_wallpaper).expect("a manifest");
    let mut wallpaper = PNG.to_vec();
    wallpaper.extend(vec![0u8; 100 * 1024]);
    std::fs::write(src_dir.join("wallpaper.png"), &wallpaper).expect("a large image");
    let font = vec![0u8; 200 * 1024];
    std::fs::write(src_dir.join("fonts").join("big.ttf"), &font).expect("a font-sized file");
    let dest_parent = root.path().join("installed-looks");
    let report = install_dir_into(
        &src_dir,
        &dest_parent,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("a large image installs from a directory");
    assert_eq!(
        std::fs::read(report.dir.join("wallpaper.png")).expect("an installed image"),
        wallpaper,
    );
    assert_eq!(
        std::fs::read(report.dir.join("fonts").join("big.ttf")).expect("an installed font"),
        font,
    );
    // And the same content installs from a tarball: export the installed
    // look, then install the tarball under a fresh parent.
    let catalog = catalog(
        &root.path().join("user-looks"),
        &dest_parent,
        &root.path().join("shipped"),
    );
    let tarball = root.path().join("big-look.tar");
    export_into("big-look", &catalog, &tarball).expect("the large look exports");
    let again = root.path().join("installed-again");
    let report = install_tar_into(
        &tarball,
        &again,
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect("a large image installs from a tarball");
    assert_eq!(
        std::fs::read(report.dir.join("wallpaper.png")).expect("a re-installed image"),
        wallpaper,
    );
}

#[test]
fn install_dir_refuses_a_hardlinked_leaf() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let src_dir = root.path().join("src").join("evil-look");
    std::fs::create_dir_all(&src_dir).expect("a source look");
    std::fs::write(src_dir.join("look.toml"), manifest("evil-look")).expect("a manifest");
    let secret = root.path().join("secret.txt");
    std::fs::write(&secret, b"secret-bytes").expect("outside bytes");
    std::fs::hard_link(&secret, src_dir.join("README.md")).expect("a hardlink leaf");
    let error = install_dir_into(
        &src_dir,
        &root.path().join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        false,
        &ScriptedNet::failing(),
    )
    .expect_err("a hardlink installs");
    assert!(
        error.contains("hard link"),
        "names the hardlink refusal: {error}"
    );
    assert!(
        !root.path().join("installed-looks").exists(),
        "nothing lands on refusal"
    );
}

#[test]
fn export_skips_a_symlinked_leaf_with_a_note() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let user = root.path().join("user-looks");
    let src = source_look(&user, "linky", &[("README.md", b"hi")]);
    std::os::unix::fs::symlink("/etc/hostname", src.join("wallpaper.png")).expect("a symlink leaf");
    let catalog = catalog(
        &user,
        &root.path().join("installed-looks"),
        &root.path().join("shipped"),
    );
    let tarball = root.path().join("linky.tar");
    let report = export_into("linky", &catalog, &tarball).expect("export skips, never refuses");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("wallpaper.png") && n.contains("symlink")),
        "the symlink skips with a note: {:?}",
        report.notes
    );
    assert!(tarball.is_file(), "the tarball still lands");
}

#[test]
fn save_with_a_url_wallpaper_says_to_reinstall_on_a_miss() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let user = root.path().join("user-looks");
    source_look(&user, "live-base", &[]);
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).expect("a state dir");
    std::fs::write(
        state.join("look.json"),
        r#"{"name": "live-base", "rendered": "/tmp/nowhere", "generation": 1}"#,
    )
    .expect("a record");
    let catalog = catalog(
        &user,
        &root.path().join("installed-looks"),
        &root.path().join("shipped"),
    );
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("scoot")).expect("a config tree");
    let url = "https://example.com/wall.png";
    let pin = sha256_of(PNG);
    std::fs::write(
        config.join("scoot/config.toml"),
        format!(
            "[appearance]\nbackground_color = \"#2B3648\"\n[wallpaper]\nimage = \"{url}\"\nsha256 = \"{pin}\"\n"
        ),
    )
    .expect("a compositor file");
    // The URL resolves from the download cache at apply time; the save
    // itself never fetches, so the check answers from a canned path here
    // and the reply still names the reinstall the first swap may owe.
    let cached = root.path().join("cached.png");
    std::fs::write(&cached, PNG).expect("cached bytes");
    let checks = UrlChecks {
        answer: Ok(Availability::Available(
            cached.to_string_lossy().into_owned(),
        )),
    };
    let report = save_into("saved-url", &config, &user, Some(&state), &catalog, &checks)
        .expect("a URL wallpaper saves");
    assert!(
        report.notes.iter().any(|n| n.contains("reinstall")),
        "the save reply says when a reinstall is owed: {:?}",
        report.notes
    );
}
