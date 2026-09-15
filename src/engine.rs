use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use crate::pathing::{
    absolute_lexical, is_same_or_child, nearest_existing, resolve_existing_prefix, sorted_children,
};
use crate::platform;
use crate::{Action, CreateMode, Issue, OnError, Options, Phase, Preflight, Report};

struct Prepared {
    source_input: PathBuf,
    source_alias: PathBuf,
    source_root: PathBuf,
    source_is_symlink: bool,
    destination_input: PathBuf,
    destination_root: PathBuf,
    destination_exists: bool,
}

impl Prepared {
    fn new(options: &Options) -> Result<Self, Report> {
        Self::try_new(options).map_err(Report::from_issue)
    }

    fn try_new(options: &Options) -> Result<Self, Issue> {
        if options.keep && options.keep_empty {
            return Err(Issue::setup("--keep cannot be used with --keep-empty"));
        }
        let source_input = absolute_lexical(&options.srcdir)
            .map_err(|error| Issue::setup(format!("cannot resolve srcdir: {error}")))?;
        let source_link_metadata = fs::symlink_metadata(&source_input).map_err(|error| {
            Issue::setup(format!("cannot inspect srcdir {source_input:?}: {error}"))
        })?;
        let source_is_symlink = source_link_metadata.file_type().is_symlink();
        let source_metadata = fs::metadata(&source_input).map_err(|error| {
            Issue::setup(format!("cannot inspect srcdir {source_input:?}: {error}"))
        })?;
        if !source_metadata.is_dir() {
            return Err(Issue::setup(format!(
                "srcdir {source_input:?} is not a directory"
            )));
        }
        let source_root = fs::canonicalize(&source_input).map_err(|error| {
            Issue::setup(format!("cannot resolve srcdir {source_input:?}: {error}"))
        })?;
        let source_alias = resolve_final_entry(&source_input)
            .map_err(|error| Issue::setup(format!("cannot resolve srcdir parent: {error}")))?;
        if source_is_symlink && !options.keep {
            let source_alias_parent = source_alias.parent().ok_or_else(|| {
                Issue::setup(format!(
                    "srcdir symlink {source_input:?} has no parent directory"
                ))
            })?;
            if path_is_within(source_alias_parent, &source_root).map_err(|error| {
                Issue::setup(format!(
                    "cannot compare srcdir symlink location with its target: {error}"
                ))
            })? {
                return Err(Issue::setup(
                    "srcdir symlink must not be located inside its target directory",
                ));
            }
        }

        let destination_input = absolute_lexical(&options.dstdir)
            .map_err(|error| Issue::setup(format!("cannot resolve dstdir: {error}")))?;
        let destination_state = fs::symlink_metadata(&destination_input);
        let destination_exists = match destination_state {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(Issue::setup(format!(
                    "cannot inspect dstdir {destination_input:?}: {error}"
                )));
            }
        };

        match options.create {
            CreateMode::Existing => {
                if !destination_exists {
                    return Err(Issue::setup(format!(
                        "dstdir {destination_input:?} does not exist (use --mkdir or --mkdirs to create it)"
                    )));
                }
                require_directory(&destination_input, "dstdir")?;
            }
            CreateMode::One => {
                if destination_exists {
                    return Err(Issue::setup(format!(
                        "dstdir {destination_input:?} already exists with --mkdir"
                    )));
                }
                let parent = destination_input.parent().ok_or_else(|| {
                    Issue::setup(format!(
                        "dstdir {destination_input:?} has no parent directory"
                    ))
                })?;
                require_directory(parent, "dstdir parent")?;
            }
            CreateMode::Parents => {
                if destination_exists {
                    require_directory(&destination_input, "dstdir")?;
                } else {
                    let ancestor = nearest_existing(&destination_input).map_err(|error| {
                        Issue::setup(format!(
                            "cannot find an existing dstdir ancestor for {destination_input:?}: {error}"
                        ))
                    })?;
                    require_directory(&ancestor, "dstdir ancestor")?;
                }
            }
        }

        let destination_root = resolve_existing_prefix(&destination_input).map_err(|error| {
            Issue::setup(format!(
                "cannot resolve dstdir {destination_input:?}: {error}"
            ))
        })?;

