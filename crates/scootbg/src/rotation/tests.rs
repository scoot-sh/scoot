use super::{
    EveryError, ListError, MAX_EVERY_SECS, MAX_LISTED, list_dir, order, parse_every,
    shuffle_with_seed,
};
use std::os::unix::ffi::OsStringExt;

#[test]
fn every_takes_durations_at_least_a_minute_whole_minutes_at_most_a_week() {
    assert_eq!(parse_every("30m"), Ok(1800));
    assert_eq!(parse_every("1m"), Ok(60));
    assert_eq!(parse_every("60s"), Ok(60));
    assert_eq!(parse_every("120s"), Ok(120));
    assert_eq!(parse_every("2h"), Ok(7200));
    assert_eq!(parse_every("1d"), Ok(86400));
    assert_eq!(parse_every("7d"), Ok(7 * 24 * 3600));
    assert_eq!(parse_every("7d"), Ok(MAX_EVERY_SECS));
    // Refused, naming what is wrong.
    assert_eq!(
        parse_every("30s"),
        Err(EveryError::TooShort),
        "under a minute"
    );
    assert_eq!(parse_every("0m"), Err(EveryError::TooShort), "zero");
    assert_eq!(
        parse_every("90s"),
        Err(EveryError::NotAligned),
        "over a minute but not whole minutes"
    );
    assert_eq!(parse_every("8d"), Err(EveryError::TooLong), "over a week");
    assert_eq!(
        parse_every("9999999999d"),
        Err(EveryError::TooLong),
        "overflow counts as too long, never wraps"
    );
    for bad in ["", "m", "30", "30M", "1H", "10x", "3 m", "-5m", "1.5h"] {
        assert_eq!(
            parse_every(bad),
            Err(EveryError::BadFormat(bad.to_owned())),
            "{bad:?} is not digits and a unit"
        );
    }
}

#[test]
fn listing_is_sorted_regular_files_only() {
    let dir = std::env::temp_dir().join(format!(
        "sbg-rotation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let remove = || std::fs::remove_dir_all(&dir).unwrap();
    std::fs::write(dir.join("b.jpg"), b"fake").unwrap();
    std::fs::write(dir.join("a.png"), b"fake").unwrap();
    std::fs::write(dir.join("notes.txt"), b"not an image, still listed").unwrap();
    std::fs::write(dir.join(".hidden.webp"), b"fake").unwrap();
    std::fs::create_dir(dir.join("subdir")).unwrap();
    let listed = list_dir(&dir).unwrap();
    assert_eq!(listed.skipped_non_utf8, 0);
    assert_eq!(
        listed.files,
        vec![
            dir.join(".hidden.webp").to_string_lossy().into_owned(),
            dir.join("a.png").to_string_lossy().into_owned(),
            dir.join("b.jpg").to_string_lossy().into_owned(),
            dir.join("notes.txt").to_string_lossy().into_owned(),
        ],
        "sorted, hidden and non-images included, the subdirectory left out"
    );
    remove();
}

#[test]
fn listing_skips_names_the_protocol_cannot_carry() {
    let dir = std::env::temp_dir().join(format!(
        "sbg-rotation-utf8-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("fine.png"), b"fake").unwrap();
    let raw = std::ffi::OsString::from_vec(vec![0xff, 0xfe, b'.', b'p', b'n', b'g']);
    std::fs::write(dir.join(&raw), b"fake").unwrap();
    let listed = list_dir(&dir).unwrap();
    assert_eq!(listed.skipped_non_utf8, 1);
    assert_eq!(listed.files.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn listing_a_missing_directory_is_an_error() {
    let missing = std::env::temp_dir().join(format!(
        "sbg-rotation-gone-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    assert!(matches!(list_dir(&missing), Err(ListError::Open(_))));
}

#[test]
fn listing_names_an_unreadable_entry_rather_than_the_directory() {
    // A symlink loop: the directory opens, but stating the entry fails
    // with ELOOP however the entries come back, and however the tests
    // run (permissions would not fail for root).
    let dir = std::env::temp_dir().join(format!(
        "sbg-rotation-loop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("fine.png"), b"fake").unwrap();
    std::os::unix::fs::symlink("loop", dir.join("loop")).unwrap();
    let error = list_dir(&dir).unwrap_err();
    let detail = error.to_string();
    assert!(
        matches!(error, ListError::Entry { .. }),
        "an entry error, not an open error: {detail}"
    );
    assert!(
        detail.contains("entry"),
        "the message names the entry: {detail}"
    );
    assert!(
        detail.contains("loop"),
        "the message names which entry: {detail}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn listing_refuses_past_the_cap_rather_than_truncating() {
    let dir = std::env::temp_dir().join(format!(
        "sbg-rotation-cap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    for i in 0..=MAX_LISTED {
        std::fs::write(dir.join(format!("{i:05}.png")), b"fake").unwrap();
    }
    let started = std::time::Instant::now();
    let error = list_dir(&dir).unwrap_err();
    let elapsed = started.elapsed();
    eprintln!("list_dir past the cap ({MAX_LISTED}+1 files): {elapsed:?}");
    match error {
        ListError::TooMany { seen } => assert_eq!(seen, MAX_LISTED + 1),
        other => panic!("a cap refusal, not {other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains(&MAX_LISTED.to_string()), "{message}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shuffling_replays_with_a_seed_and_keeps_the_set() {
    let sorted = vec![
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
        "e".to_owned(),
        "f".to_owned(),
    ];
    let mut first = sorted.clone();
    shuffle_with_seed(&mut first, 42);
    let mut second = sorted.clone();
    shuffle_with_seed(&mut second, 42);
    assert_eq!(first, second, "a fixed seed replays");
    let mut ranked = first.clone();
    ranked.sort();
    assert_eq!(ranked, sorted, "a permutation, nothing lost or doubled");
    // Unshuffled stays sorted.
    assert_eq!(order(sorted.clone(), false, 42), sorted);
    assert_eq!(order(sorted.clone(), true, 42), first);
}

#[test]
fn listing_follows_links_to_files_but_not_into_directories() {
    let dir = std::env::temp_dir().join(format!(
        "sbg-rotation-links-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("real.png"), b"fake").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    // A linked image shows; a linked directory does not recurse.
    std::os::unix::fs::symlink("real.png", dir.join("linked.png")).unwrap();
    std::os::unix::fs::symlink("sub", dir.join("linked-dir")).unwrap();
    let listed = list_dir(&dir).unwrap();
    assert_eq!(
        listed.files,
        vec![
            dir.join("linked.png").to_string_lossy().into_owned(),
            dir.join("real.png").to_string_lossy().into_owned(),
        ],
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
