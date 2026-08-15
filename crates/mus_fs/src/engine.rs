//! The one place that moves data, and the rules it obeys.
//!
//! Every move goes through the same three steps:
//!
//! 1. **Preflight** — the source is still exactly what the plan saw, the
//!    destination does not exist, and neither path is nested in the other.
//! 2. **Transfer** — `renameat2(RENAME_NOREPLACE)` where the kernel supports it,
//!    so an existing destination is impossible rather than merely unlikely.
//!    Across devices: copy into a private temporary directory, verify it, put it
//!    in place, and only then delete the source.
//! 3. **Verify** — the destination's inventory is compared with the source's,
//!    entry by entry. A mismatch is a failure, and the move is rolled back.
//!
//! There is no code path that writes over an existing file or directory.

use std::path::{Path, PathBuf};

use crate::inventory::{inventory, EntryKind, Inventory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyLevel {
    /// Compare the file tree and the sizes.
    Metadata,
    /// Also compare blake3 of every file.
    Hash,
}

impl VerifyLevel {
    pub fn hashes(self) -> bool {
        self == VerifyLevel::Hash
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VerifyLevel::Metadata => "metadata",
            VerifyLevel::Hash => "hash",
        }
    }
}

/// What to do when source and destination live on different filesystems and a
/// rename is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDevice {
    /// Fail the move. Nothing is copied, nothing is deleted.
    Refuse,
    /// Copy, verify by hash, then delete the source.
    CopyVerifyDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMethod {
    /// `renameat2(RENAME_NOREPLACE)` — atomic, cannot clobber.
    RenameNoReplace,
    /// Plain `rename` after checking the destination is free (older kernels, FAT).
    RenameChecked,
    /// Copy to a temporary, verify by hash, swap in, delete the source.
    CopyVerifyDelete,
}

impl MoveMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            MoveMethod::RenameNoReplace => "rename-noreplace",
            MoveMethod::RenameChecked => "rename-checked",
            MoveMethod::CopyVerifyDelete => "copy-verify-delete",
        }
    }
}

/// Why a move was not even attempted. The source is untouched in every case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

/// A move that was attempted and went wrong.
#[derive(Debug, Clone)]
pub struct MoveFailure {
    pub error: String,
    /// `true` when the source directory is known to still hold the data.
    pub source_intact: bool,
}

#[derive(Debug, Clone)]
pub struct MoveOutcome {
    pub method: MoveMethod,
    pub files: usize,
    pub bytes: u64,
    pub verified: VerifyLevel,
    /// Directories created to hold the destination (undo removes them if empty).
    pub created_dirs: Vec<PathBuf>,
}

