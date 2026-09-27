use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{JobError, Worker, work};
use crate::color::Color;
use crate::image::decode::DecodeError;
use crate::image::render::Look;
use crate::image::{Filter, Mode, samples};
use crate::jobs::Target;
use crate::outputs::{OutputId, Outputs};
use crate::wallpaper::Image;

fn ids(n: usize) -> Vec<OutputId> {
    let mut outputs: Outputs<()> = Outputs::default();
    (0..n).map(|i| outputs.add(i as u32, |_| ())).collect()
}

fn image(path: &std::path::Path, mode: Mode) -> Image {
    Image {
        path: path.to_str().unwrap().into(),
        look: Look {
            mode,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial: 1,
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sbg-worker-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One decode for every target: each distinct size drawn once, and an
/// output of a size already drawn gets a copy of the same pixels.
#[test]
fn every_target_gets_its_buffer_from_one_decode() {
    let dir = scratch("targets");
    let file = dir.join("q.jpg");
    std::fs::write(&file, samples::QUADRANTS_JPEG).unwrap();
    let [a, b, c] = ids(3)[..] else {
        unreachable!()
    };
    let targets = [
        Target {
            output: a,
            dims: (64, 40),
        },
        Target {
            output: b,
            dims: (30, 20),
        },
        Target {
            output: c,
            dims: (64, 40),
        },
    ];
    let mut done = work(&image(&file, Mode::Fill), &targets).unwrap();
    assert_eq!(done.len(), 3);
    done.sort_by_key(|(target, _)| target.dims);
    let mut pixels: Vec<(Target, Vec<u8>)> = done
        .into_iter()
        .map(|(target, buffer)| {
            let mut buffer = buffer.unwrap();
            let geometry = buffer.geometry();
            assert_eq!((geometry.width as u32, geometry.height as u32), target.dims);
            (target, buffer.pixels_mut().to_vec())
        })
        .collect();
    pixels.sort_by_key(|(target, _)| (target.dims, target.output == c));
    assert_eq!(pixels[1].1, pixels[2].1, "the same size: the same pixels");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_cannot_be_decoded_fails_the_whole_job() {
    let dir = scratch("bad");
    let [a] = ids(1)[..] else { unreachable!() };
    let target = [Target {
        output: a,
        dims: (8, 8),
    }];
    let missing = work(&image(&dir.join("missing.png"), Mode::Fit), &target);
    assert!(matches!(
        missing,
        Err(JobError::Decode(DecodeError::NotFound))
    ));
    // No targets (no output configured yet): decoded all the same, to
    // know whether it can be shown.
    let text = dir.join("text.png");
    std::fs::write(&text, "text").unwrap();
    assert!(matches!(
        work(&image(&text, Mode::Fit), &[]),
        Err(JobError::Decode(DecodeError::NotAnImage))
    ));
    let good = dir.join("q.jpg");
    std::fs::write(&good, samples::QUADRANTS_JPEG).unwrap();
    assert!(work(&image(&good, Mode::Fit), &[]).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A target too large for `wl_shm` fails alone, at once (before anything
/// that size is scaled: this would be 4.8 GB), and the others are drawn.
#[test]
fn one_target_failing_leaves_the_others() {
    let dir = scratch("one");
    let file = dir.join("q.jpg");
    std::fs::write(&file, samples::QUADRANTS_JPEG).unwrap();
    let [a, b] = ids(2)[..] else { unreachable!() };
    let targets = [
        Target {
            output: a,
            dims: (40_000, 40_000),
        },
        Target {
            output: b,
            dims: (16, 16),
        },
    ];
    for mode in Mode::ALL {
        let done = work(&image(&file, mode), &targets).unwrap();
        for (target, result) in done {
            assert_eq!(result.is_ok(), target.output == b, "{mode:?} {target:?}");
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The thread's result comes back through the channel and wakes the
/// eventfd; the thread is gone afterwards.
#[test]
fn a_job_runs_on_its_own_thread_and_wakes_the_loop() {
    let dir = scratch("thread");
    let file = dir.join("q.jpg");
    std::fs::write(&file, samples::QUADRANTS_JPEG).unwrap();
    let mut worker = Worker::new().unwrap();
    assert!(worker.take().is_none(), "nothing yet");
    let [a] = ids(1)[..] else { unreachable!() };
    let job = Arc::new(image(&file, Mode::Fill));
    worker
        .start(
            job,
            vec![Target {
                output: a,
                dims: (32, 32),
            }],
        )
        .unwrap();
    assert!(readable(&worker, 20));
    let done = worker.take().unwrap().unwrap();
    assert_eq!(done.len(), 1);
    // Read once: not readable again until the next job.
    assert!(!readable(&worker, 0));
    assert!(worker.take().is_none());
    // The thread has ended (give it a moment to finish unwinding).
    let deadline = Instant::now() + Duration::from_secs(5);
    while threads_named("scootbg-decode") > 0 {
        assert!(Instant::now() < deadline, "the decoding thread stayed");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

fn threads_named(name: &str) -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|task| {
            std::fs::read_to_string(task.path().join("comm")).is_ok_and(|comm| comm.trim() == name)
        })
        .count()
}

fn refuse(_: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    Err(std::io::Error::from_raw_os_error(11))
}

fn readable(worker: &Worker, seconds: i64) -> bool {
    let fd = worker.fd();
    let mut fds = [rustix::event::PollFd::new(
        &fd,
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: seconds,
        tv_nsec: 0,
    };
    rustix::event::poll(&mut fds, Some(&timeout)).unwrap() == 1
}

/// A thread that cannot be started reports that once, as `start`'s error,
/// and leaves nothing behind: the next job's result is its own. (Before,
/// the guard was made outside the thread and dropped with the refused
/// closure, sending a "thread failed" that the next job then took for its
/// own result, and every later result was one job late.)
#[test]
fn a_thread_that_cannot_start_leaves_no_result_behind() {
    let dir = scratch("spawn");
    let blue = dir.join("blue.png");
    let yellow = dir.join("yellow.png");
    for (file, rgb) in [(&blue, [0, 0, 255]), (&yellow, [255, 255, 0])] {
        let data = rgb.repeat(4 * 4);
        std::fs::write(
            file,
            samples::png(4, 4, png::ColorType::Rgb, png::BitDepth::Eight, &data, None),
        )
        .unwrap();
    }
    let [a] = ids(1)[..] else { unreachable!() };
    let target = || {
        vec![Target {
            output: a,
            dims: (2, 2),
        }]
    };
    let mut worker = Worker::with_spawn(refuse).unwrap();
    for _ in 0..2 {
        let refused = worker.start(Arc::new(image(&blue, Mode::Stretch)), target());
        assert!(matches!(refused, Err(JobError::Spawn(_))));
        assert!(!readable(&worker, 0), "a refused spawn wakes nothing");
        assert!(worker.take().is_none(), "and sends nothing");
    }
    worker.set_spawn(super::spawn_thread);
    for (file, want) in [(&blue, [255, 0, 0, 0xff]), (&yellow, [0, 255, 255, 0xff])] {
        worker
            .start(Arc::new(image(file, Mode::Stretch)), target())
            .unwrap();
        assert!(readable(&worker, 20));
        let mut done = worker.take().unwrap().unwrap();
        let (_, buffer) = done.pop().unwrap();
        let mut buffer = buffer.unwrap();
        assert_eq!(buffer.pixels_mut()[..4], want, "{file:?}: its own pixels");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Only the awaited job's result is taken: one carrying another ticket is
/// discarded, never paired with the wrong job.
#[test]
fn a_result_for_another_job_is_discarded() {
    let dir = scratch("ticket");
    let file = dir.join("q.jpg");
    std::fs::write(&file, samples::QUADRANTS_JPEG).unwrap();
    let mut worker = Worker::new().unwrap();
    // Nothing awaited: a stray result is dropped.
    worker.inject(7, Err(JobError::Lost));
    assert!(worker.take().is_none());
    let [a] = ids(1)[..] else { unreachable!() };
    worker
        .start(
            Arc::new(image(&file, Mode::Fill)),
            vec![Target {
                output: a,
                dims: (8, 8),
            }],
        )
        .unwrap();
    // A stale one ahead of the real one: skipped.
    worker.inject(0, Err(JobError::Lost));
    let deadline = Instant::now() + Duration::from_secs(20);
    let done = loop {
        if let Some(done) = worker.take() {
            break done;
        }
        assert!(Instant::now() < deadline, "no result");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(done.unwrap().len(), 1, "the job's own result");
    std::fs::remove_dir_all(&dir).unwrap();
}