        validate_roots(
            &source_input,
            &source_root,
            &destination_input,
            &destination_root,
        )?;

        Ok(Self {
            source_input,
            source_alias,
            source_root,
            source_is_symlink,
            destination_input,
            destination_root,
            destination_exists,
        })
    }

    fn create_destination(&mut self, options: &Options) -> Result<(), Issue> {
        if self.destination_exists {
            return Ok(());
        }

        let result = match options.create {
            CreateMode::Existing => unreachable!("an existing destination was already required"),
            CreateMode::One => fs::create_dir(&self.destination_input),
            CreateMode::Parents => fs::create_dir_all(&self.destination_input),
        };
        result.map_err(|error| {
            Issue::at(
                Phase::Mutation,
                None,
                Some(&self.destination_input),
                format!("cannot create dstdir: {error}"),
            )
        })?;

        let actual = fs::canonicalize(&self.destination_input).map_err(|error| {
            Issue::at(
                Phase::Mutation,
                None,
                Some(&self.destination_input),
                format!("cannot resolve newly created dstdir: {error}"),
            )
        })?;
        if actual != self.destination_root {
            return Err(Issue::at(
                Phase::Mutation,
                None,
                Some(&self.destination_input),
                format!(
                    "dstdir resolved to {actual:?} after creation instead of preflight target {:?}",
                    self.destination_root
                ),
            ));
        }
        validate_roots(
            &self.source_input,
            &self.source_root,
            &self.destination_input,
            &actual,
        )?;
        self.destination_root = actual;
        self.destination_exists = true;
        Ok(())
    }
}

fn resolve_final_entry(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no final component")
    })?;
    Ok(fs::canonicalize(parent)?.join(name))
}

fn require_directory(path: &Path, label: &str) -> Result<(), Issue> {
    let metadata = fs::metadata(path)
        .map_err(|error| Issue::setup(format!("cannot inspect {label} {path:?}: {error}")))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(Issue::setup(format!("{label} {path:?} is not a directory")))
    }
}

fn validate_roots(
    source_input: &Path,
    source_root: &Path,
    destination_input: &Path,
    destination_root: &Path,
) -> Result<(), Issue> {
    if is_same_or_child(destination_input, source_input) {
        let relationship = if destination_input == source_input {
            "must not equal srcdir"
        } else {
            "must not be a child of srcdir"
        };
        return Err(Issue::setup(format!("dstdir {relationship}")));
    }
    if is_same_or_child(destination_root, source_root) {
        let relationship = if destination_root == source_root {
            "must not resolve to srcdir"
        } else {
            "must not resolve to a child of srcdir"
        };
        return Err(Issue::setup(format!("dstdir {relationship}")));
    }
    if path_is_within(destination_root, source_root).map_err(|error| {
        Issue::setup(format!(
            "cannot inspect resolved dstdir ancestry {destination_root:?}: {error}"
        ))
    })? {
        let relationship = if fs::symlink_metadata(destination_root).is_ok()
            && platform::same_file(destination_root, source_root).unwrap_or(false)
        {
            "must not resolve to srcdir"
        } else {
            "must not resolve to a child of srcdir"
        };
        return Err(Issue::setup(format!("dstdir {relationship}")));
    }
    Ok(())
}

fn path_is_within(path: &Path, root: &Path) -> io::Result<bool> {
    let mut ancestor = nearest_existing(path)?;
    loop {
        if platform::same_file(&ancestor, root)? {
            return Ok(true);
        }
        let Some(parent) = ancestor.parent() else {
            return Ok(false);
        };
        ancestor = parent.to_path_buf();
    }
}

pub fn run(options: &Options) -> Result<(), Report> {
    let mut prepared = Prepared::new(options)?;
    Checker::new(options, &prepared, false).run()?;

    prepared
        .create_destination(options)
        .map_err(Report::from_issue)?;

    let mut mutator = Mutator::new(options, &prepared);
    mutator.run();
    if mutator.issues.is_empty() {
        Ok(())
    } else {
        Err(Report::from_issues(mutator.issues))
    }
}

/// Check and list operations in execution order without changing the filesystem.
/// Collision authorization is checked even when permission preflight is off.
pub fn dry_run(options: &Options) -> Result<Vec<Action>, Report> {
    let prepared = Prepared::new(options)?;
    Checker::new(options, &prepared, true).run()
}

