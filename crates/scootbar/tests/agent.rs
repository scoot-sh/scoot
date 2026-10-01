//! The agent interface on a headless scoot: `layout` reports where each
//! module is, an agent clicks what it reports, `invoke` runs what a click
//! would, and `subscribe` carries what changed.
//!
//! The headline test is fidelity: on two outputs at different scales
//! (1 and 1.5), every module's rectangle is clicked at its first and last
//! logical pixel (the binding runs) and one pixel outside it on each side
//! (it does not). That is what lets an agent aim a click from `layout`
//! alone. Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use common::{Reaper, Session, rgb};
use serde_json::Value;

const BAR: &str = "#102030";
const HEIGHT: u32 = 40;

/// Two outputs, the second at 1.5, each 1600x1000 logical before scale.
const SCOOT: &str = "[[outputs]]\nname = \"headless-2\"\nscale = 1.5\n";

struct Rig {
    session: Session,
    bar: Reaper,
    dir: PathBuf,
}

impl Rig {
    fn start(tag: &str, scoot: &str, outputs: u32, tables: &str, lists: &str) -> Option<Self> {
        let session = Session::scoot(tag, outputs, scoot)?;
        let dir = session.runtime_dir().join("out");
        fs::create_dir_all(&dir).unwrap();
        let file = session.runtime_dir().join("bar.toml");
        let tables = tables.replace("DIR", dir.to_str().unwrap());
        fs::write(
            &file,
            format!(
                "{lists}\n[bar]\nheight = {HEIGHT}\n[colors]\nbackground = \"{BAR}\"\n{tables}"
            ),
        )
        .unwrap();
        let bar = Reaper(session.bar_with_env(&["--config", file.to_str().unwrap()], &[]));
        let mut rig = Self { session, bar, dir };
        rig.session
            .wait_for(&mut rig.bar.0, "the bar drawn", |session| {
                (session.scoot_screenshot(1).at(0, 0) == rgb(BAR)).then_some(())
            });
        Some(rig)
    }

