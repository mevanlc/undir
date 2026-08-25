use std::io;
use std::path::Path;

#[cfg(unix)]
mod imp {
    #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
    use std::ffi::CStr;
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    #[cfg(target_os = "linux")]
    use std::path::PathBuf;

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    pub enum MountId {
        #[cfg(target_os = "linux")]
        Linux(u64),
        #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
        MountPoint(Vec<u8>),
        Device(u64),
    }

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
    }

    #[cfg(target_os = "linux")]
    fn unescape_mount_path(value: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(value.len());
        let mut index = 0;
        while index < value.len() {
            if value[index] == b'\\' && index + 3 < value.len() {
                let digits = &value[index + 1..index + 4];
                if digits.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
                    let decoded =
                        (digits[0] - b'0') * 64 + (digits[1] - b'0') * 8 + (digits[2] - b'0');
                    result.push(decoded);
                    index += 4;
                    continue;
                }
            }
            result.push(value[index]);
            index += 1;
        }
        result
    }

    #[cfg(target_os = "linux")]
    fn linux_mount_id(path: &Path) -> io::Result<Option<u64>> {
        use std::os::unix::ffi::OsStringExt;

        let canonical = std::fs::canonicalize(path)?;
        let mountinfo = std::fs::read("/proc/self/mountinfo")?;
        let mut best = None::<(usize, u64)>;

        for line in mountinfo.split(|byte| *byte == b'\n') {
            let fields = line.split(|byte| *byte == b' ').take(6).collect::<Vec<_>>();
            if fields.len() < 5 {
                continue;
            }
            let Some(id) = std::str::from_utf8(fields[0])
                .ok()
                .and_then(|id| id.parse().ok())
            else {
                continue;
            };
            let mount_path =
                PathBuf::from(std::ffi::OsString::from_vec(unescape_mount_path(fields[4])));
            if canonical.starts_with(&mount_path) {
                let depth = mount_path.components().count();
                if best.is_none_or(|(best_depth, _)| depth > best_depth) {
                    best = Some((depth, id));
                }
            }
        }

        Ok(best.map(|(_, id)| id))
    }

    #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
    fn mount_point(path: &Path) -> io::Result<Vec<u8>> {
        let path = c_path(path)?;
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        let result = unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        Ok(unsafe { CStr::from_ptr(stat.f_mntonname.as_ptr()) }
            .to_bytes()
            .to_vec())
    }

    pub fn mount_id(path: &Path) -> io::Result<MountId> {
        #[cfg(target_os = "linux")]
        if let Ok(Some(id)) = linux_mount_id(path) {
            return Ok(MountId::Linux(id));
        }

        #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
        return mount_point(path).map(MountId::MountPoint);

        #[allow(unreachable_code)]
        Ok(MountId::Device(std::fs::metadata(path)?.dev()))
    }

    fn check_access(path: &Path, mode: libc::c_int) -> io::Result<()> {
        let path = c_path(path)?;
        if unsafe { libc::access(path.as_ptr(), mode) } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn check_directory_read(path: &Path) -> io::Result<()> {
        check_access(path, libc::R_OK | libc::X_OK)
    }

    pub fn check_destination_parent(path: &Path, _entry_is_dir: bool) -> io::Result<()> {
        check_access(path, libc::W_OK | libc::X_OK)
    }

    pub fn check_remove(path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        check_access(parent, libc::W_OK | libc::X_OK)?;

        let parent_metadata = std::fs::metadata(parent)?;
        if parent_metadata.mode() & u32::from(libc::S_ISVTX) != 0 {
            let entry_metadata = std::fs::symlink_metadata(path)?;
            let effective_uid = unsafe { libc::geteuid() };
            if effective_uid != 0
                && effective_uid != parent_metadata.uid()
                && effective_uid != entry_metadata.uid()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "sticky directory does not permit removing this entry",
                ));
            }
        }
        Ok(())
    }

    pub fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    pub fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
        let left = std::fs::symlink_metadata(left)?;
        let right = std::fs::symlink_metadata(right)?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
        FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TRAVERSE, GetFileInformationByHandle,
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, MOVEFILE_REPLACE_EXISTING,
        MoveFileExW, OPEN_EXISTING,
    };

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    pub struct MountId(Vec<u16>);

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn open_handle(path: &Path, access: u32, open_reparse_point: bool) -> io::Result<HANDLE> {
        let path = wide(path);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS
                    | if open_reparse_point {
                        FILE_FLAG_OPEN_REPARSE_POINT
                    } else {
                        0
                    },
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    fn open_for_access(path: &Path, access: u32, open_reparse_point: bool) -> io::Result<()> {
        let handle = open_handle(path, access, open_reparse_point)?;
        if unsafe { CloseHandle(handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn file_identity(path: &Path, open_reparse_point: bool) -> io::Result<(u32, u64)> {
        let handle = open_handle(path, FILE_READ_ATTRIBUTES, open_reparse_point)?;
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        let result = unsafe { GetFileInformationByHandle(handle, information.as_mut_ptr()) };
        let operation_error = if result == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        let close_error = if unsafe { CloseHandle(handle) } == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        if let Some(error) = operation_error.or(close_error) {
            return Err(error);
        }
        let information = unsafe { information.assume_init() };
        let index =
            u64::from(information.nFileIndexHigh) << 32 | u64::from(information.nFileIndexLow);
        Ok((information.dwVolumeSerialNumber, index))
    }

    pub fn mount_id(path: &Path) -> io::Result<MountId> {
        const BUFFER_LENGTH: usize = 32_768;
        let path = wide(path);
        let mut volume_path = vec![0_u16; BUFFER_LENGTH];
        if unsafe {
            GetVolumePathNameW(
                path.as_ptr(),
                volume_path.as_mut_ptr(),
                u32::try_from(volume_path.len()).expect("volume path buffer length fits in u32"),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        truncate_wide(&mut volume_path);

        let mut volume_name = vec![0_u16; BUFFER_LENGTH];
        let volume_path_nul = volume_path
            .iter()
            .copied()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let identity = if unsafe {
            GetVolumeNameForVolumeMountPointW(
                volume_path_nul.as_ptr(),
                volume_name.as_mut_ptr(),
                u32::try_from(volume_name.len()).expect("volume name buffer length fits in u32"),
            )
        } != 0
        {
            truncate_wide(&mut volume_name);
            volume_name
        } else {
            volume_path
        };
        Ok(MountId(fold_ascii_case(identity)))
    }

    fn truncate_wide(value: &mut Vec<u16>) {
        if let Some(end) = value.iter().position(|unit| *unit == 0) {
            value.truncate(end);
        }
    }

    fn fold_ascii_case(mut value: Vec<u16>) -> Vec<u16> {
        for unit in &mut value {
            if let Ok(byte) = u8::try_from(*unit)
                && byte.is_ascii_uppercase()
            {
                *unit = u16::from(byte.to_ascii_lowercase());
            }
        }
        value
    }

    pub fn check_directory_read(path: &Path) -> io::Result<()> {
        open_for_access(path, FILE_LIST_DIRECTORY | FILE_TRAVERSE, false)
    }

    pub fn check_destination_parent(path: &Path, entry_is_dir: bool) -> io::Result<()> {
        open_for_access(
            path,
            FILE_TRAVERSE
                | if entry_is_dir {
                    FILE_ADD_SUBDIRECTORY
                } else {
                    FILE_ADD_FILE
                },
            false,
        )
    }

    pub fn check_remove(path: &Path) -> io::Result<()> {
        if open_for_access(path, DELETE, true).is_ok() {
            return Ok(());
        }
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        open_for_access(parent, FILE_DELETE_CHILD | FILE_TRAVERSE, false)
    }

    pub fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
        let from = wide(from);
        let to = wide(to);
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
        let left_mount = mount_id(left.parent().unwrap_or(left))?;
        let right_mount = mount_id(right.parent().unwrap_or(right))?;
        let (_, left_index) = file_identity(left, true)?;
        let (_, right_index) = file_identity(right, true)?;
        Ok(left_mount == right_mount && left_index == right_index)
    }
}

pub(crate) use imp::MountId;

pub(crate) fn mount_id(path: &Path) -> io::Result<MountId> {
    imp::mount_id(path)
}

pub(crate) fn check_directory_read(path: &Path) -> io::Result<()> {
    imp::check_directory_read(path)
}

pub(crate) fn check_destination_parent(path: &Path, entry_is_dir: bool) -> io::Result<()> {
    imp::check_destination_parent(path, entry_is_dir)
}

pub(crate) fn check_remove(path: &Path) -> io::Result<()> {
    imp::check_remove(path)
}

pub(crate) fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    imp::rename_replace(from, to)
}

pub(crate) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    imp::same_file(left, right)
}
