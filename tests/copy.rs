use std::fs;
use std::path::Path;

#[cfg(unix)]
use undir::Phase;
use undir::{Action, CreateMode, OnError, Options, Preflight};

fn options(source: &Path, destination: &Path) -> Options {
    Options {
        srcdir: source.into(),
        dstdir: destination.into(),
        merge: false,
        overwrite: false,
        on_error: OnError::Stop,
        keep: true,
        keep_empty: false,
        preflight: Preflight::Fast,
        create: CreateMode::Existing,
        strict: false,
    }
}

#[test]
fn keep_copies_the_tree_and_previews_without_source_cleanup() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    let destination = root.join("parents/destination");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::write(source.join(".hidden"), "hidden").unwrap();
    fs::write(source.join("nested/file"), "contents").unwrap();
    let mut opts = options(&source, &destination);
    opts.create = CreateMode::Parents;

    assert_eq!(
        undir::dry_run(&opts).unwrap(),
        vec![
            Action::CreateDirectory {
                path: destination.clone(),
                parents: true
            },
            Action::Copy {
                source: source.join(".hidden"),
                destination: destination.join(".hidden"),
                overwrite: false
            },
            Action::CreateDirectory {
                path: destination.join("nested"),
                parents: false
            },
            Action::CreateDirectory {
                path: destination.join("nested/empty"),
                parents: false
            },
            Action::Copy {
                source: source.join("nested/file"),
                destination: destination.join("nested/file"),
                overwrite: false
            },
        ]
    );
    assert!(!destination.exists());

    undir::run(&opts).unwrap();
    for tree in [&source, &destination] {
        assert_eq!(fs::read_to_string(tree.join(".hidden")).unwrap(), "hidden");
        assert_eq!(
            fs::read_to_string(tree.join("nested/file")).unwrap(),
            "contents"
        );
        assert!(tree.join("nested/empty").is_dir());
        assert_eq!(fs::read_dir(tree).unwrap().count(), 2);
    }
    fs::write(destination.join("nested/file"), "changed").unwrap();
    assert_eq!(
        fs::read_to_string(source.join("nested/file")).unwrap(),
        "contents"
    );
}

#[test]
fn keep_requires_collision_flags_and_preserves_merged_sources() {
    for preflight in [Preflight::Fast, Preflight::Full, Preflight::Off] {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs::create_dir_all(source.join("shared/nested")).unwrap();
        fs::create_dir_all(destination.join("shared/nested")).unwrap();
        fs::write(source.join("shared/nested/file"), "source").unwrap();
        fs::write(destination.join("shared/nested/file"), "destination").unwrap();
        fs::write(destination.join("shared/extra"), "extra").unwrap();
        let mut opts = options(&source, &destination);
        opts.preflight = preflight;

        assert!(undir::dry_run(&opts).is_err());
        assert!(undir::run(&opts).is_err());
        opts.merge = true;
        assert!(undir::dry_run(&opts).is_err());
        assert!(undir::run(&opts).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("shared/nested/file")).unwrap(),
            "destination"
        );
        opts.overwrite = true;
        assert!(matches!(
            undir::dry_run(&opts).unwrap().as_slice(),
            [Action::Copy {
                overwrite: true,
                ..
            }]
        ));
        undir::run(&opts).unwrap();
        for tree in [&source, &destination] {
            assert_eq!(
                fs::read_to_string(tree.join("shared/nested/file")).unwrap(),
                "source"
            );
        }
        assert_eq!(
            fs::read_to_string(destination.join("shared/extra")).unwrap(),
            "extra"
        );
    }
}

#[test]
fn keep_overwrite_breaks_destination_hard_links_without_changing_source() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("file"), "contents").unwrap();
    fs::hard_link(source.join("file"), destination.join("file")).unwrap();
    let mut opts = options(&source, &destination);
    opts.overwrite = true;

    undir::run(&opts).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("file")).unwrap(),
        "contents"
    );
    fs::write(destination.join("file"), "changed").unwrap();
    assert_eq!(fs::read_to_string(source.join("file")).unwrap(), "contents");
}

