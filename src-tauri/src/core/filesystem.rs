//! Filesystem seam used by fixture-backed cleanup tests.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub canonical_path: PathBuf,
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
    pub volume_serial: Option<u64>,
    pub file_reference: Option<String>,
}

pub trait FileSystemAdapter: Send + Sync {
    fn metadata(&self, path: &Path) -> io::Result<Metadata>;
    fn list(&self, directory: &Path) -> io::Result<Vec<PathBuf>>;
    fn exists(&self, path: &Path) -> bool;
    fn identity(&self, path: &Path) -> io::Result<FileIdentity>;
    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn restore(&self, backup: &Path, destination: &Path) -> io::Result<u64>;
    /// Delete `path` using the requested disposition.
    ///
    /// Implementations MUST honour the requested mode and MUST NOT silently
    /// fall back from Recycle Bin to permanent deletion.
    fn delete(
        &self,
        path: &Path,
        expected_identity: Option<&crate::core::file_identity::FileIdentity>,
        recycle_bin: bool,
    ) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeFileSystem;

impl FileSystemAdapter for NativeFileSystem {
    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        fs::symlink_metadata(path)
    }

    fn list(&self, directory: &Path) -> io::Result<Vec<PathBuf>> {
        fs::read_dir(directory)?
            .map(|entry| entry.map(|value| value.path()))
            .collect()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn identity(&self, path: &Path) -> io::Result<FileIdentity> {
        identity(path)
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
        fs::copy(source, destination)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn delete(
        &self,
        path: &Path,
        expected_identity: Option<&crate::core::file_identity::FileIdentity>,
        recycle_bin: bool,
    ) -> io::Result<()> {
        crate::core::race_safe_delete::delete_file(path, path, expected_identity, recycle_bin)
            .map_err(io::Error::other)
    }

    fn restore(&self, backup: &Path, destination: &Path) -> io::Result<u64> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(backup, destination)
    }
}

#[derive(Debug, Clone)]
pub struct FixtureFileSystem {
    root: PathBuf,
}

impl FixtureFileSystem {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        // Validate existing ancestors before creating anything. This prevents
        // create_dir_all from following an already-present reparse point.
        reject_reparse_components(root)?;
        create_dirs_without_reparse_points(root)?;
        reject_reparse_components(root)?;
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "fixture root is not a directory",
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn checked(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalize_path(path)?;
        let candidate = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        reject_reparse_components(&candidate)?;
        let canonical = if has_directory_entry(&candidate)? {
            candidate.canonicalize()?
        } else {
            let parent = candidate
                .parent()
                .ok_or_else(|| invalid_path("path has no parent"))?;
            let canonical_parent = parent.canonicalize()?;
            canonical_parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| invalid_path("path has no filename"))?,
            )
        };
        self.ensure_contained(canonical)
    }

    fn checked_restore_destination(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalize_path(path)?;
        let candidate = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        reject_reparse_components(&candidate)?;
        if has_directory_entry(&candidate)? {
            return self.checked(&candidate);
        }

        let mut ancestor = candidate.as_path();
        let mut missing = Vec::new();
        while !has_directory_entry(ancestor)? {
            missing.push(
                ancestor
                    .file_name()
                    .ok_or_else(|| invalid_path("path has no filename"))?,
            );
            ancestor = ancestor
                .parent()
                .ok_or_else(|| invalid_path("path has no existing ancestor"))?;
        }
        let mut canonical = ancestor.canonicalize()?;
        missing.reverse();
        for component in missing {
            canonical.push(component);
        }
        self.ensure_contained(canonical)
    }

    fn ensure_contained(&self, path: PathBuf) -> io::Result<PathBuf> {
        if path == self.root || path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path is outside fixture root",
            ))
        }
    }
}

