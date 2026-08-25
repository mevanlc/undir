use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(crate) fn absolute_lexical(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(normalize(&absolute))
}

fn normalize(path: &Path) -> PathBuf {
    let mut prefix = None;
    let mut has_root = false;
    let mut components = Vec::<OsString>::new();

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => {
                has_root = true;
                components.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop();
            }
            Component::Normal(value) => components.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(value) = prefix {
        normalized.push(value);
    }
    if has_root {
        normalized.push(Component::RootDir.as_os_str());
    }
    normalized.extend(components);
    normalized
}

pub(crate) fn resolve_existing_prefix(path: &Path) -> io::Result<PathBuf> {
    let absolute = absolute_lexical(path)?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::<OsString>::new();

    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "path has no existing prefix")
                })?;
                missing.push(name.to_owned());
                existing = existing.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "path has no existing prefix")
                })?;
            }
            Err(error) => return Err(error),
        }
    }

    let mut resolved = std::fs::canonicalize(existing)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(normalize(&resolved))
}

pub(crate) fn nearest_existing(path: &Path) -> io::Result<PathBuf> {
    let mut candidate = path;
    loop {
        match std::fs::metadata(candidate) {
            Ok(_) => return Ok(candidate.to_path_buf()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                candidate = candidate.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn sorted_children(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut children = std::fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    children.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(children)
}

pub(crate) fn is_same_or_child(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_normalization_removes_dot_segments() {
        let normalized = absolute_lexical(Path::new("one/./two/../three")).unwrap();
        assert!(normalized.ends_with(Path::new("one/three")));
    }
}
