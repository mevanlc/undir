use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use undir::{CreateMode, OnError, Options, Phase, Preflight};

fn options(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Options {
    Options {
        srcdir: source.into(),
        dstdir: destination.into(),
        merge: false,
        overwrite: false,
        on_error: OnError::Stop,
        keep: false,
        preflight: Preflight::Fast,
        create: CreateMode::Existing,
        strict: false,
    }
}

fn roots() -> (TempDir, PathBuf, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    (temporary, source, destination)
}

#[test]
fn moves_every_child_and_removes_source() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("visible"), "visible").unwrap();
    fs::write(source.join(".hidden"), "hidden").unwrap();
    fs::create_dir(source.join("directory")).unwrap();
    fs::write(source.join("directory/child"), "child").unwrap();

    undir::run(&options(&source, &destination)).unwrap();

    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(destination.join("visible")).unwrap(),
        "visible"
    );
    assert_eq!(
        fs::read_to_string(destination.join(".hidden")).unwrap(),
        "hidden"
    );
    assert_eq!(
        fs::read_to_string(destination.join("directory/child")).unwrap(),
        "child"
    );
}

#[test]
fn keep_preserves_an_empty_source() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("file"), "contents").unwrap();
    let mut opts = options(&source, &destination);
    opts.keep = true;

    undir::run(&opts).unwrap();

    assert!(source.is_dir());
    assert!(fs::read_dir(&source).unwrap().next().is_none());
}

#[test]
fn merge_and_overwrite_are_both_required_before_mutation() {
    let (_temporary, source, destination) = roots();
    fs::create_dir(source.join("shared")).unwrap();
    fs::create_dir(destination.join("shared")).unwrap();
    fs::write(source.join("shared/a"), "source a").unwrap();
    fs::write(source.join("shared/b"), "source b").unwrap();
    fs::write(destination.join("shared/a"), "destination a").unwrap();
    fs::write(destination.join("shared/b"), "destination b").unwrap();
    let mut opts = options(&source, &destination);
    opts.preflight = Preflight::Full;

    let report = undir::run(&opts).unwrap_err();

    assert_eq!(report.issues().len(), 3);
    assert!(
        report
            .issues()
            .iter()
            .all(|issue| issue.phase() == Phase::Preflight)
    );
    assert_eq!(
        fs::read_to_string(source.join("shared/a")).unwrap(),
        "source a"
    );
    assert_eq!(
        fs::read_to_string(destination.join("shared/a")).unwrap(),
        "destination a"
    );
}

#[test]
fn recursively_merges_and_overwrites() {
    let (_temporary, source, destination) = roots();
    fs::create_dir_all(source.join("shared/nested")).unwrap();
    fs::create_dir_all(destination.join("shared/nested")).unwrap();
    fs::write(source.join("shared/new"), "new").unwrap();
    fs::write(source.join("shared/nested/replaced"), "source").unwrap();
    fs::write(destination.join("shared/nested/replaced"), "destination").unwrap();
    let mut opts = options(&source, &destination);
    opts.merge = true;
    opts.overwrite = true;

    undir::run(&opts).unwrap();

    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(destination.join("shared/new")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(destination.join("shared/nested/replaced")).unwrap(),
        "source"
    );
}

#[test]
fn directory_nondirectory_collisions_are_never_overridable() {
    let (_temporary, source, destination) = roots();
    fs::create_dir(source.join("source-dir")).unwrap();
    fs::write(destination.join("source-dir"), "file").unwrap();
    fs::write(source.join("source-file"), "file").unwrap();
    fs::create_dir(destination.join("source-file")).unwrap();
    let mut opts = options(&source, &destination);
    opts.merge = true;
    opts.overwrite = true;
    opts.preflight = Preflight::Full;

    let report = undir::run(&opts).unwrap_err();

    assert_eq!(report.issues().len(), 2);
    assert!(source.join("source-dir").is_dir());
    assert!(source.join("source-file").is_file());
}

#[test]
fn preflight_off_still_blocks_hard_errors_before_any_move() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("a-movable"), "movable").unwrap();
    fs::create_dir(source.join("z-directory")).unwrap();
    fs::write(destination.join("z-directory"), "nondirectory").unwrap();
    let mut opts = options(&source, &destination);
    opts.preflight = Preflight::Off;
    opts.merge = true;
    opts.overwrite = true;

    let report = undir::run(&opts).unwrap_err();

    assert!(
        report
            .issues()
            .iter()
            .all(|issue| issue.phase() == Phase::Preflight)
    );
    assert!(source.join("a-movable").exists());
    assert!(!destination.join("a-movable").exists());
}

#[test]
fn destination_cannot_replace_the_source_root_entry() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path();
    let source = temporary.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("source")).unwrap();
    let mut opts = options(&source, destination);
    opts.merge = true;

    let report = undir::run(&opts).unwrap_err();

    assert!(
        report.issues()[0]
            .message()
            .contains("overwrite the srcdir entry")
    );
    assert!(source.join("source").is_dir());
}

#[test]
fn preflight_off_continue_processes_independent_siblings() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("a-collision"), "source").unwrap();
    fs::write(source.join("b-movable"), "movable").unwrap();
    fs::write(destination.join("a-collision"), "destination").unwrap();
    let mut opts = options(&source, &destination);
    opts.preflight = Preflight::Off;
    opts.on_error = OnError::Continue;

    let report = undir::run(&opts).unwrap_err();

    assert_eq!(report.issues().len(), 1);
    assert_eq!(report.issues()[0].phase(), Phase::Mutation);
    assert!(source.join("a-collision").exists());
    assert!(!source.join("b-movable").exists());
    assert_eq!(
        fs::read_to_string(destination.join("b-movable")).unwrap(),
        "movable"
    );
}

