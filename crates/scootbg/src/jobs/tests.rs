use std::sync::Arc;

use super::{Busy, Jobs, MAX_TRIALS, Target, Trial};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::outputs::{OutputId, Outputs};
use crate::wallpaper::Image;

fn image(serial: u64) -> Arc<Image> {
    Arc::new(Image {
        path: format!("/{serial}.png"),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial,
    })
}

/// Output ids, made the only way they can be.
fn ids(n: usize) -> Vec<OutputId> {
    let mut outputs: Outputs<()> = Outputs::default();
    (0..n).map(|i| outputs.add(i as u32, |_| ())).collect()
}

fn target(output: OutputId, width: u32) -> Target {
    Target {
        output,
        dims: (width, 100),
    }
}

fn all(conn: u32) -> Trial<u32> {
    Trial { conn, output: None }
}

#[test]
fn the_newest_runs_first_one_at_a_time() {
    let [a] = ids(1)[..] else { unreachable!() };
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.trial(image(1), all(10), vec![target(a, 100)]).unwrap();
    jobs.trial(image(3), all(30), vec![target(a, 100)]).unwrap();
    jobs.trial(image(2), all(20), vec![]).unwrap();
    let (running, targets) = jobs.next(|_, _| true).unwrap();
    assert_eq!((running.serial, targets), (3, vec![target(a, 100)]));
    assert!(jobs.next(|_, _| true).is_none(), "one at a time");
    let done = jobs.finished().unwrap();
    assert_eq!(done.trial, Some(all(30)));
    assert_eq!(jobs.next(|_, _| true).unwrap().0.serial, 2);
    jobs.finished();
    assert_eq!(jobs.next(|_, _| true).unwrap().0.serial, 1);
    jobs.finished();
    assert!(jobs.next(|_, _| true).is_none());
}

#[test]
fn superseded_trials_are_answered_without_running() {
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.trial(image(1), all(10), vec![]).unwrap();
    jobs.trial(
        image(2),
        Trial {
            conn: 20,
            output: Some("DP-1".into()),
        },
        vec![],
    )
    .unwrap();
    jobs.trial(image(4), all(40), vec![]).unwrap();
    // Running: 4. Suppose it lands and wins everything below 4.
    jobs.next(|_, _| true).unwrap();
    let mut answered = Vec::new();
    jobs.sweep(
        |_, serial| serial < 4,
        |conn, serial| answered.push((conn, serial)),
    );
    assert_eq!(answered, [(10, 1), (20, 2)]);
    assert_eq!(jobs.queued(), 0);
    // The running job is not swept: it is answered when it lands.
    assert!(jobs.is_running());
}

#[test]
fn a_sweep_passes_each_trials_output() {
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.trial(
        image(1),
        Trial {
            conn: 1,
            output: Some("A".into()),
        },
        vec![],
    )
    .unwrap();
    jobs.trial(
        image(2),
        Trial {
            conn: 2,
            output: Some("B".into()),
        },
        vec![],
    )
    .unwrap();
    let mut answered = Vec::new();
    jobs.sweep(
        |output, _| output == Some("B"),
        |conn, _| answered.push(conn),
    );
    assert_eq!(answered, [2]);
    assert_eq!(jobs.queued(), 1);
}

#[test]
fn renders_merge_and_are_not_asked_twice() {
    let [a, b] = ids(2)[..] else { unreachable!() };
    let img = image(5);
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.render(&img, target(a, 100));
    jobs.render(&img, target(b, 200));
    jobs.render(&img, target(a, 100));
    assert_eq!(jobs.queued(), 1, "one job for the image");
    // A new size for an output replaces its old one.
    jobs.render(&img, target(a, 300));
    let (_, targets) = jobs.next(|_, _| true).unwrap();
    assert_eq!(targets, vec![target(b, 200), target(a, 300)]);
    // While it runs, the same target is covered; another is a new job.
    jobs.render(&img, target(b, 200));
    assert_eq!(jobs.queued(), 0);
    jobs.render(&img, target(b, 400));
    assert_eq!(jobs.queued(), 1);
    // A trial's targets are covered too.
    let trial = image(6);
    jobs.trial(Arc::clone(&trial), all(1), vec![target(a, 100)])
        .unwrap();
    jobs.render(&trial, target(a, 100));
    assert_eq!(jobs.queued(), 2);
}