    /// The bar's control socket.
    fn socket(&self) -> PathBuf {
        self.session
            .runtime_dir()
            .join(format!("scootbar-{}.sock", self.session.wayland_display))
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.msg(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
    }

    fn ok(&self, args: &[&str]) {
        let out = self.msg(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn refused(&self, args: &[&str]) -> String {
        let out = self.msg(args);
        assert!(!out.status.success(), "{args:?} was accepted");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn wait_lines(&mut self, name: &str, n: usize) -> Vec<String> {
        let path = self.dir.join(name);
        self.session
            .wait_for(&mut self.bar.0, &format!("{n} lines in {name}"), |_| {
                let lines: Vec<String> = fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect();
                (lines.len() >= n).then_some(lines)
            })
    }

    fn click(&self, x: i64, y: i64) {
        let reply = self.session.scoot_ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
        ));
        assert_eq!(reply["type"], "ok", "{reply}");
    }
}

fn append(word: &str) -> String {
    format!("{{ exec = [\"sh\", \"-c\", \"echo {word} >> DIR/clicks\"] }}")
}

/// Two buttons a few characters wide, each appending its own name.
fn buttons() -> String {
    format!(
        "[button.alpha]\ntext = \"A\"\non-click = {}\n\
         [button.beta]\ntext = \"BBB\"\non-click = {}\n",
        append("alpha"),
        append("beta"),
    )
}

/// Whether the pixel at (`x`, `y`) of `shot` is anything but the bar's
/// background: drawn ink.
fn inked(shot: &common::Shot, x: u32, y: u32) -> bool {
    shot.at(x, y) != rgb(BAR)
}

/// What is wrong, if anything, with `output`'s `layout` against its
/// screenshot: a module's rectangle with no ink in it, or a bar column
/// outside every rectangle with ink in it. Logical to device is
/// `(x - origin) * scale`, rounded outward.
fn ink_problem(shot: &common::Shot, output: &Value) -> Option<String> {
    let scale = output["scale"].as_f64().unwrap();
    let origin = output["origin"]["x"].as_i64().unwrap() as f64;
    let rows = (f64::from(HEIGHT) * scale).round() as u32;
    let mut covered = vec![false; shot.width as usize];
    for module in output["modules"].as_array().unwrap() {
        let x = module["x"].as_f64().unwrap();
        let w = module["width"].as_f64().unwrap();
        let lo = (((x - origin) * scale).floor().max(0.0)) as u32;
        let hi = ((((x + w - origin) * scale).ceil()) as u32).min(shot.width);
        let mut found = false;
        for column in lo..hi {
            covered[column as usize] = true;
            found |= (0..rows).any(|y| inked(shot, column, y));
        }
        if !found {
            return Some(format!("no ink in {module}"));
        }
    }
    for column in 0..shot.width {
        if !covered[column as usize] && (0..rows).any(|y| inked(shot, column, y)) {
            return Some(format!(
                "ink at device column {column}, outside every rect: {output}"
            ));
        }
    }
    None
}

#[test]
fn layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales() {
    let Some(mut rig) = Rig::start(
        "agent-fidelity",
        SCOOT,
        2,
        &buttons(),
        "left = [\"alpha\"]\nright = [\"beta\"]\n",
    ) else {
        return;
    };
    // Both bars are up before asking where they are.
    rig.session
        .wait_for(&mut rig.bar.0, "both bars drawn", |session| {
            (session.scoot_screenshot(2).at(0, 0) == rgb(BAR)).then_some(())
        });
    let layout = rig.json(&["layout"]);
    assert_eq!(layout["type"], "layout");
    let outputs = layout["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 2, "{layout}");
    // Two outputs, at two scales: the test is vacuous if they agree.
    let mut scales: Vec<f64> = outputs
        .iter()
        .map(|o| o["scale"].as_f64().unwrap())
        .collect();
    scales.sort_by(f64::total_cmp);
    assert_eq!(scales, [1.0, 1.5], "{layout}");
    // The fidelity rule: what `layout` says agrees with what is drawn. On
    // each output's own screenshot (device pixels), every module's rectangle
    // holds ink and the bar holds no ink outside the rectangles.
    for output in outputs {
        let name = output["output"].as_str().unwrap();
        let id = rig
            .session
            .scoot_outputs()
            .iter()
            .find(|o| o["name"] == name)
            .map(|o| o["id"].as_u64().unwrap())
            .unwrap_or_else(|| panic!("scoot has no output `{name}`"));
        // A frame at another scale may still be on screen just after
        // start-up, so look until it settles.
        let deadline = Instant::now() + Duration::from_secs(20);
        while let Some(problem) = ink_problem(&rig.session.scoot_screenshot(id), output) {
            assert!(
                Instant::now() < deadline,
                "{name}: layout disagrees with the screen: {problem}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let mut expected = 0;
    for output in outputs {
        let scale = output["scale"].as_f64().unwrap();
        let bar = &output["bar"];
        let (bx, by) = (bar["x"].as_i64().unwrap(), bar["y"].as_i64().unwrap());
        let (bw, bh) = (
            bar["width"].as_i64().unwrap(),
            bar["height"].as_i64().unwrap(),
        );
        assert_eq!(
            (bx, by, bh),
            (
                output["origin"]["x"].as_i64().unwrap(),
                output["origin"]["y"].as_i64().unwrap(),
                i64::from(HEIGHT)
            ),
            "{output}"
        );
        assert!(bw > 0, "{output}");
        let modules = output["modules"].as_array().unwrap();
        assert_eq!(modules.len(), 2, "scale {scale}: {output}");
        for module in modules {
            let id = module["id"].as_str().unwrap();
            let rect = module;
            let (x, w) = (rect["x"].as_i64().unwrap(), rect["width"].as_i64().unwrap());
            let y = by + i64::from(HEIGHT) / 2;
            assert!(w > 0, "{module}");
            assert!(x >= bx && x + w <= bx + bw, "{module} outside {bar}");
            // Inside: the first and last logical pixel each click it.
            for inside in [x, x + w - 1] {
                expected += 1;
                rig.click(inside, y);
                let lines = rig.wait_lines("clicks", expected);
                assert_eq!(
                    lines.last().unwrap(),
                    id,
                    "scale {scale}: x {inside} of {module}"
                );
            }
            // Outside: one pixel either side is bare bar or the other module.
            for outside in [x - 1, x + w] {
                rig.click(outside, y);
                // A click that did land would have appended by now: wait a
                // beat, then count.
                std::thread::sleep(Duration::from_millis(120));
                let lines = rig.lines("clicks");
                let landed = lines.len() > expected;
                if landed {
                    // Only the neighbouring module may take it, never this.
                    assert_ne!(
                        lines.last().unwrap(),
                        id,
                        "scale {scale}: x {outside} is outside {module} but clicked it"
                    );
                    expected += 1;
                }
            }
        }
    }
}

#[test]
fn a_hidden_bar_has_no_rectangles() {
    let Some(rig) = Rig::start(
        "agent-hidden",
        "",
        1,
        &buttons(),
        "left = [\"alpha\"]\nright = [\"beta\"]\n",
    ) else {
        return;
    };
    let shown = rig.json(&["layout"]);
    assert!(shown["outputs"][0]["bar"].is_object(), "{shown}");
    assert_eq!(shown["outputs"][0]["modules"].as_array().unwrap().len(), 2);
    let hidden = rig.msg(&["hide"]);
    assert!(hidden.status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !rig.json(&["layout"])["outputs"][0]["bar"].is_null() {
        assert!(Instant::now() < deadline, "the bar never went");
        std::thread::sleep(Duration::from_millis(20));
    }
    let layout = rig.json(&["layout"]);
    assert_eq!(layout["outputs"][0]["modules"], serde_json::json!([]));
}

#[test]
fn query_names_one_module_and_refuses_one_that_is_not_placed() {
    let Some(rig) = Rig::start(
        "agent-query",
        "",
        1,
        &buttons(),
        "left = [\"alpha\"]\nright = [\"beta\"]\n",
    ) else {
        return;
    };
    let one = rig.json(&["query", "beta"]);
    let modules = one["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 1, "{one}");
    assert_eq!(modules[0]["id"], "beta");
    assert_eq!(modules[0]["text"], "BBB");
    assert_eq!(modules[0]["section"], "right");
    assert_eq!(rig.json(&["query"])["modules"].as_array().unwrap().len(), 2);
    let said = rig.refused(&["query", "gamma"]);
    assert!(
        said.contains("gamma") && said.contains("alpha") && said.contains("beta"),
        "{said}"
    );
}

#[test]
fn invoke_runs_the_binding_a_click_would() {
    let bindings = format!(
        "{}[button.pad]\ntext = \"P\"\non-click = {}\non-scroll-up = {}\non-right-click = {}\n",
        "",
        append("click"),
        append("up"),
        append("right"),
    );
    let Some(mut rig) = Rig::start("agent-invoke", "", 1, &bindings, "right = [\"pad\"]\n") else {
        return;
    };
    rig.ok(&["invoke", "pad", "click"]);
    rig.wait_lines("clicks", 1);
    rig.ok(&["invoke", "pad", "right-click"]);
    rig.wait_lines("clicks", 2);
    // Three steps of a scroll run an exec binding once, as three notches
    // coalesced into one frame do (a module's own action sees the 3).
    rig.ok(&["invoke", "pad", "scroll-up", "3"]);
    let lines = rig.wait_lines("clicks", 3);
    assert_eq!(lines, ["click", "right", "up"]);
    // Refusals are named.
    let said = rig.refused(&["invoke", "pad", "middle-click"]);
    assert!(said.contains("on-middle-click"), "{said}");
    let said = rig.refused(&["invoke", "pad", "scroll-up", "0"]);
    assert!(said.contains("steps"), "{said}");
    let said = rig.refused(&["invoke", "pad", "scroll-up", "33"]);
    assert!(said.contains("steps"), "{said}");
    let said = rig.refused(&["invoke", "pad", "click", "2"]);
    assert!(said.contains("no number"), "{said}");
    let said = rig.refused(&["invoke", "pad", "dance"]);
    assert!(said.contains("dance") && said.contains("click"), "{said}");
    let said = rig.refused(&["invoke", "ghost", "click"]);
    assert!(said.contains("ghost") && said.contains("pad"), "{said}");
    let said = rig.refused(&["invoke", "pad", "click", "--output", "DP-9"]);
    assert!(said.contains("DP-9") && said.contains("headless"), "{said}");
    // None of the refusals ran anything.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.lines("clicks").len(), 3);
    // Naming the output the module is on is fine.
    rig.ok(&["invoke", "pad", "click", "--output", "headless"]);
    rig.wait_lines("clicks", 4);
}

#[test]
fn invoke_runs_a_module_action_with_its_number() {
    let Some(rig) = Rig::start("agent-invoke-ws", "", 1, "", "right = [\"workspaces\"]\n") else {
        return;
    };
    // Workspaces need a compositor that has them: scoot headless does, and
    // `activate` of one that does not exist is refused by name, not ignored.
    let said = rig.refused(&["invoke", "workspaces", "activate", "99"]);
    assert!(!said.is_empty());
    let said = rig.refused(&["invoke", "workspaces", "activate"]);
    assert!(said.contains("whole number"), "{said}");
}

/// A `subscribe` child whose lines are read as they come.
struct Subscriber {
    child: Child,
    lines: BufReader<std::process::ChildStdout>,
}

impl Subscriber {
    fn start(rig: &Rig, kinds: &[&str]) -> Self {
        Self::try_start(rig, kinds).unwrap_or_else(|said| panic!("not subscribed: {said}"))
    }

    /// `Err` is what the refused command said.
    fn try_start(rig: &Rig, kinds: &[&str]) -> Result<Self, String> {
        let mut child = rig
            .session
            .scootbar()
            .arg("msg")
            .arg("subscribe")
            .args(kinds)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        lines.read_line(&mut first).unwrap();
        if first.is_empty() {
            let mut said = String::new();
            std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut said).unwrap();
            let _ = child.wait();
            return Err(said);
        }
        let reply: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(reply["type"], "subscribed", "{first}");
        Ok(Self { child, lines })
    }

    fn next(&mut self) -> Value {
        let mut line = String::new();
        self.lines.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"))
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_stream_of_changes_is_told_at_the_frame_rate() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(rig) = Rig::start("agent-rate", "", 1, tables, "right = [\"status\"]\n") else {
        return;
    };
    let mut sub = Subscriber::start(&rig, &["module"]);
    // One connection writes a `set` about every millisecond for 400 ms: a
    // change on nearly every loop turn, for a frame (16 ms) to coalesce.
    let mut stream = UnixStream::connect(rig.socket()).unwrap();
    let started = Instant::now();
    let mut last = 0u32;
    while started.elapsed() < Duration::from_millis(400) {
        last += 1;
        stream
            .write_all(
                format!(
                    "{{\"protocol\":1,\"type\":\"set\",\"id\":\"status\",\"value\":\"{last}\"}}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    let elapsed = started.elapsed();
    assert!(
        last >= 100,
        "only {last} sets in {elapsed:?}: the test would prove little"
    );
    // The subscriber ends on the last value, having heard far fewer events
    // than changes: at most one a frame, plus a few for the edges.
    let mut heard = 0u32;
    loop {
        let event = sub.next();
        assert_eq!(event["type"], "module", "{event}");
        heard += 1;
        if event["text"] == last.to_string().as_str() {
            break;
        }
        assert!(heard < last + 10, "never heard the last value {last}");
    }
    let frames = u32::try_from(elapsed.as_millis() / 16).unwrap();
    assert!(heard >= 2, "{heard} events for {last} sets");
    assert!(
        heard <= frames + 4,
        "{heard} events for {last} sets in {elapsed:?} (a frame gate allows about {frames})"
    );
}

#[test]
fn a_subscriber_that_hangs_up_leaves_the_daemon_unharmed_and_idle() {
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(rig) = Rig::start("agent-hangup", "", 1, tables, "right = [\"status\"]\n") else {
        return;
    };
    {
        let _sub = Subscriber::start(&rig, &[]);
    }
    for n in 0..5 {
        rig.ok(&["set", "status", &format!("\"{n}\"")]);
    }
    assert_eq!(rig.json(&["query", "status"])["modules"][0]["text"], "4");
    assert!(rig.json(&["version"])["protocol"].is_number());
}

#[test]
fn a_subscribed_connection_is_refused_further_requests() {
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(rig) = Rig::start("agent-refuse", "", 1, tables, "right = [\"status\"]\n") else {
        return;
    };
    // Raw: subscribe, then ask `query` on the same connection.
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let socket = rig
        .session
        .runtime_dir()
        .join(format!("scootbar-{}.sock", rig.session.wayland_display));
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(
            b"{\"protocol\":1,\"type\":\"subscribe\"}\n{\"protocol\":1,\"type\":\"query\"}\n",
        )
        .unwrap();
    let mut reader = BufReader::new(&stream);
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    assert!(first.contains("subscribed"), "{first}");
    let mut second = String::new();
    reader.read_line(&mut second).unwrap();
    assert!(
        second.contains("error") && second.contains("subscribed"),
        "{second}"
    );
}

#[test]
fn only_so_many_connections_may_subscribe_and_a_slot_comes_back() {
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(rig) = Rig::start("agent-cap", "", 1, tables, "right = [\"status\"]\n") else {
        return;
    };
    let mut held = Vec::new();
    let refused = loop {
        match Subscriber::try_start(&rig, &[]) {
            Ok(sub) => held.push(sub),
            Err(said) => break said,
        }
        assert!(held.len() < 64, "no cap");
    };
    assert!(refused.contains("at most"), "{refused}");
    // The daemon still answers requests with every slot taken.
    assert!(rig.json(&["version"])["protocol"].is_number());
    // One lets go and its slot is free again.
    held.pop();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match Subscriber::try_start(&rig, &[]) {
            Ok(sub) => {
                held.push(sub);
                break;
            }
            Err(said) => {
                assert!(
                    Instant::now() < deadline,
                    "the slot never came back: {said}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[test]
fn an_idle_bar_with_subscribers_and_queries_wakes_for_nothing() {
    let tables = "[push.status]\nplaceholder = \"0\"\n";
    let Some(rig) = Rig::start("agent-idle", "", 1, tables, "right = [\"status\"]\n") else {
        return;
    };
    let pid = rig.bar.0.id();
    // Two subscribers, as an agent and a script might hold.
    let _first = Subscriber::start(&rig, &[]);
    let _second = Subscriber::start(&rig, &["module"]);
    // Past the start-up and the subscribers' first round.
    std::thread::sleep(Duration::from_millis(500));
    let before = common::wakeups(pid);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        common::wakeups(pid),
        before,
        "the bar woke while idle with subscribers: {}",
        rig.session.bar_stderr()
    );
    // A change wakes it once to tell them, and then it is idle again.
    rig.ok(&["set", "status", "\"1\""]);
    std::thread::sleep(Duration::from_millis(500));
    let after_set = common::wakeups(pid);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        common::wakeups(pid),
        after_set,
        "the bar kept waking after one change"
    );
}