#[test]
fn preflight_off_stop_leaves_later_siblings_untouched() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("a-collision"), "source").unwrap();
    fs::write(source.join("b-later"), "later").unwrap();
    fs::write(destination.join("a-collision"), "destination").unwrap();
    let mut opts = options(&source, &destination);
    opts.preflight = Preflight::Off;

    let report = undir::run(&opts).unwrap_err();

    assert_eq!(report.issues().len(), 1);
    assert!(source.join("a-collision").exists());
    assert!(source.join("b-later").exists());
    assert!(!destination.join("b-later").exists());
}

#[test]
fn mkdir_requires_a_new_final_component() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), "contents").unwrap();
    let mut opts = options(&source, &destination);
    opts.create = CreateMode::One;

    undir::run(&opts).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("file")).unwrap(),
        "contents"
    );

    let second_source = temporary.path().join("second-source");
    fs::create_dir(&second_source).unwrap();
    let mut second = options(second_source, &destination);
    second.create = CreateMode::One;
    assert!(
        undir::run(&second).unwrap_err().issues()[0]
            .message()
            .contains("already exists")
    );
}

#[test]
fn mkdirs_creates_parents_and_accepts_an_existing_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("one/two/destination");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("first"), "first").unwrap();
    let mut opts = options(&source, &destination);
    opts.create = CreateMode::Parents;
    opts.keep = true;

    undir::run(&opts).unwrap();
    fs::write(source.join("second"), "second").unwrap();
    undir::run(&opts).unwrap();

    assert_eq!(
        fs::read_to_string(destination.join("first")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(destination.join("second")).unwrap(),
        "second"
    );
}

#[test]
fn rejects_lexical_and_resolved_destination_children() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("child")).unwrap();

    let lexical = options(&source, source.join("child"));
    assert!(
        undir::run(&lexical).unwrap_err().issues()[0]
            .message()
            .contains("child of srcdir")
    );

    #[cfg(unix)]
    {
        let alias = temporary.path().join("alias");
        std::os::unix::fs::symlink(source.join("child"), &alias).unwrap();
        let resolved = options(&source, &alias);
        assert!(
            undir::run(&resolved).unwrap_err().issues()[0]
                .message()
                .contains("resolve to a child")
        );
    }
}

#[test]
fn overwrite_removes_only_the_source_name_for_same_hard_link() {
    let (_temporary, source, destination) = roots();
    fs::write(source.join("same"), "contents").unwrap();
    fs::hard_link(source.join("same"), destination.join("same")).unwrap();
    let mut opts = options(&source, &destination);
    opts.overwrite = true;

    undir::run(&opts).unwrap();

    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(destination.join("same")).unwrap(),
        "contents"
    );
}

#[cfg(unix)]
#[test]
fn nested_symlinks_are_moved_without_following_them() {
    let (_temporary, source, destination) = roots();
    let external = destination.parent().unwrap().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("untouched"), "contents").unwrap();
    std::os::unix::fs::symlink(&external, source.join("link")).unwrap();

    undir::run(&options(&source, &destination)).unwrap();

    assert!(
        fs::symlink_metadata(destination.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(external.join("untouched")).unwrap(),
        "contents"
    );
}

#[cfg(unix)]
#[test]
fn source_root_symlink_is_removed_but_its_empty_target_remains() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("target");
    let source_link = temporary.path().join("source-link");
    let destination = temporary.path().join("destination");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(target.join("file"), "contents").unwrap();
    std::os::unix::fs::symlink(&target, &source_link).unwrap();

    undir::run(&options(&source_link, &destination)).unwrap();

    assert!(target.is_dir());
    assert!(fs::read_dir(&target).unwrap().next().is_none());
    assert!(fs::symlink_metadata(&source_link).is_err());
    assert_eq!(
        fs::read_to_string(destination.join("file")).unwrap(),
        "contents"
    );
}

#[cfg(unix)]
#[test]
fn source_root_symlink_inside_its_target_is_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("target");
    let source_link = target.join("source-link");
    let destination = temporary.path().join("destination");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&destination).unwrap();
    std::os::unix::fs::symlink(&target, &source_link).unwrap();

    let report = undir::run(&options(&source_link, &destination)).unwrap_err();

    assert!(
        report.issues()[0]
            .message()
            .contains("must not be located inside its target")
    );
    assert!(fs::symlink_metadata(&source_link).is_ok());
}

#[test]
fn existing_destination_is_required_without_creation_flags() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    fs::create_dir(&source).unwrap();
    let report = undir::run(&options(&source, temporary.path().join("missing"))).unwrap_err();
    assert!(report.issues()[0].message().contains("does not exist"));
}

#[cfg(unix)]
#[test]
fn permission_preflight_does_not_mutate() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        return;
    }

    let (_temporary, source, destination) = roots();
    fs::write(source.join("file"), "contents").unwrap();
    let original_permissions = fs::metadata(&destination).unwrap().permissions();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o500)).unwrap();

    let result = undir::run(&options(&source, &destination));
    fs::set_permissions(&destination, original_permissions).unwrap();

    let report = result.unwrap_err();
    assert_eq!(report.issues()[0].phase(), Phase::Preflight);
    assert!(source.join("file").exists());
    assert!(!destination.join("file").exists());
}

#[test]
fn strict_move_succeeds_when_the_host_filesystem_supports_it() {
    let (_temporary, source, destination) = roots();
    if !renamore::rename_exclusive_is_atomic(&destination).unwrap_or(false) {
        return;
    }
    fs::write(source.join("file"), "contents").unwrap();
    let mut opts = options(&source, &destination);
    opts.strict = true;

    undir::run(&opts).unwrap();

    assert_eq!(
        fs::read_to_string(destination.join("file")).unwrap(),
        "contents"
    );
}
