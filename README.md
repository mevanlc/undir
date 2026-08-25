# undir

`undir` moves every child of one directory into another directory.

```console
undir [OPTIONS] <SRCDIR> [DSTDIR]
undir --completion <SHELL>
```

`DSTDIR` defaults to the current directory. Successful operations are silent.

## behavior

When a destination name does not exist, `undir` renames the source entry into
place. Directories are moved whole whenever possible. When matching source and
destination directories both exist, `--merge` recursively moves their children
and removes each emptied source directory.

Hidden entries, symlinks, regular files, and other nondirectory filesystem
entries are included. Symlinks encountered below either root are moved as links
and are never followed.

The source directory is removed after all children have been moved. Use `--keep`
to preserve it. A source directory that remains nonempty is always preserved.
When `SRCDIR` itself is a symlink to a directory, `undir` moves the target's
children and removes only the command-line symlink; the empty target directory
remains. A source symlink located inside its own target tree is rejected because
moving the target's children would also move the command-line source entry.

`SRCDIR` and `DSTDIR` may themselves be symlinks to directories. Both their
written paths and resolved targets are checked: the destination must not equal
the source or be inside it.

## collisions

The two opt-in collision behaviors apply recursively:

- directory onto directory requires `--merge`;
- nondirectory onto nondirectory requires `--overwrite`;
- directory onto nondirectory is always an error; and
- nondirectory onto directory is always an error.

`--overwrite` replaces the destination entry. If the two names are hard links to
the same object, only the source name is removed.

Moves across filesystems are not supported. Source mount points that cannot be
moved or removed are also rejected.

## destination creation

Without a creation flag, `DSTDIR` must already resolve to a directory.

- `--mkdir` requires `DSTDIR` not to exist and its immediate parent to be an
  existing directory. Any existing filesystem entry at `DSTDIR` is an error.
- `--mkdirs` creates missing parents like `mkdir --parents`. An existing
  directory is accepted; an existing nondirectory is an error.

The flags are mutually exclusive. Destination creation happens only after
preflight succeeds.

## preflight

Preflight models the actual renames, recursive merges, replacements, directory
removals, and destination creation before changing the filesystem. It checks
collision authorization, entry types, mount boundaries, and the process's
current read, traversal, insertion, replacement, and removal permissions.

- `--preflight fast` is the default and stops after the first detected issue.
- `--preflight full` reports every issue it can discover in deterministic path
  order.
- `--preflight off` skips permission and missing-`--merge`/`--overwrite` checks.
  Root containment, cross-filesystem moves, unoverrideable type collisions, and
  `--strict` capability requirements are still checked before mutation.

Preflight is a read-only snapshot. Permissions, mounts, sharing locks, and paths
can change afterward, so every filesystem mutation still handles its own errors.

## rename safety

By default, a move to a missing destination uses the platform's atomic
no-clobber rename when available and otherwise uses a check-then-rename fallback.
The fallback preserves broad filesystem and BSD support but cannot eliminate a
concurrent destination-creation race.

`--strict` disables that fallback. Every planned move to a missing destination
must support an atomic no-clobber rename or the operation fails before mutation.
Authorized replacements under `--overwrite` use the platform's normal
replacement rename and are outside `--strict`'s scope.

The no-clobber implementation is pinned to the maintained
[Renamore fork](https://github.com/mevanlc/renamore), which currently provides
native operations for supported Linux filesystems, Apple platforms, and Windows.
Strict missing-destination moves are unavailable on FreeBSD, NetBSD, and OpenBSD;
non-strict moves use the portable fallback there.

`--strict` protects each rename's destination. It does not make traversal of a
filesystem that is being adversarially modified transactional.

## error handling

`--error stop` is the default and stops at the first mutation-time error.
`--error continue` continues with independent siblings and reports every
failure. It does not change preflight behavior, and `undir` never rolls back
successful earlier moves.

Operational and preflight failures are written to standard error and exit with
status 1. Command-line parsing failures use Clap's status 2.

## shell completion

`--completion SHELL` writes a completion script to standard output and exits.
`SHELL` is one of `bash`, `elvish`, `fish`, `pwsh`, or `zsh`.

## platforms and building

`undir` targets:

- Linux, including Debian, Red Hat, Arch, Alpine, and NixOS families;
- macOS 10.15 Catalina and newer;
- Windows 10 and 11; and
- FreeBSD, NetBSD, and OpenBSD.

Rust 1.98 or newer is required.

```console
cargo build --release
cargo nextest run
```

Linux, macOS, and Windows are covered by hosted CI, with an additional Linux
musl build check. BSD support is kept behind portable `cfg` implementations and
requires manual validation on BSD hosts.