struct Checker<'a> {
    options: &'a Options,
    prepared: &'a Prepared,
    issues: Vec<Issue>,
    mounts: HashMap<PathBuf, Result<platform::MountId, String>>,
    strict_support: HashMap<platform::MountId, bool>,
    actions: Option<Vec<Action>>,
}

impl<'a> Checker<'a> {
    fn new(options: &'a Options, prepared: &'a Prepared, dry_run: bool) -> Self {
        Self {
            options,
            prepared,
            issues: Vec::new(),
            mounts: HashMap::new(),
            strict_support: HashMap::new(),
            actions: dry_run.then(Vec::new),
        }
    }

    fn run(mut self) -> Result<Vec<Action>, Report> {
        if self.check_permissions() {
            self.permission_check(
                &self.prepared.source_root,
                None,
                "cannot enumerate srcdir",
                platform::check_directory_read,
            );
            if !self.should_stop() && !self.prepared.destination_exists {
                match nearest_existing(&self.prepared.destination_root) {
                    Ok(parent) => self.permission_check(
                        &parent,
                        Some(&self.prepared.destination_root),
                        "cannot create dstdir",
                        |path| platform::check_destination_parent(path, true),
                    ),
                    Err(error) => self.push(Issue::at(
                        Phase::Preflight,
                        None,
                        Some(&self.prepared.destination_root),
                        format!("cannot inspect dstdir ancestor permissions: {error}"),
                    )),
                }
            }
        }

        if !self.should_stop() {
            self.check_root_cleanup();
        }
        if !self.prepared.destination_exists
            && let Some(actions) = &mut self.actions
        {
            actions.push(Action::CreateDirectory {
                path: self.prepared.destination_input.clone(),
                parents: self.options.create == CreateMode::Parents,
            });
        }
        if !self.should_stop() {
            match sorted_children(&self.prepared.source_root) {
                Ok(children) => {
                    for source in children {
                        let destination = self.prepared.destination_root.join(
                            source
                                .file_name()
                                .expect("a directory child always has a file name"),
                        );
                        self.check_entry(&source, &destination);
                        if self.should_stop() {
                            break;
                        }
                    }
                }
                Err(error) => self.push(Issue::at(
                    Phase::Preflight,
                    Some(&self.prepared.source_root),
                    None,
                    format!("cannot enumerate srcdir: {error}"),
                )),
            }
        }
        if !self.issues.is_empty() {
            return Err(Report::from_issues(self.issues));
        }
        if !self.options.keep
            && !self.options.keep_empty
            && let Some(actions) = &mut self.actions
        {
            actions.push(if self.prepared.source_is_symlink {
                Action::RemoveFile {
                    path: self.prepared.source_input.clone(),
                }
            } else {
                Action::RemoveDirectory {
                    path: self.prepared.source_root.clone(),
                }
            });
        }
        Ok(self.actions.unwrap_or_default())
    }

    fn check_root_cleanup(&mut self) {
        if self.options.keep || self.options.keep_empty {
            return;
        }
        let cleanup_path = if self.prepared.source_is_symlink {
            &self.prepared.source_alias
        } else {
            &self.prepared.source_root
        };
        if self.check_permissions() {
            self.permission_check(
                cleanup_path,
                None,
                "cannot remove srcdir",
                platform::check_remove,
            );
        }
        if !self.should_stop() && !self.prepared.source_is_symlink {
            self.check_mount_point(&self.prepared.source_root);
        }
    }