/// Checks that run before anything is touched.
///
/// `expected` is the source identity recorded when the plan was built; `None`
/// skips the check (undo has no plan behind it).
pub fn preflight(
    from: &Path,
    to: &Path,
    expected: Option<&mus_core::DirStamp>,
    verify: VerifyLevel,
) -> Result<Inventory, Refusal> {
    let meta =
        std::fs::symlink_metadata(from).map_err(|e| Refusal(format!("source {}: {e}", from.display())))?;
    if meta.file_type().is_symlink() {
        return Err(Refusal(format!("source {} is a symlink", from.display())));
    }
    if !meta.is_dir() {
        return Err(Refusal(format!("source {} is not a directory", from.display())));
    }
    if to.starts_with(from) {
        return Err(Refusal(format!("destination {} is inside the source {}", to.display(), from.display())));
    }
    if from == to {
        return Err(Refusal("source and destination are the same path".to_string()));
    }

    // The tree may have changed between planning and applying — a folder that
    // is no longer the folder we judged must not be moved on that judgement.
    if let Some(expected) = expected {
        let current = mus_core::DirStamp::of(from)
            .map_err(|e| Refusal(format!("re-reading source {}: {e}", from.display())))?;
        if !current.matches(expected) {
            return Err(Refusal(format!(
                "{} changed since the plan was built (was {} entries, now {})",
                from.display(),
                expected.entries,
                current.entries
            )));
        }
    }

    match std::fs::symlink_metadata(to) {
        Ok(_) => return Err(Refusal(format!("destination {} already exists", to.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Refusal(format!("checking destination {}: {e}", to.display()))),
    }

    inventory(from, verify.hashes()).map_err(|e| Refusal(format!("reading source {}: {e}", from.display())))
}

/// Move `from` to `to`, having already passed [`preflight`].
///
/// `before` is the inventory preflight captured; it is the yardstick the result
/// is measured against.
pub fn execute_move(
    from: &Path,
    to: &Path,
    before: &Inventory,
    verify: VerifyLevel,
    cross_device: CrossDevice,
) -> Result<MoveOutcome, MoveFailure> {
    let mut created_dirs = Vec::new();
    if let Some(parent) = to.parent() {
        match create_dirs(parent) {
            Ok(created) => created_dirs = created,
            Err(e) => {
                return Err(MoveFailure {
                    error: format!("creating {}: {e}", parent.display()),
                    source_intact: true,
                })
            }
        }
    }

    let method = match rename_noreplace(from, to) {
        Ok(method) => method,
        Err(e) if is_cross_device(&e) => match cross_device {
            CrossDevice::Refuse => {
                return Err(MoveFailure {
                    error: format!(
                        "{} and {} are on different filesystems (use --cross-device copy to copy, verify and delete)",
                        from.display(),
                        to.display()
                    ),
                    source_intact: true,
                })
            }
            CrossDevice::CopyVerifyDelete => return copy_verify_delete(from, to, created_dirs),
        },
        Err(e) => {
            return Err(MoveFailure {
                error: format!("renaming {} -> {}: {e}", from.display(), to.display()),
                source_intact: true,
            })
        }
    };

    // The rename returned success; prove it.
    let after = match inventory(to, verify.hashes()) {
        Ok(inv) => inv,
        Err(e) => {
            return Err(MoveFailure {
                error: format!("reading destination {} after the move: {e}", to.display()),
                source_intact: false,
            })
        }
    };
    let diff = before.diff(&after);
    if !diff.is_empty() {
        // Put it back where it came from before reporting.
        let rolled_back = rename_noreplace(to, from).is_ok();
        return Err(MoveFailure {
            error: format!(
                "verification failed after moving {} -> {}: {}{}",
                from.display(),
                to.display(),
                diff.join("; "),
                if rolled_back { " (rolled back)" } else { " (NOT rolled back)" }
            ),
            source_intact: rolled_back,
        });
    }

    if let Some(parent) = to.parent() {
        let _ = fsync_dir(parent);
    }

    Ok(MoveOutcome {
        method,
        files: after.file_count(),
        bytes: after.total_bytes(),
        verified: verify,
        created_dirs,
    })
}

/// Cross-device transfer. The source is deleted only after the copy has been
/// verified byte for byte and is already in its final place.
fn copy_verify_delete(
    from: &Path,
    to: &Path,
    created_dirs: Vec<PathBuf>,
) -> Result<MoveOutcome, MoveFailure> {
    let fail = |error: String| MoveFailure { error, source_intact: true };

    // Copies are always verified by hash: unlike a rename, every byte was
    // re-written, so sizes alone would not prove anything.
    let source_inv = inventory(from, true).map_err(|e| fail(format!("reading {}: {e}", from.display())))?;
    if source_inv.has_special_files() {
        return Err(fail(format!(
            "{} contains device/fifo/socket entries which cannot be copied faithfully",
            from.display()
        )));
    }

    let parent = to.parent().ok_or_else(|| fail(format!("{} has no parent directory", to.display())))?;
    let temp = reserve_temp_dir(parent)
        .map_err(|e| fail(format!("creating a staging directory in {}: {e}", parent.display())))?;

    let staged = (|| -> std::io::Result<()> {
        copy_tree(from, &temp)?;
        fsync_dir(&temp)
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(fail(format!("copying {} -> {}: {e}", from.display(), temp.display())));
    }

    let staged_inv = match inventory(&temp, true) {
        Ok(inv) => inv,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(fail(format!("reading the staged copy {}: {e}", temp.display())));
        }
    };
    let diff = source_inv.diff(&staged_inv);
    if !diff.is_empty() {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(fail(format!(
            "the copy of {} does not match the source: {}",
            from.display(),
            diff.join("; ")
        )));
    }

    if let Err(e) = rename_noreplace(&temp, to) {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(fail(format!("putting the verified copy in place at {}: {e}", to.display())));
    }
    let _ = fsync_dir(parent);

    // Only now, with a verified copy sitting at its final path, is deleting the
    // original safe. If this fails the data still exists twice — never zero times.
    if let Err(e) = std::fs::remove_dir_all(from) {
        return Err(MoveFailure {
            error: format!(
                "copied and verified into {}, but removing the original {} failed: {e} (the data is now in both places)",
                to.display(),
                from.display()
            ),
            source_intact: true,
        });
    }

    Ok(MoveOutcome {
        method: MoveMethod::CopyVerifyDelete,
        files: staged_inv.file_count(),
        bytes: staged_inv.total_bytes(),
        verified: VerifyLevel::Hash,
        created_dirs,
    })
}

/// Move a single file (what the trash holds most of the time).
///
/// Same guarantees as [`execute_move`], scaled down: the destination must be
/// free, the transfer is checked, and a cross-device copy is verified before the
/// original goes away.
pub fn move_file(
    from: &Path,
    to: &Path,
    verify: VerifyLevel,
    cross_device: CrossDevice,
) -> Result<MoveOutcome, MoveFailure> {
    let fail = |error: String| MoveFailure { error, source_intact: true };

    let meta =
        std::fs::symlink_metadata(from).map_err(|e| fail(format!("source {}: {e}", from.display())))?;
    if meta.is_dir() {
        return Err(fail(format!("{} is a directory", from.display())));
    }
    match std::fs::symlink_metadata(to) {
        Ok(_) => return Err(fail(format!("destination {} already exists", to.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(fail(format!("checking destination {}: {e}", to.display()))),
    }

    let is_link = meta.file_type().is_symlink();
    let len = meta.len();
    let hash = if verify.hashes() && !is_link {
        Some(
            crate::inventory::hash_file(from)
                .map_err(|e| fail(format!("hashing {}: {e}", from.display())))?,
        )
    } else {
        None
    };

    let mut created_dirs = Vec::new();
    if let Some(parent) = to.parent() {
        created_dirs =
            create_dirs(parent).map_err(|e| fail(format!("creating {}: {e}", parent.display())))?;
    }

    let method = match rename_noreplace(from, to) {
        Ok(method) => method,
        Err(e) if is_cross_device(&e) && cross_device == CrossDevice::CopyVerifyDelete => {
            if is_link {
                copy_symlink(from, to)
                    .map_err(|e| fail(format!("copying the symlink {}: {e}", from.display())))?;
            } else {
                std::fs::copy(from, to).map_err(|e| fail(format!("copying {}: {e}", from.display())))?;
                std::fs::File::open(to)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| fail(format!("flushing {}: {e}", to.display())))?;
                let copied = crate::inventory::hash_file(to)
                    .map_err(|e| fail(format!("hashing {}: {e}", to.display())))?;
                let original = crate::inventory::hash_file(from)
                    .map_err(|e| fail(format!("hashing {}: {e}", from.display())))?;
                if copied != original {
                    let _ = std::fs::remove_file(to);
                    return Err(fail(format!("the copy of {} does not match the original", from.display())));
                }
            }
            std::fs::remove_file(from).map_err(|e| MoveFailure {
                error: format!("copied {} but could not remove the original: {e}", from.display()),
                source_intact: true,
            })?;
            MoveMethod::CopyVerifyDelete
        }
        Err(e) => return Err(fail(format!("moving {} -> {}: {e}", from.display(), to.display()))),
    };

    let after = std::fs::symlink_metadata(to).map_err(|e| MoveFailure {
        error: format!("destination {} after the move: {e}", to.display()),
        source_intact: false,
    })?;
    if !is_link && after.len() != len {
        return Err(MoveFailure {
            error: format!("{} is {} bytes at its destination, was {len}", to.display(), after.len()),
            source_intact: false,
        });
    }
    if let Some(hash) = hash {
        let now = crate::inventory::hash_file(to).map_err(|e| MoveFailure {
            error: format!("hashing {}: {e}", to.display()),
            source_intact: false,
        })?;
        if now != hash {
            return Err(MoveFailure {
                error: format!("{} changed contents during the move", to.display()),
                source_intact: false,
            });
        }
    }

    Ok(MoveOutcome { method, files: 1, bytes: len, verified: verify, created_dirs })
}

/// A staging directory nobody else can be using: `create_dir` fails if the name
/// is taken, so the loop ends on a name we created ourselves.
fn reserve_temp_dir(parent: &Path) -> std::io::Result<PathBuf> {
    let pid = std::process::id();
    for n in 0u32..10_000 {
        let candidate = parent.join(format!(".mus_renamer-partial-{pid}-{n}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free staging directory name"))
}

/// Recursive copy into an existing, empty `dst`. Never follows symlinks: a link
/// is recreated as a link.
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = std::fs::symlink_metadata(&from)?;
        let ft = meta.file_type();

        if ft.is_symlink() {
            copy_symlink(&from, &to)?;
        } else if ft.is_dir() {
            std::fs::create_dir(&to)?;
            copy_tree(&from, &to)?;
            fsync_dir(&to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to)?;
            // The copy must survive a crash that happens before the rename.
            std::fs::File::open(&to)?.sync_all()?;
        } else {
            return Err(std::io::Error::other(format!(
                "{} is neither a file, a directory nor a symlink",
                from.display()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    let target = std::fs::read_link(from)?;
    std::os::unix::fs::symlink(target, to)
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other(format!(
        "{} is a symlink; copying symlinks is not supported on this platform",
        from.display()
    )))
}

/// `mkdir -p`, reporting which levels it actually created so undo can retract them.
fn create_dirs(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    if dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    let mut cursor = Some(dir);
    while let Some(path) = cursor {
        if path.is_dir() {
            break;
        }
        missing.push(path.to_path_buf());
        cursor = path.parent();
    }
    missing.reverse();

    let mut created = Vec::new();
    for path in missing {
        match std::fs::create_dir(&path) {
            Ok(()) => created.push(path),
            // Someone else got there first; that is fine, it exists now.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(created)
}

pub fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// `EXDEV` — the only errno that means "you must copy instead".
fn is_cross_device(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        e.raw_os_error() == Some(18)
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

/// Rename that refuses to replace an existing destination.
///
/// On Linux `renameat2(RENAME_NOREPLACE)` makes that a kernel guarantee. Where
/// the syscall or the filesystem does not support the flag (older kernels, some
/// FUSE/FAT drivers) we fall back to "check, then rename", which leaves a window
/// of a few microseconds — narrow, and still followed by full verification.
pub fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<MoveMethod> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{renameat_with, RenameFlags, CWD};
        match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok(MoveMethod::RenameNoReplace),
            Err(rustix::io::Errno::NOSYS)
            | Err(rustix::io::Errno::INVAL)
            | Err(rustix::io::Errno::OPNOTSUPP) => {
                // Fall through to the checked path below.
            }
            Err(e) => return Err(std::io::Error::from_raw_os_error(e.raw_os_error())),
        }
    }

    match std::fs::symlink_metadata(to) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists", to.display()),
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::rename(from, to)?;
    Ok(MoveMethod::RenameChecked)
}

/// Whether a directory holds nothing at all.
pub fn is_empty_dir(dir: &Path) -> std::io::Result<bool> {
    Ok(std::fs::read_dir(dir)?.next().is_none())
}

/// Files inside `dir` that are not directories, for the cleaning commands.
pub fn kind_of(path: &Path) -> std::io::Result<EntryKind> {
    let meta = std::fs::symlink_metadata(path)?;
    let ft = meta.file_type();
    Ok(if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Special
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        dir
    }

    #[test]
    fn rename_noreplace_refuses_an_occupied_destination() {
        let root = dir_with(&[("a/track.mp3", b"x"), ("b/other.mp3", b"y")]);
        let err = rename_noreplace(&root.path().join("a"), &root.path().join("b")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
        // Both sides survived.
        assert!(root.path().join("a/track.mp3").exists());
        assert!(root.path().join("b/other.mp3").exists());
    }

    #[test]
    fn rename_noreplace_refuses_an_empty_destination_directory_too() {
        // Plain rename(2) would happily replace an empty directory.
        let root = dir_with(&[("a/track.mp3", b"x")]);
        std::fs::create_dir(root.path().join("b")).unwrap();
        let err = rename_noreplace(&root.path().join("a"), &root.path().join("b")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
    }

    #[test]
    fn a_move_verifies_and_leaves_nothing_behind() {
        let root = dir_with(&[("a/track.mp3", b"hello"), ("a/cd2/more.mp3", b"world")]);
        let from = root.path().join("a");
        let to = root.path().join("Artist - 1999 (Album)");

        let before = preflight(&from, &to, None, VerifyLevel::Hash).unwrap();
        let outcome = execute_move(&from, &to, &before, VerifyLevel::Hash, CrossDevice::Refuse).unwrap();

        assert_eq!(outcome.files, 2);
        assert_eq!(outcome.bytes, 10);
        assert!(!from.exists());
        assert_eq!(std::fs::read(to.join("cd2/more.mp3")).unwrap(), b"world");
    }

    #[test]
    fn preflight_refuses_when_the_source_changed_since_planning() {
        let root = dir_with(&[("a/track.mp3", b"x")]);
        let from = root.path().join("a");
        let stamp = mus_core::DirStamp::of(&from).unwrap();

        std::fs::write(from.join("added.mp3"), b"y").unwrap();

        let err = preflight(&from, &root.path().join("b"), Some(&stamp), VerifyLevel::Metadata).unwrap_err();
        assert!(err.0.contains("changed since the plan"), "{}", err.0);
    }

    #[test]
    fn preflight_refuses_an_existing_destination() {
        let root = dir_with(&[("a/x.mp3", b"x"), ("b/y.mp3", b"y")]);
        let err = preflight(&root.path().join("a"), &root.path().join("b"), None, VerifyLevel::Metadata)
            .unwrap_err();
        assert!(err.0.contains("already exists"), "{}", err.0);
    }

    #[test]
    fn preflight_refuses_to_move_a_folder_into_itself() {
        let root = dir_with(&[("a/x.mp3", b"x")]);
        let err =
            preflight(&root.path().join("a"), &root.path().join("a/inner"), None, VerifyLevel::Metadata)
                .unwrap_err();
        assert!(err.0.contains("inside the source"), "{}", err.0);
    }

    #[test]
    fn creating_the_destination_parent_is_reported_for_undo() {
        let root = dir_with(&[("a/x.mp3", b"x")]);
        let from = root.path().join("a");
        let to = root.path().join("Artist/1999 (Album)");

        let before = preflight(&from, &to, None, VerifyLevel::Metadata).unwrap();
        let outcome = execute_move(&from, &to, &before, VerifyLevel::Metadata, CrossDevice::Refuse).unwrap();

        assert_eq!(outcome.created_dirs, vec![root.path().join("Artist")]);
    }

    #[test]
    fn a_staging_directory_is_never_shared() {
        let root = tempfile::tempdir().unwrap();
        let a = reserve_temp_dir(root.path()).unwrap();
        let b = reserve_temp_dir(root.path()).unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
    }

    #[test]
    fn copy_tree_reproduces_the_whole_shape() {
        let src = dir_with(&[("cd1/a.mp3", b"aaa"), ("cd2/b.mp3", b"bb"), ("cover.jpg", b"j")]);
        let dst = tempfile::tempdir().unwrap();
        copy_tree(src.path(), dst.path()).unwrap();

        let a = inventory(src.path(), true).unwrap();
        let b = inventory(dst.path(), true).unwrap();
        assert!(a.diff(&b).is_empty(), "{:?}", a.diff(&b));
    }
}
