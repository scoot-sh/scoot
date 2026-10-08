use super::Rotation;
use crate::image::render::Look;
use crate::transition::Spec;

fn look() -> Look {
    Look {
        mode: crate::image::Mode::Fill,
        fill: crate::color::Color::parse("#000000").unwrap(),
        filter: crate::image::Filter::Lanczos3,
    }
}

#[test]
fn starting_lists_once_and_fires_one_interval_out() {
    let dir = std::env::temp_dir().join(format!(
        "sbg-drotation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("b.png"), b"fake").unwrap();
    std::fs::write(dir.join("a.png"), b"fake").unwrap();
    let path = dir.to_str().unwrap().to_owned();
    // Without the debug shortening (unset here): the request's minutes.
    let rotation = Rotation::start(
        &path,
        1800,
        false,
        None,
        look(),
        Spec::none(),
        std::time::Instant::now(),
    )
    .unwrap();
    assert_eq!(rotation.first(), format!("{path}/a.png").as_str());
    assert_eq!(rotation.every(), std::time::Duration::from_secs(1800));
    assert!(!rotation.due(std::time::Instant::now()));
    assert!(rotation.due(std::time::Instant::now() + std::time::Duration::from_secs(1800)));
    let info = rotation.info();
    assert_eq!(info.directory, path.as_str());
    assert_eq!(info.every_secs, 1800);
    assert!(!info.shuffle);
    assert_eq!(info.files, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn starting_refuses_what_has_nothing_to_cycle() {
    let missing = std::env::temp_dir().join(format!(
        "sbg-drotation-gone-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    assert_eq!(
        Rotation::start(
            missing.to_str().unwrap(),
            1800,
            false,
            None,
            look(),
            Spec::none(),
            std::time::Instant::now()
        )
        .unwrap_err(),
        super::StartError::NotDirectory
    );
    let file = std::env::temp_dir().join(format!(
        "sbg-drotation-file-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::write(&file, b"not a directory").unwrap();
    assert_eq!(
        Rotation::start(
            file.to_str().unwrap(),
            1800,
            false,
            None,
            look(),
            Spec::none(),
            std::time::Instant::now()
        )
        .unwrap_err(),
        super::StartError::NotDirectory
    );
    std::fs::remove_file(&file).unwrap();
    let empty = std::env::temp_dir().join(format!(
        "sbg-drotation-empty-{}-{}",
        std::process::id(),
        std::time::UNIX_EPOCH.elapsed().unwrap().subsec_nanos()
    ));
    std::fs::create_dir(&empty).unwrap();
    assert_eq!(
        Rotation::start(
            empty.to_str().unwrap(),
            1800,
            false,
            None,
            look(),
            Spec::none(),
            std::time::Instant::now()
        )
        .unwrap_err(),
        super::StartError::Empty
    );
    std::fs::remove_dir(&empty).unwrap();
}