impl FileSystemAdapter for FixtureFileSystem {
    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        fs::symlink_metadata(self.checked(path)?)
    }

    fn list(&self, directory: &Path) -> io::Result<Vec<PathBuf>> {
        fs::read_dir(self.checked(directory)?)?
            .map(|entry| entry.map(|value| value.path()))
            .collect()
    }

    fn exists(&self, path: &Path) -> bool {
        self.checked(path)
            .map(|value| value.exists())
            .unwrap_or(false)
    }

    fn identity(&self, path: &Path) -> io::Result<FileIdentity> {
        identity(&self.checked(path)?)
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
        let source = self.checked(source)?;
        let destination = self.checked(destination)?;
        secure_copy(&source, &destination)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(self.checked(path)?)
    }

    fn delete(
        &self,
        path: &Path,
        _expected_identity: Option<&crate::core::file_identity::FileIdentity>,
        recycle_bin: bool,
    ) -> io::Result<()> {
        let checked = self.checked(path)?;
        if !recycle_bin {
            return fs::remove_file(checked);
        }
        // Simulate a recoverable Recycle Bin disposition inside the fixture:
        // move the file to a recycle area instead of permanently removing it,
        // so a RecycleBin plan can never masquerade as a permanent delete.
        let recycle_root = self.root.join(".recycle");
        create_dirs_without_reparse_points(&recycle_root)?;
        let file_name = checked
            .file_name()
            .ok_or_else(|| invalid_path("path has no filename"))?;
        let mut destination = recycle_root.join(file_name);
        let mut counter = 1u32;
        while destination.exists() {
            destination = recycle_root.join(format!("{}.{counter}", file_name.to_string_lossy()));
            counter += 1;
        }
        fs::rename(checked, destination)
    }

    fn restore(&self, backup: &Path, destination: &Path) -> io::Result<u64> {
        let backup = self.checked(backup)?;
        let destination = self.checked_restore_destination(destination)?;
        if let Some(parent) = destination.parent() {
            create_dirs_without_reparse_points(parent)?;
        }
        let destination = self.checked_restore_destination(destination.as_path())?;
        secure_copy(&backup, &destination)
    }
}

// Recheck immediately before the path-based copy. Windows has no stable std API
// for copying through an already-open directory handle, so this closes the
// validation window as much as the adapter can while refusing all reparse points.
// These std path-based operations cannot eliminate an attacker-controlled
// TOCTOU race; FixtureFileSystem is a test seam, not a security boundary.
fn secure_copy(source: &Path, destination: &Path) -> io::Result<u64> {
    reject_reparse_components(source)?;
    reject_reparse_components(destination)?;
    fs::copy(source, destination)
}

fn create_dirs_without_reparse_points(path: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    while !has_directory_entry(&current)? {
        let component = current
            .file_name()
            .ok_or_else(|| invalid_path("path has no filename"))?
            .to_os_string();
        let parent = current
            .parent()
            .ok_or_else(|| invalid_path("path has no existing ancestor"))?
            .to_path_buf();
        missing.push(component);
        current = parent;
    }
    for component in missing.into_iter().rev() {
        let next = current.join(component);
        match fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        reject_reparse_components(&next)?;
        current = next;
    }
    Ok(())
}

fn reject_reparse_components(path: &Path) -> io::Result<()> {
    let mut current = path;
    loop {
        match fs::symlink_metadata(current) {
            Ok(_) => {
                #[cfg(windows)]
                if windows_reparse_point(current)? {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "reparse points are not allowed in fixture paths",
                    ));
                }
                #[cfg(unix)]
                if fs::symlink_metadata(current)?.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "symbolic links are not allowed in fixture paths",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        current = match current.parent() {
            Some(parent) if parent != current => parent,
            _ => break,
        };
    }
    Ok(())
}

#[cfg(windows)]
fn windows_reparse_point(path: &Path) -> io::Result<bool> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attributes == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
}

fn has_directory_entry(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn normalize_path(path: &Path) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(invalid_path(
                    "path must not contain parent directory components",
                ));
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        normalized.push(".");
    }
    Ok(normalized)
}

fn identity(path: &Path) -> io::Result<FileIdentity> {
    let canonical_path = path.canonicalize()?;
    let metadata = fs::metadata(&canonical_path)?;
    let snapshot = crate::core::file_identity::snapshot_for_path(&canonical_path).ok();
    #[cfg(unix)]
    let platform_identity = {
        use std::os::unix::fs::MetadataExt;
        (Some(metadata.dev()), Some(metadata.ino().to_string()))
    };
    #[cfg(not(unix))]
    let platform_identity = (None, None);
    let volume_serial = snapshot
        .as_ref()
        .and_then(|value| value.identity.volume_serial)
        .or(platform_identity.0);
    let file_reference = snapshot
        .and_then(|value| value.identity.file_reference)
        .or(platform_identity.1);
    Ok(FileIdentity {
        canonical_path,
        len: metadata.len(),
        modified: metadata.modified().ok(),
        volume_serial,
        file_reference,
    })
}