    fn check_entry(&mut self, source: &Path, destination: &Path) {
        match targets_source_alias(destination, &self.prepared.source_alias) {
            Ok(true) => {
                self.push(Issue::at(
                    Phase::Preflight,
                    Some(source),
                    Some(destination),
                    "destination would overwrite the srcdir entry",
                ));
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot compare destination with the srcdir entry",
                    error,
                ));
                return;
            }
        }

        let source_metadata = match fs::symlink_metadata(source) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot inspect source",
                    error,
                ));
                return;
            }
        };
        let destination_metadata = match optional_metadata(destination) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot inspect destination",
                    error,
                ));
                return;
            }
        };

        match destination_metadata {
            None => self.check_transfer(source, destination, &source_metadata, true),
            Some(destination_metadata) => {
                let source_is_dir = is_directory(&source_metadata);
                let destination_is_dir = is_directory(&destination_metadata);
                match (source_is_dir, destination_is_dir) {
                    (true, true) => self.check_merge(source, destination),
                    (false, false) => {
                        if self.check_authorization() && !self.options.overwrite {
                            self.push(Issue::at(
                                Phase::Preflight,
                                Some(source),
                                Some(destination),
                                "destination nondirectory exists; use --overwrite",
                            ));
                        }
                        if !self.should_stop() {
                            self.check_transfer(source, destination, &source_metadata, false);
                        }
                        if self.check_permissions() && !self.should_stop() {
                            self.permission_check(
                                destination,
                                Some(destination),
                                "cannot replace destination",
                                platform::check_remove,
                            );
                        }
                    }
                    (true, false) => self.push(Issue::at(
                        Phase::Preflight,
                        Some(source),
                        Some(destination),
                        "source is a directory but destination is a nondirectory",
                    )),
                    (false, true) => self.push(Issue::at(
                        Phase::Preflight,
                        Some(source),
                        Some(destination),
                        "source is a nondirectory but destination is a directory",
                    )),
                }
            }
        }
    }

    fn check_merge(&mut self, source: &Path, destination: &Path) {
        if self.check_authorization() && !self.options.merge {
            self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                Some(destination),
                "destination directory exists; use --merge",
            ));
        }
        if self.should_stop() {
            return;
        }
        if !self.options.keep {
            self.check_mount_point(source);
        }
        if self.check_permissions() && !self.should_stop() {
            self.permission_check(
                source,
                Some(destination),
                "cannot enumerate source directory",
                platform::check_directory_read,
            );
            if !self.options.keep {
                self.permission_check(
                    source,
                    Some(destination),
                    "cannot remove merged source directory",
                    platform::check_remove,
                );
            }
        }
        if self.should_stop() {
            return;
        }

        self.check_children(source, destination);
        if !self.options.keep
            && let Some(actions) = &mut self.actions
        {
            actions.push(Action::RemoveDirectory {
                path: source.to_path_buf(),
            });
        }
    }

    fn check_children(&mut self, source: &Path, destination: &Path) {
        match sorted_children(source) {
            Ok(children) => {
                for child in children {
                    let child_destination = destination.join(
                        child
                            .file_name()
                            .expect("a directory child always has a file name"),
                    );
                    self.check_entry(&child, &child_destination);
                    if self.should_stop() {
                        break;
                    }
                }
            }
            Err(error) => self.push(io_issue(
                source,
                destination,
                "cannot enumerate source directory",
                error,
            )),
        }
    }

    fn check_transfer(
        &mut self,
        source: &Path,
        destination: &Path,
        source_metadata: &Metadata,
        destination_missing: bool,
    ) {
        if self.options.keep {
            self.check_copy(source, destination, source_metadata, destination_missing);
        } else {
            self.check_rename(source, destination, source_metadata, destination_missing);
        }
    }

    fn check_copy(
        &mut self,
        source: &Path,
        destination: &Path,
        metadata: &Metadata,
        destination_missing: bool,
    ) {
        if let Err(error) = require_copyable(metadata) {
            self.push(io_issue(source, destination, "cannot copy entry", error));
            return;
        }
        let source_is_dir = is_directory(metadata);
        let parent = match nearest_existing(destination.parent().expect("a child has a parent")) {
            Ok(parent) => parent,
            Err(error) => {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot inspect destination parent",
                    error,
                ));
                return;
            }
        };
        if self.options.strict && destination_missing && !source_is_dir {
            self.check_strict_support(source, destination, &parent);
        }
        if self.check_permissions() && !self.should_stop() {
            if source_is_dir {
                self.permission_check(
                    source,
                    Some(destination),
                    "cannot enumerate source directory",
                    platform::check_directory_read,
                );
            } else if metadata.is_file() {
                self.permission_check(
                    source,
                    Some(destination),
                    "cannot read source file",
                    platform::check_file_read,
                );
            } else if let Err(error) = fs::read_link(source) {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot read source symlink",
                    error,
                ));
            }
            // Nondirectory copies are staged in a temporary destination directory.
            self.permission_check(
                &parent,
                Some(destination),
                "cannot create destination directory",
                |path| platform::check_destination_parent(path, true),
            );
            if !source_is_dir {
                self.permission_check(
                    &parent,
                    Some(destination),
                    "cannot add destination entry",
                    |path| platform::check_destination_parent(path, false),
                );
            }
        }
        if self.should_stop() {
            return;
        }
        if let Some(actions) = &mut self.actions {
            actions.push(if source_is_dir {
                Action::CreateDirectory {
                    path: destination.to_path_buf(),
                    parents: false,
                }
            } else {
                Action::Copy {
                    source: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                    overwrite: !destination_missing,
                }
            });
        }
        if source_is_dir {
            self.check_children(source, destination);
        }
    }

    fn check_rename(
        &mut self,
        source: &Path,
        destination: &Path,
        source_metadata: &Metadata,
        destination_missing: bool,
    ) {
        let source_is_dir = is_directory(source_metadata);
        self.check_mount_point_if_directory(source, source_is_dir);
        if self.should_stop() {
            return;
        }

        let source_parent = source.parent().expect("a directory child has a parent");
        let destination_parent = destination
            .parent()
            .expect("a destination child has a parent");
        let destination_mount_path = match nearest_existing(destination_parent) {
            Ok(path) => path,
            Err(error) => {
                self.push(io_issue(
                    source,
                    destination,
                    "cannot identify destination filesystem",
                    error,
                ));
                return;
            }
        };
        let source_mount = self.cached_mount_id(source_parent);
        let destination_mount = self.cached_mount_id(&destination_mount_path);
        match (&source_mount, &destination_mount) {
            (Ok(source_mount), Ok(destination_mount)) => {
                if source_mount != destination_mount {
                    self.push(Issue::at(
                        Phase::Preflight,
                        Some(source),
                        Some(destination),
                        "move would cross filesystems",
                    ));
                }
            }
            (Err(error), _) => self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                Some(destination),
                format!("cannot identify source filesystem: {error}"),
            )),
            (_, Err(error)) => self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                Some(destination),
                format!("cannot identify destination filesystem: {error}"),
            )),
        }
        if self.should_stop() {
            return;
        }

        if self.options.strict && destination_missing {
            self.check_strict_support(source, destination, &destination_mount_path);
        }
        if self.check_permissions() && !self.should_stop() {
            self.permission_check(
                source,
                Some(destination),
                "cannot remove source entry",
                platform::check_remove,
            );
            self.permission_check(
                &destination_mount_path,
                Some(destination),
                "cannot add destination entry",
                |path| platform::check_destination_parent(path, source_is_dir),
            );
        }
        if self.issues.is_empty() && self.actions.is_some() {
            let action = if destination_missing {
                Action::Move {
                    source: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                }
            } else {
                match platform::same_file(source, destination) {
                    Ok(true) => Action::RemoveFile {
                        path: source.to_path_buf(),
                    },
                    Ok(false) => Action::Replace {
                        source: source.to_path_buf(),
                        destination: destination.to_path_buf(),
                    },
                    Err(error) => {
                        self.push(io_issue(
                            source,
                            destination,
                            "cannot compare entries",
                            error,
                        ));
                        return;
                    }
                }
            };
            if let Some(actions) = &mut self.actions {
                actions.push(action);
            }
        }
    }

    fn check_strict_support(
        &mut self,
        source: &Path,
        destination: &Path,
        destination_mount_path: &Path,
    ) {
        let destination_mount = self.cached_mount_id(destination_mount_path);
        let cached = destination_mount
            .as_ref()
            .ok()
            .and_then(|mount| self.strict_support.get(mount).copied());
        let support = cached
            .map(Ok)
            .unwrap_or_else(|| renamore::rename_exclusive_is_atomic(destination_mount_path));
        if let (Ok(mount), Ok(supported)) = (&destination_mount, &support) {
            self.strict_support.insert(mount.clone(), *supported);
        }
        match support {
            Ok(true) => {}
            Ok(false) => self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                Some(destination),
                "--strict requires atomic no-clobber rename support on the destination filesystem",
            )),
            Err(error) => self.push(io_issue(
                source,
                destination,
                "cannot determine atomic no-clobber rename support",
                error,
            )),
        }
    }

    fn check_mount_point_if_directory(&mut self, source: &Path, source_is_dir: bool) {
        if source_is_dir {
            self.check_mount_point(source);
        }
    }

    fn check_mount_point(&mut self, source: &Path) {
        let Some(parent) = source.parent() else {
            self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                None,
                "cannot remove a filesystem root",
            ));
            return;
        };
        let source_mount = self.cached_mount_id(source);
        let parent_mount = self.cached_mount_id(parent);
        match (source_mount, parent_mount) {
            (Ok(source_mount), Ok(parent_mount)) => {
                if source_mount != parent_mount {
                    self.push(Issue::at(
                        Phase::Preflight,
                        Some(source),
                        None,
                        "source directory is a mount point and cannot be moved or removed",
                    ));
                }
            }
            (Err(error), _) | (_, Err(error)) => self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                None,
                format!("cannot determine whether source is a mount point: {error}"),
            )),
        }
    }

    fn cached_mount_id(&mut self, path: &Path) -> Result<platform::MountId, String> {
        if let Some(result) = self.mounts.get(path) {
            return result.clone();
        }
        let result = platform::mount_id(path).map_err(|error| error.to_string());
        self.mounts.insert(path.to_path_buf(), result.clone());
        result
    }

    fn permission_check<F>(
        &mut self,
        source: &Path,
        destination: Option<&Path>,
        description: &str,
        check: F,
    ) where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        if let Err(error) = check(source) {
            self.push(Issue::at(
                Phase::Preflight,
                Some(source),
                destination,
                format!("{description}: {error}"),
            ));
        }
    }

    fn push(&mut self, issue: Issue) {
        self.issues.push(issue);
    }

    fn should_stop(&self) -> bool {
        self.options.preflight == Preflight::Fast && !self.issues.is_empty()
    }

    fn check_permissions(&self) -> bool {
        self.options.preflight != Preflight::Off
    }

    fn check_authorization(&self) -> bool {
        self.options.preflight != Preflight::Off || self.actions.is_some()
    }
}