#[cfg(unix)]
#[test]
fn keep_preserves_root_and_nested_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let alias = temporary.path().join("alias");
    let destination = temporary.path().join("destination");
    let external = temporary.path().join("external");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(&external, "external").unwrap();
    fs::write(source.join("file"), "source").unwrap();
    symlink(&source, &alias).unwrap();
    symlink("missing", source.join("dangling")).unwrap();
    symlink(".", source.join("loop")).unwrap();
    symlink(&external, destination.join("file")).unwrap();
    let mut opts = options(&alias, &destination);
    opts.overwrite = true;

    undir::run(&opts).unwrap();
    assert_eq!(fs::read_link(&alias).unwrap(), source);
    for tree in [&source, &destination] {
        assert_eq!(
            fs::read_link(tree.join("dangling")).unwrap(),
            Path::new("missing")
        );
        assert_eq!(fs::read_link(tree.join("loop")).unwrap(), Path::new("."));
        assert_eq!(fs::read_to_string(tree.join("file")).unwrap(), "source");
    }
    assert!(
        !fs::symlink_metadata(destination.join("file"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(external).unwrap(), "external");
}

#[cfg(unix)]
#[test]
fn keep_checks_special_files_inside_new_directories_before_mutation() {
    use std::os::unix::net::UnixListener;

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("a-file"), "contents").unwrap();
    let socket_path = source.join("nested/socket");
    let _socket = UnixListener::bind(&socket_path).unwrap();
    let mut opts = options(&source, &destination);
    opts.create = CreateMode::One;
    for preflight in [Preflight::Fast, Preflight::Full, Preflight::Off] {
        opts.preflight = preflight;
        let report = undir::run(&opts).unwrap_err();
        assert_eq!(report.issues()[0].phase(), Phase::Preflight);
        assert!(
            report.issues()[0]
                .message()
                .contains("regular files, directories, and symlinks only")
        );
        assert!(!destination.exists());
    }
}

#[cfg(unix)]
#[test]
fn keep_copies_read_only_sources_and_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("nested/file"), "contents").unwrap();
    fs::set_permissions(
        source.join("nested/file"),
        fs::Permissions::from_mode(0o440),
    )
    .unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o550)).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o550)).unwrap();

    let result = undir::run(&options(&source, &destination));
    // Restore directory permissions for test cleanup even if copying failed.
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    let copied_mode =
        fs::metadata(destination.join("nested")).map(|m| m.permissions().mode() & 0o777);
    if destination.join("nested").exists() {
        fs::set_permissions(
            destination.join("nested"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    result.unwrap();
    assert_eq!(copied_mode.unwrap(), 0o550);
    assert_eq!(
        fs::metadata(destination.join("nested/file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o440
    );
    assert_eq!(
        fs::read_to_string(source.join("nested/file")).unwrap(),
        "contents"
    );
    assert_eq!(
        fs::read_to_string(destination.join("nested/file")).unwrap(),
        "contents"
    );
}

#[cfg(unix)]
#[test]
fn failed_copy_preserves_existing_destination_and_obeys_error_policy() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    for policy in [OnError::Stop, OnError::Continue] {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("a-unreadable"), "source").unwrap();
        fs::write(destination.join("a-unreadable"), "destination").unwrap();
        fs::write(source.join("b-later"), "later").unwrap();
        fs::set_permissions(
            source.join("a-unreadable"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let mut opts = options(&source, &destination);
        opts.overwrite = true;
        opts.on_error = policy;
        assert_eq!(
            undir::run(&opts).unwrap_err().issues()[0].phase(),
            Phase::Preflight
        );
        assert!(!destination.join("b-later").exists());
        opts.preflight = Preflight::Off;
        let result = undir::run(&opts);
        fs::set_permissions(
            source.join("a-unreadable"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(result.unwrap_err().issues()[0].phase(), Phase::Mutation);
        assert_eq!(
            destination.join("b-later").exists(),
            policy == OnError::Continue
        );
        assert_eq!(
            fs::read_to_string(source.join("a-unreadable")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(destination.join("a-unreadable")).unwrap(),
            "destination"
        );
        assert_eq!(fs::read_to_string(source.join("b-later")).unwrap(), "later");
        assert_eq!(
            fs::read_dir(&destination).unwrap().count(),
            if policy == OnError::Continue { 2 } else { 1 }
        );
    }
}

#[test]
fn keep_honors_strict_rename_support() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("file"), "contents").unwrap();
    let mut opts = options(&source, &destination);
    opts.strict = true;
    let supported = renamore::rename_exclusive_is_atomic(&destination).unwrap();
    assert_eq!(undir::dry_run(&opts).is_ok(), supported);
    assert_eq!(undir::run(&opts).is_ok(), supported);
    assert_eq!(destination.join("file").exists(), supported);
    assert_eq!(fs::read_to_string(source.join("file")).unwrap(), "contents");
}

#[cfg(target_os = "linux")]
#[test]
fn keep_can_copy_across_filesystems() {
    use std::os::unix::fs::MetadataExt;

    let source = tempfile::tempdir().unwrap();
    let Ok(destination) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    if fs::metadata(source.path()).unwrap().dev() == fs::metadata(destination.path()).unwrap().dev()
    {
        return;
    }
    fs::write(source.path().join("file"), "contents").unwrap();
    let mut opts = options(source.path(), destination.path());
    opts.keep = false;
    assert!(
        undir::run(&opts).unwrap_err().issues()[0]
            .message()
            .contains("cross filesystems")
    );
    opts.keep = true;
    undir::run(&opts).unwrap();
    for tree in [source.path(), destination.path()] {
        assert_eq!(fs::read_to_string(tree.join("file")).unwrap(), "contents");
    }
}