fn invalid_path(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_adapter_contains_copy_delete_restore_and_rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        fs::create_dir_all(root.join("backups")).unwrap();
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, b"outside canary").unwrap();
        let source = root.join("source.txt");
        let backup = root.join("backups/source.bak");
        fs::write(&source, b"fixture bytes").unwrap();
        let adapter = FixtureFileSystem::new(&root).unwrap();

        assert!(adapter.exists(&source));
        assert_eq!(adapter.identity(&source).unwrap().len, 13);
        assert_eq!(adapter.copy(&source, &backup).unwrap(), 13);
        adapter.remove_file(&source).unwrap();
        assert!(!adapter.exists(&source));
        let restored = root.join("nested/deeper/restored.txt");
        adapter.restore(&backup, &restored).unwrap();
        assert_eq!(fs::read(&restored).unwrap(), b"fixture bytes");
        let outside_restore = temp.path().join("outside-restore.txt");
        fs::write(&outside_restore, b"outside restore canary").unwrap();
        let traversal_destination = root.join("new/../../outside-restore.txt");
        assert!(adapter.restore(&backup, &traversal_destination).is_err());
        assert_eq!(
            fs::read(&outside_restore).unwrap(),
            b"outside restore canary"
        );
        assert!(adapter.copy(&outside, &root.join("escaped")).is_err());
        assert!(adapter
            .remove_file(&root.join("..\u{5c}outside.txt"))
            .is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"outside canary");
    }

    #[test]
    fn fixture_rejects_dangling_destination_links_for_copy_and_restore() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        fs::create_dir_all(root.join("backups")).unwrap();
        let adapter = FixtureFileSystem::new(&root).unwrap();
        let source = root.join("source.txt");
        let backup = root.join("backups/source.bak");
        fs::write(&source, b"fixture bytes").unwrap();
        fs::copy(&source, &backup).unwrap();
        let canary = temp.path().join("outside-canary.txt");
        fs::write(&canary, b"outside canary").unwrap();
        let missing_target = temp.path().join("missing-target.txt");
        let copy_destination = root.join("dangling-copy.txt");
        let restore_destination = root.join("dangling-restore.txt");

        for destination in [&copy_destination, &restore_destination] {
            if let Err(error) = create_dangling_link(&missing_target, destination) {
                if is_link_capability_error(&error) {
                    eprintln!("skipping dangling-link capability test: {error}");
                    return;
                }
                panic!("failed to create dangling link: {error}");
            }
        }

        assert!(adapter.copy(&source, &copy_destination).is_err());
        assert!(adapter.restore(&backup, &restore_destination).is_err());
        assert_eq!(fs::read(&canary).unwrap(), b"outside canary");
    }

    #[test]
    fn fixture_rejects_intermediate_links_for_copy_and_restore() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        let adapter = FixtureFileSystem::new(&root).unwrap();
        let source = root.join("source.txt");
        let backup = root.join("source.bak");
        fs::write(&source, b"fixture bytes").unwrap();
        fs::copy(&source, &backup).unwrap();

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let canary = outside.join("canary.txt");
        fs::write(&canary, b"outside canary").unwrap();
        let redirect = root.join("redirect");
        if let Err(error) = create_directory_link(&outside, &redirect) {
            if is_link_capability_error(&error) {
                eprintln!("skipping intermediate-link capability test: {error}");
                return;
            }
            panic!("failed to create intermediate link: {error}");
        }

        assert!(adapter.copy(&source, &redirect.join("copy.txt")).is_err());
        assert!(adapter
            .restore(&backup, &redirect.join("restore.txt"))
            .is_err());
        assert_eq!(fs::read(&canary).unwrap(), b"outside canary");
    }

    #[test]
    fn fixture_constructor_rejects_existing_reparse_ancestor_before_creation() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let redirect = temp.path().join("redirect");
        if let Err(error) = create_directory_link(&outside, &redirect) {
            if is_link_capability_error(&error) {
                eprintln!("skipping constructor reparse capability test: {error}");
                return;
            }
            panic!("failed to create constructor test link: {error}");
        }

        let requested_root = redirect.join("new-fixture");
        assert!(FixtureFileSystem::new(&requested_root).is_err());
        assert!(!outside.join("new-fixture").exists());
    }

    #[cfg(unix)]
    fn create_dangling_link(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_dangling_link(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(unix)]
    fn create_directory_link(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_directory_link(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    /// Symlink creation without Developer Mode/elevation fails with Win32
    /// `ERROR_PRIVILEGE_NOT_HELD` (1314), which `io::ErrorKind` maps to
    /// `Uncategorized`. Treat it as an environment limitation, not a failure.
    fn is_link_capability_error(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
        ) || error.raw_os_error() == Some(1314)
    }

    /// Junctions are reparse points that any user can create, so the core
    /// reparse-safety invariants stay covered even without symlink privilege.
    #[cfg(windows)]
    fn create_directory_junction(target: &Path, link: &Path) -> io::Result<()> {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("mklink /J did not create the junction"))
        }
    }

    #[cfg(windows)]
    #[test]
    fn fixture_rejects_intermediate_junctions_for_copy_and_restore() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        let adapter = FixtureFileSystem::new(&root).unwrap();
        let source = root.join("source.txt");
        let backup = root.join("source.bak");
        fs::write(&source, b"fixture bytes").unwrap();
        fs::copy(&source, &backup).unwrap();

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let canary = outside.join("canary.txt");
        fs::write(&canary, b"outside canary").unwrap();
        let redirect = root.join("redirect");
        create_directory_junction(&outside, &redirect)
            .expect("mklink /J should succeed without admin");

        assert!(adapter.copy(&source, &redirect.join("copy.txt")).is_err());
        assert!(adapter
            .restore(&backup, &redirect.join("restore.txt"))
            .is_err());
        assert_eq!(fs::read(&canary).unwrap(), b"outside canary");
    }

    #[cfg(windows)]
    #[test]
    fn fixture_rejects_junction_destinations_for_copy_and_restore() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        fs::create_dir_all(root.join("backups")).unwrap();
        let adapter = FixtureFileSystem::new(&root).unwrap();
        let source = root.join("source.txt");
        let backup = root.join("backups/source.bak");
        fs::write(&source, b"fixture bytes").unwrap();
        fs::copy(&source, &backup).unwrap();

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let canary = outside.join("canary.txt");
        fs::write(&canary, b"outside canary").unwrap();
        let destination = root.join("junction-destination");
        create_directory_junction(&outside, &destination)
            .expect("mklink /J should succeed without admin");

        assert!(adapter.copy(&source, &destination).is_err());
        assert!(adapter.restore(&backup, &destination).is_err());
        assert_eq!(fs::read(&canary).unwrap(), b"outside canary");
    }

    #[cfg(windows)]
    #[test]
    fn fixture_constructor_rejects_existing_junction_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let redirect = temp.path().join("redirect");
        create_directory_junction(&outside, &redirect)
            .expect("mklink /J should succeed without admin");

        let requested_root = redirect.join("new-fixture");
        assert!(FixtureFileSystem::new(&requested_root).is_err());
        assert!(!outside.join("new-fixture").exists());
    }

    #[test]
    fn fixture_accepts_in_root_absolute_and_rooted_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.txt");
        fs::write(&source, b"bytes").unwrap();
        let adapter = FixtureFileSystem::new(&root).unwrap();

        assert_eq!(
            adapter.identity(&source).unwrap().canonical_path,
            source.canonicalize().unwrap(),
        );
        assert_eq!(adapter.identity(Path::new("source.txt")).unwrap().len, 5);
        assert_eq!(adapter.identity(&root.join("source.txt")).unwrap().len, 5);
    }

    #[test]
    fn fixture_listing_is_rooted_and_native_adapter_remains_available() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        let adapter = FixtureFileSystem::new(&root).unwrap();
        fs::write(root.join("item"), b"x").unwrap();
        let listed = adapter.list(Path::new(".")).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed.iter().all(|path| path.starts_with(adapter.root())));
        assert!(NativeFileSystem.metadata(&root.join("item")).is_ok());
    }
}
