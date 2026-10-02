//! A refused reload keeps the running bar: [`Responder::stage`] fails the
//! font before anything is started, moved or killed.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::Responder;
use crate::config::Config;
use crate::layout::Layout;
use crate::modules::custom::{Custom, Kind, intern};
use crate::modules::exec::{Restart, Settings};
use crate::modules::harness::drive_placed;
use crate::modules::payload::Format;
use crate::modules::{OutputView, Settings as Bar, View, start};

/// A scratch directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("scootbar-refuse-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(placed: &crate::modules::Placed) -> String {
    let mut view = View::default();
    placed.module.view(&OutputView { name: None }, &mut view);
    view.text().to_owned()
}

/// The pid a test command wrote to `file` (`echo $$ > file`).
fn pid_in(file: &Path) -> u32 {
    fs::read_to_string(file)
        .expect("the command wrote its pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// Whether process `pid` is gone from the process table: a zombie is not.
fn gone(pid: u32) -> bool {
    !Path::new(&format!("/proc/{pid}")).exists()
}

#[test]
fn a_font_that_vanished_refuses_before_anything_moves() {
    let dir = Dir::new("font");
    let pidfile = dir.file("pid");
    let id = intern("refused").unwrap();
    let script = format!("echo $$ > {}; echo started; sleep 336", pidfile.display());
    let exec = Settings {
        command: vec!["sh".into(), "-c".into(), script],
        format: Format::Text,
        placeholder: "ph".into(),
        restart: Restart {
            first: Duration::from_millis(20),
            max: Duration::from_millis(80),
            stable: Duration::from_secs(10),
        },
    };
    let modules = Bar {
        custom: vec![Custom {
            id,
            kind: Kind::Exec(exec),
        }],
        ..Default::default()
    };
    let lists = Layout {
        left: vec![id],
        center: vec![],
        right: vec![],
        ..Layout::default()
    };
    // The running bar: an `exec` with its child up and its line shown.
    let mut said = Vec::new();
    let mut old = start(&lists, &modules, &mut Vec::new(), &mut |id, why| {
        said.push(format!("{id}: {why}"))
    });
    assert!(said.is_empty(), "unexpected unavailable: {said:?}");
    drive_placed(&mut old[0], "started", |p| text(p) == "started");
    let pid = pid_in(&pidfile);
    let revision = old[0].revision;
    // The same table, with a font that does not exist.
    let config = Config {
        font: Some(PathBuf::from("/nonexistent/scootbar-test-font.ttf")),
        modules: modules.clone(),
        layout: lists.clone(),
        ..Default::default()
    };
    let result = Responder::stage(&config, &mut old, &mut |_, _| {
        panic!("nothing starts on a refused reload")
    });
    assert!(result.is_err(), "a missing font refuses the reload");
    assert_eq!(old.len(), 1, "the running bar stands as it was");
    assert_eq!(text(&old[0]), "started");
    assert_eq!(old[0].revision, revision);
    assert_eq!(pid_in(&pidfile), pid, "no new child was started");
    assert!(!gone(pid), "the child is still running");
    drop(old);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gone(pid) {
        assert!(
            Instant::now() < deadline,
            "pid {pid} is still in the process table"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
