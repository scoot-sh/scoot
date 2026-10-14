//! The binaries' version label (`build_label.rs`, loaded by each binary's
//! build script): the pure formatting and parsing, then the git queries
//! against throwaway repositories.

#[path = "../build_label.rs"]
mod build_label;

use build_label::{GitState, git_state, label, parse_describe};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tagged(tag_version: &str, since: u64, sha: &str) -> GitState {
    GitState::Tagged {
        tag_version: tag_version.to_string(),
        since,
        sha: sha.to_string(),
    }
}

#[test]
fn no_git_is_the_plain_version() {
    assert_eq!(label("0.2.0", None), "0.2.0");
}

#[test]
fn a_build_at_its_release_tag_is_the_plain_version() {
    assert_eq!(
        label("0.2.0", Some(&tagged("0.2.0", 0, "615608c"))),
        "0.2.0"
    );
}

#[test]
fn a_build_past_the_tag_names_the_tag_the_count_and_the_commit() {
    assert_eq!(
        label("0.2.0", Some(&tagged("0.2.0", 14, "615608c"))),
        "0.2.0+14.g615608c"
    );
}

#[test]
fn the_release_pr_branch_still_names_the_last_tag() {
    // The release PR has bumped the manifest to 0.2.1 but v0.2.1 does not
    // exist yet: the build is the last release plus commits, not 0.2.1.
    assert_eq!(
        label("0.2.1", Some(&tagged("0.2.0", 3, "abc1234"))),
        "0.2.0+3.gabc1234"
    );
}

#[test]
fn a_tag_whose_version_is_not_the_manifests_is_labeled_even_at_zero() {
    // Tagging a commit whose manifest disagrees is a wrong cut the release
    // gate refuses; the binary must not claim to be that release either.
    assert_eq!(
        label("0.2.1", Some(&tagged("0.2.0", 0, "abc1234"))),
        "0.2.0+0.gabc1234"
    );
}

#[test]
fn before_any_tag_the_manifest_version_and_total_count() {
    let state = GitState::Untagged {
        commits: 2081,
        sha: "615608c".to_string(),
    };
    assert_eq!(label("0.2.0", Some(&state)), "0.2.0+2081.g615608c");
}

#[test]
fn describe_output_parses() {
    assert_eq!(
        parse_describe("v0.2.0-14-g615608c\n"),
        Some(tagged("0.2.0", 14, "615608c"))
    );
    assert_eq!(
        parse_describe("v10.20.30-0-gdeadbeefcafe"),
        Some(tagged("10.20.30", 0, "deadbeefcafe"))
    );
}

#[test]
fn malformed_describe_output_is_refused() {
    for bad in [
        "",
        "615608c",
        "v0.2.0",
        "v0.2.0-14",
        "0.2.0-14-g615608c",
        "v0.2-14-g615608c",
        "v0.2.0.1-14-g615608c",
        "v0.2.x-14-g615608c",
        "v0.2.0-rc1-14-g615608c",
        "v0.2.0-x-g615608c",
        "v0.2.0-14-615608c",
        "v0.2.0-14-gnothex",
        "v0.2.0-14-g",
        "nightly-3-g615608c",
    ] {
        assert_eq!(parse_describe(bad), None, "{bad:?} parsed");
    }
}

/// A throwaway repository, or `None` (and the test skips, loudly) when
/// there is no `git` to make one with.
struct Repo {
    root: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Option<Repo> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipping: no git on PATH");
            return None;
        }
        let root =
            std::env::temp_dir().join(format!("scoot-build-label-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("crates/scoot")).unwrap();
        let repo = Repo { root };
        repo.git(&["init", "-q", "-b", "main"]);
        Some(repo)
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn commit(&self, n: usize) {
        for _ in 0..n {
            self.git(&["commit", "-q", "--allow-empty", "-m", "c"]);
        }
    }

    fn crate_dir(&self) -> PathBuf {
        self.root.join("crates/scoot")
    }

    fn short(&self) -> String {
        self.git(&["rev-parse", "--short=7", "HEAD"])
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_checkout_without_tags_counts_every_commit() {
    let Some(repo) = Repo::new("untagged") else {
        return;
    };
    repo.commit(3);
    let (state, watch) = git_state(&repo.crate_dir(), &repo.root);
    assert_eq!(
        state,
        Some(GitState::Untagged {
            commits: 3,
            sha: repo.short()
        })
    );
    assert!(watch.iter().any(|p| p.ends_with("HEAD")), "{watch:?}");
    assert!(watch.iter().all(|p| p.exists()), "{watch:?}");
}

#[test]
fn a_checkout_past_a_release_tag_names_it() {
    let Some(repo) = Repo::new("tagged") else {
        return;
    };
    repo.commit(1);
    repo.git(&["tag", "-a", "-m", "v0.2.0", "v0.2.0"]);
    // A later non-release tag (nightly) must not be what describe picks.
    repo.commit(2);
    repo.git(&["tag", "-f", "nightly"]);
    let (state, _) = git_state(&repo.crate_dir(), &repo.root);
    assert_eq!(state, Some(tagged("0.2.0", 2, &repo.short())));
    assert_eq!(
        label("0.2.0", state.as_ref()),
        format!("0.2.0+2.g{}", repo.short())
    );
}

#[test]
fn a_checkout_at_the_release_tag_is_the_release() {
    let Some(repo) = Repo::new("release") else {
        return;
    };
    repo.commit(4);
    repo.git(&["tag", "v0.2.0"]);
    let (state, _) = git_state(&repo.crate_dir(), &repo.root);
    assert_eq!(label("0.2.0", state.as_ref()), "0.2.0");
}

#[test]
fn a_tree_inside_some_other_repository_is_not_labeled() {
    // A release tarball unpacked inside a packager's own checkout (an AUR
    // clone): git answers for the outer repository, whose history is not
    // this tree's.
    let Some(repo) = Repo::new("nested") else {
        return;
    };
    repo.commit(5);
    let workspace = repo.root.join("src/scoot-0.2.0");
    std::fs::create_dir_all(workspace.join("crates/scoot")).unwrap();
    let (state, watch) = git_state(&workspace.join("crates/scoot"), &workspace);
    assert_eq!(state, None);
    assert!(watch.is_empty());
}

#[test]
fn a_tree_with_no_repository_at_all_is_not_labeled() {
    let dir = std::env::temp_dir().join(format!("scoot-build-label-bare-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("crates/scoot")).unwrap();
    // The temp dir itself might sit inside a repository on some machines;
    // the root check refuses that too, so the answer is None either way.
    let (state, _) = git_state(&dir.join("crates/scoot"), Path::new(&dir));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(state, None);
}

#[test]
fn a_pre_release_tag_is_skipped_for_the_last_release() {
    let Some(repo) = Repo::new("prerelease") else {
        return;
    };
    repo.commit(1);
    repo.git(&["tag", "v0.2.0"]);
    repo.commit(1);
    repo.git(&["tag", "v0.3.0-rc1"]);
    repo.commit(1);
    let (state, _) = git_state(&repo.crate_dir(), &repo.root);
    assert_eq!(state, Some(tagged("0.2.0", 2, &repo.short())));
}