fn io_issue(source: &Path, destination: &Path, description: &str, error: io::Error) -> Issue {
    Issue::at(
        Phase::Preflight,
        Some(source),
        Some(destination),
        format!("{description}: {error}"),
    )
}

fn optional_metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_directory(metadata: &Metadata) -> bool {
    metadata.file_type().is_dir()
}

fn require_copyable(metadata: &Metadata) -> io::Result<()> {
    if metadata.is_file() || metadata.is_dir() || metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "--keep supports regular files, directories, and symlinks only",
        ))
    }
}

fn targets_source_alias(destination: &Path, source_alias: &Path) -> io::Result<bool> {
    if destination == source_alias {
        return Ok(true);
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => platform::same_file(destination, source_alias),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

struct Mutator<'a> {
    options: &'a Options,
    prepared: &'a Prepared,
    issues: Vec<Issue>,
    stopped: bool,
}

impl<'a> Mutator<'a> {
    fn new(options: &'a Options, prepared: &'a Prepared) -> Self {
        Self {
            options,
            prepared,
            issues: Vec::new(),
            stopped: false,
        }
    }

    fn run(&mut self) {
        let children = match sorted_children(&self.prepared.source_root) {
            Ok(children) => children,
            Err(error) => {
                self.record(Issue::at(
                    Phase::Mutation,
                    Some(&self.prepared.source_root),
                    None,
                    format!("cannot enumerate srcdir: {error}"),
                ));
                return;
            }
        };

        for source in children {
            let destination = self.prepared.destination_root.join(
                source
                    .file_name()
                    .expect("a directory child always has a file name"),
            );
            self.transfer_entry(&source, &destination, true);
            if self.stopped {
                return;
            }
        }

        if !self.options.keep && !self.options.keep_empty {
            self.remove_source_root();
        }
    }

    fn transfer_entry(&mut self, source: &Path, destination: &Path, allow_retry: bool) {
        match targets_source_alias(destination, &self.prepared.source_alias) {
            Ok(true) => {
                self.record(Issue::at(
                    Phase::Mutation,
                    Some(source),
                    Some(destination),
                    "destination would overwrite the srcdir entry",
                ));
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.record(mutation_io_issue(
                    source,
                    destination,
                    "cannot compare destination with the srcdir entry",
                    error,
                ));
                return;
            }
        }
        let source_metadata = match fs::symlink_metadata(source) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.record(mutation_io_issue(
                    source,
                    destination,
                    "cannot inspect source",
                    error,
                ));
                return;
            }
        };
        let destination_metadata = match optional_metadata(destination) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.record(mutation_io_issue(
                    source,
                    destination,
                    "cannot inspect destination",
                    error,
                ));
                return;
            }
        };

        match destination_metadata {
            None if self.options.keep && is_directory(&source_metadata) => {
                match fs::create_dir(destination) {
                    Ok(()) => {
                        self.transfer_children(source, destination);
                        // Apply permissions after copying children so a read-only source
                        // directory does not prevent populating its new copy.
                        if let Err(error) =
                            fs::set_permissions(destination, source_metadata.permissions())
                        {
                            self.record(mutation_io_issue(
                                source,
                                destination,
                                "cannot set copied directory permissions",
                                error,
                            ));
                        }
                    }
                    Err(error) if allow_retry && error.kind() == io::ErrorKind::AlreadyExists => {
                        self.transfer_entry(source, destination, false);
                    }
                    Err(error) => self.record(mutation_io_issue(
                        source,
                        destination,
                        "cannot create copied directory",
                        error,
                    )),
                }
            }
            None => match if self.options.keep {
                self.copy_nondirectory(source, destination, false)
            } else {
                self.rename_missing(source, destination)
            } {
                Ok(()) => {}
                Err(error)
                    if allow_retry
                        && matches!(
                            error.kind(),
                            io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                        ) =>
                {
                    self.transfer_entry(source, destination, false);
                }
                Err(error) => self.record(mutation_io_issue(
                    source,
                    destination,
                    if self.options.keep {
                        "cannot copy entry"
                    } else {
                        "cannot move entry"
                    },
                    error,
                )),
            },
            Some(destination_metadata) => {
                match (
                    is_directory(&source_metadata),
                    is_directory(&destination_metadata),
                ) {
                    (true, true) => self.merge_directory(source, destination),
                    (false, false) => self.replace_nondirectory(source, destination),
                    (true, false) => self.record(Issue::at(
                        Phase::Mutation,
                        Some(source),
                        Some(destination),
                        "source is a directory but destination is a nondirectory",
                    )),
                    (false, true) => self.record(Issue::at(
                        Phase::Mutation,
                        Some(source),
                        Some(destination),
                        "source is a nondirectory but destination is a directory",
                    )),
                }
            }
        }
    }

    fn rename_missing(&self, source: &Path, destination: &Path) -> io::Result<()> {
        if self.options.strict {
            renamore::rename_exclusive(source, destination)
        } else {
            renamore::rename_exclusive_fallback(source, destination).map(|_| ())
        }
    }

    fn copy_nondirectory(
        &self,
        source: &Path,
        destination: &Path,
        overwrite: bool,
    ) -> io::Result<()> {
        let metadata = fs::symlink_metadata(source)?;
        require_copyable(&metadata)?;
        if is_directory(&metadata) {
            return Err(io::Error::other("source is no longer a nondirectory"));
        }
        // Publish a completed copy with the same rename guarantees as a move.
        // Never open the destination for writing: it may alias the source or
        // be a symlink to an unrelated file.
        let staging = tempfile::Builder::new().prefix(".undir-").tempdir_in(
            destination
                .parent()
                .expect("a destination child has a parent"),
        )?;
        let staged = staging.path().join("entry");
        if metadata.file_type().is_symlink() {
            platform::copy_symlink(source, &staged)?;
        } else {
            fs::copy(source, &staged)?;
        }
        if overwrite {
            platform::rename_replace(&staged, destination)?;
        } else {
            self.rename_missing(&staged, destination)?;
        }
        staging.close()
    }

    fn merge_directory(&mut self, source: &Path, destination: &Path) {
        if !self.options.merge {
            self.record(Issue::at(
                Phase::Mutation,
                Some(source),
                Some(destination),
                "destination directory exists; use --merge",
            ));
            return;
        }

        let issue_count = self.issues.len();
        if !self.transfer_children(source, destination) || self.options.keep {
            return;
        }

        match directory_is_empty(source) {
            Ok(true) => {
                if let Err(error) = fs::remove_dir(source) {
                    self.record(mutation_io_issue(
                        source,
                        destination,
                        "cannot remove emptied source directory",
                        error,
                    ));
                }
            }
            Ok(false) if self.issues.len() == issue_count => self.record(Issue::at(
                Phase::Mutation,
                Some(source),
                Some(destination),
                "source directory is not empty after merge",
            )),
            Ok(false) => {}
            Err(error) => self.record(mutation_io_issue(
                source,
                destination,
                "cannot inspect merged source directory",
                error,
            )),
        }
    }

    fn transfer_children(&mut self, source: &Path, destination: &Path) -> bool {
        let children = match sorted_children(source) {
            Ok(children) => children,
            Err(error) => {
                self.record(mutation_io_issue(
                    source,
                    destination,
                    "cannot enumerate source directory",
                    error,
                ));
                return false;
            }
        };
        for child in children {
            let child_destination = destination.join(
                child
                    .file_name()
                    .expect("a directory child always has a file name"),
            );
            self.transfer_entry(&child, &child_destination, true);
            if self.stopped {
                return false;
            }
        }
        true
    }

    fn replace_nondirectory(&mut self, source: &Path, destination: &Path) {
        if !self.options.overwrite {
            self.record(Issue::at(
                Phase::Mutation,
                Some(source),
                Some(destination),
                "destination nondirectory exists; use --overwrite",
            ));
            return;
        }

        let result = if self.options.keep {
            self.copy_nondirectory(source, destination, true)
        } else {
            match platform::same_file(source, destination) {
                Ok(true) => fs::remove_file(source),
                Ok(false) => platform::rename_replace(source, destination),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            self.record(mutation_io_issue(
                source,
                destination,
                "cannot replace destination",
                error,
            ));
        }
    }

    fn remove_source_root(&mut self) {
        match directory_is_empty(&self.prepared.source_root) {
            Ok(false) if self.issues.is_empty() => self.record(Issue::at(
                Phase::Mutation,
                Some(&self.prepared.source_root),
                None,
                "srcdir is not empty after moving its children",
            )),
            Ok(false) => {}
            Ok(true) => {
                let result = if self.prepared.source_is_symlink {
                    match fs::symlink_metadata(&self.prepared.source_input) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            match fs::canonicalize(&self.prepared.source_input) {
                                Ok(target) if target == self.prepared.source_root => {
                                    fs::remove_file(&self.prepared.source_input)
                                }
                                Ok(_) => Err(io::Error::other(
                                    "srcdir symlink target changed during the operation",
                                )),
                                Err(error) => Err(error),
                            }
                        }
                        Ok(_) => Err(io::Error::other("srcdir is no longer the original symlink")),
                        Err(error) => Err(error),
                    }
                } else {
                    fs::remove_dir(&self.prepared.source_root)
                };
                if let Err(error) = result {
                    self.record(Issue::at(
                        Phase::Mutation,
                        Some(&self.prepared.source_input),
                        None,
                        format!("cannot remove srcdir: {error}"),
                    ));
                }
            }
            Err(error) => self.record(Issue::at(
                Phase::Mutation,
                Some(&self.prepared.source_root),
                None,
                format!("cannot inspect srcdir after moving its children: {error}"),
            )),
        }
    }

    fn record(&mut self, issue: Issue) {
        self.issues.push(issue);
        if self.options.on_error == OnError::Stop {
            self.stopped = true;
        }
    }
}

fn directory_is_empty(path: &Path) -> io::Result<bool> {
    Ok(fs::read_dir(path)?.next().is_none())
}

fn mutation_io_issue(
    source: &Path,
    destination: &Path,
    description: &str,
    error: io::Error,
) -> Issue {
    Issue::at(
        Phase::Mutation,
        Some(source),
        Some(destination),
        format!("{description}: {error}"),
    )
}
