use std::ffi::CString;
use std::fs::{File, Permissions};
use std::io::{self, BufRead, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path};

pub fn effective_uid() -> libc::uid_t {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

/// Opens each component without following symlinks. Shared sticky ancestors
/// are allowed, but the leaf must be controlled by this user.
pub fn owned_directory(path: &Path, create: bool) -> io::Result<File> {
    let absolute = std::path::absolute(path)?;
    let mut components = absolute.components().peekable();
    components.next(); // RootDir
    let mut directory = File::open("/")?;
    let uid = effective_uid();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Local directory path must not contain parent references",
            ));
        };
        let name = CString::new(name.as_bytes())?;
        let parent = directory.as_raw_fd();
        let open = || {
            // SAFETY: parent and name remain valid throughout the call.
            let fd = unsafe {
                libc::openat(
                    parent,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                // SAFETY: openat returned a new owned descriptor.
                Ok(unsafe { File::from_raw_fd(fd) })
            }
        };
        let next = match open() {
            Err(err) if create && err.kind() == io::ErrorKind::NotFound => {
                // SAFETY: parent and name remain valid throughout the call.
                if unsafe { libc::mkdirat(parent, name.as_ptr(), 0o700) } != 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() != io::ErrorKind::AlreadyExists {
                        return Err(err);
                    }
                }
                open()?
            }
            result => result?,
        };
        let metadata = next.metadata()?;
        let leaf = components.peek().is_none();
        let writable_by_others = metadata.mode() & 0o022 != 0;
        let sticky = metadata.mode() & 0o1000 != 0;
        if (metadata.uid() != uid && (leaf || metadata.uid() != 0))
            || (writable_by_others && (leaf || !sticky))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unsafe local directory owner or write permissions",
            ));
        }
        directory = next;
    }
    if absolute == Path::new("/") && uid != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Local directory must belong to the current user",
        ));
    }
    Ok(directory)
}

pub fn create_private_directory(path: &Path) -> io::Result<File> {
    if std::path::absolute(path)? == Path::new("/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The filesystem root cannot be used as a private directory",
        ));
    }
    let directory = owned_directory(path, true)?;
    directory.set_permissions(Permissions::from_mode(0o700))?;
    Ok(directory)
}

pub fn socket_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Includes the line delimiter in the byte limit. The caller can close the
/// transport on overflow without buffering or draining the rest of the line.
pub fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<usize> {
    line.clear();
    let count = reader.take(max_bytes as u64 + 1).read_until(b'\n', line)?;
    if count > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Line exceeds the {max_bytes}-byte limit"),
        ));
    }
    Ok(count)
}