#[test]
fn unwanted_render_targets_are_dropped_before_running() {
    let [a, b] = ids(2)[..] else { unreachable!() };
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.render(&image(1), target(a, 100));
    jobs.render(&image(2), target(a, 100));
    jobs.render(&image(2), target(b, 100));
    // Image 2 is wanted on `b` only now; image 1 nowhere.
    let (running, targets) = jobs
        .next(|img, t| img.serial == 2 && t.output == b)
        .unwrap();
    assert_eq!((running.serial, targets), (2, vec![target(b, 100)]));
    jobs.finished();
    assert!(
        jobs.next(|img, t| img.serial == 2 && t.output == b)
            .is_none()
    );
    assert_eq!(jobs.queued(), 0, "the empty job for image 1 is gone");
}

#[test]
fn trials_are_bounded() {
    let mut jobs: Jobs<u32> = Jobs::default();
    for i in 0..MAX_TRIALS as u64 {
        jobs.trial(image(i + 1), all(i as u32), vec![]).unwrap();
    }
    assert_eq!(jobs.trial(image(999), all(999), vec![]), Err(Busy));
    // A running trial is not queued: room for one more.
    jobs.next(|_, _| true);
    assert_eq!(jobs.trial(image(1000), all(1000), vec![]), Ok(()));
    // Renders are not trials, and not refused.
    let [a] = ids(1)[..] else { unreachable!() };
    jobs.render(&image(7), target(a, 1));
    assert_eq!(jobs.queued(), MAX_TRIALS + 1);
}

/// A render target another output's pixels serve leaves the queue without
/// being decoded for; the rest stay, and a render left with no targets
/// goes. Trials are never offered: nothing has their image yet.
#[test]
fn targets_served_by_shared_pixels_are_not_decoded_for() {
    let [a, b, c] = ids(3)[..] else {
        unreachable!()
    };
    let mut jobs: Jobs<u32> = Jobs::default();
    let shown = image(1);
    let other = image(2);
    jobs.render(&shown, target(a, 100));
    jobs.render(&shown, target(b, 200));
    jobs.render(&other, target(c, 100));
    jobs.trial(image(3), all(1), vec![target(a, 100)]).unwrap();
    // Nothing served: nothing changes, and it says so.
    assert!(!jobs.satisfy(|_, _| false));
    assert_eq!(jobs.queued(), 3);
    // Image 1 at width 100 is on screen somewhere.
    let mut asked = Vec::new();
    let served = jobs.satisfy(|img, t| {
        asked.push(img.serial);
        img.serial == 1 && t.dims.0 == 100
    });
    assert!(served);
    assert!(!asked.contains(&3), "a trial is never offered: {asked:?}");
    assert_eq!(jobs.queued(), 3, "image 1 still needs width 200");
    let (running, targets) = jobs.next(|_, _| true).unwrap();
    assert_eq!((running.serial, targets), (3, vec![target(a, 100)]));
    jobs.finished();
    // Everything left is served: both renders go.
    assert!(jobs.satisfy(|_, _| true));
    assert_eq!(jobs.queued(), 0);
    assert!(jobs.next(|_, _| true).is_none());
}

/// The running job is the worker's: nothing is taken from it.
#[test]
fn the_running_job_is_not_satisfied_under_the_worker() {
    let [a] = ids(1)[..] else { unreachable!() };
    let mut jobs: Jobs<u32> = Jobs::default();
    jobs.render(&image(1), target(a, 100));
    jobs.next(|_, _| true).unwrap();
    assert!(!jobs.satisfy(|_, _| true));
    let finished = jobs.finished().unwrap();
    assert_eq!(finished.targets, vec![target(a, 100)]);
}
