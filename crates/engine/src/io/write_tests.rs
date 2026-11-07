//! Real publication ordering and faults; creation, writes, replacement and cleanup stay real.
use super::*;
use crate::owned_directory::Directory;
use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, Error, ErrorKind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

fn directory() -> Directory {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Directory::acquire(
        (0..).map(|_| std::env::temp_dir().join(format!("gridcraft-save-sync-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))),
    )
    .unwrap()
}

fn temporary(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.sctmp", path.to_str().unwrap()))
}

fn publish(path: &Path, bytes: &[u8], sync: impl FnOnce(&File) -> io::Result<()>) -> Result<()> {
    write_file_with_sync(path.to_str().unwrap(), bytes, sync, |_| Ok(()))
}

#[test]
fn file_sync_precedes_publication_exactly_once() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    let bytes = vec![b'n'; 100_000];
    let calls = Cell::new(0);
    publish(&path, &bytes, |file| {
        calls.set(calls.get() + 1);
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read(temporary(&path)).unwrap(), bytes);
        file.sync_all()
    })
    .unwrap();
    assert_eq!(calls.get(), 1, "the barrier must not be omitted");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!temporary(&path).try_exists().unwrap());
}

fn refused_sync(error: Error, initially_absent: bool) {
    let dir = directory();
    let path = dir.join("book.xlsx");
    if !initially_absent {
        fs::write(&path, b"original").unwrap();
    }
    let message = error.to_string();
    let calls = Cell::new(0);
    let parents = Cell::new(0);
    let result = write_file_with_sync(
        path.to_str().unwrap(),
        b"complete replacement",
        |_| {
            calls.set(calls.get() + 1);
            if initially_absent {
                assert!(!path.try_exists().unwrap());
            } else {
                assert_eq!(fs::read(&path).unwrap(), b"original");
            }
            assert_eq!(fs::read(temporary(&path)).unwrap(), b"complete replacement");
            Err(error)
        },
        |_| {
            parents.set(parents.get() + 1);
            Ok(())
        },
    );
    assert_eq!(calls.get(), 1);
    let failure = result.unwrap_err().to_string();
    assert!(failure.starts_with(&format!("{}: ", path.display())), "{failure}");
    assert!(failure.contains(&message), "{failure}");
    if initially_absent {
        assert!(!path.try_exists().unwrap());
    } else {
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }
    assert!(!temporary(&path).try_exists().unwrap());
    assert_eq!(parents.get(), 0, "no parent barrier before publication");
}

#[test]
fn distinctive_sync_failure_preserves_original() {
    refused_sync(Error::other("distinctive file barrier failure"), false);
}

#[test]
fn unsupported_sync_failure_preserves_original() {
    refused_sync(Error::new(ErrorKind::Unsupported, "unsupported file barrier"), false);
}

#[test]
fn sync_failure_keeps_absent_destination_absent() {
    refused_sync(Error::other("absent destination barrier failure"), true);
}

#[cfg(unix)]
#[test]
fn unix_raw25_sync_failure_preserves_original() {
    refused_sync(Error::from_raw_os_error(25), false);
}

#[cfg(windows)]
#[test]
fn windows_error_seek_sync_failure_preserves_original() {
    refused_sync(Error::from_raw_os_error(25), false);
}

#[cfg(target_vendor = "apple")]
#[test]
fn apple_enotsup_sync_failure_preserves_original() {
    refused_sync(Error::from_raw_os_error(45), false);
}

#[test]
fn actual_sync_replaces_with_shorter_bytes_and_creates_new_file() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"a much longer original workbook").unwrap();
    write_file(path.to_str().unwrap(), b"new").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert!(!temporary(&path).try_exists().unwrap());
    let fresh = dir.join("fresh.xlsx");
    write_file(fresh.to_str().unwrap(), b"fresh").unwrap();
    assert_eq!(fs::read(&fresh).unwrap(), b"fresh");
    assert!(!temporary(&fresh).try_exists().unwrap());
}

#[test]
fn rename_failure_cleans_owned_temp_without_parent_sync() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep"), b"directory sentinel").unwrap();
    let calls = Cell::new(0);
    let parents = Cell::new(0);
    let result = write_file_with_sync(
        path.to_str().unwrap(),
        b"new",
        |file| {
            calls.set(calls.get() + 1);
            assert_eq!(fs::read(temporary(&path)).unwrap(), b"new");
            file.sync_all()
        },
        |_| {
            parents.set(parents.get() + 1);
            Ok(())
        },
    );
    assert!(result.unwrap_err().to_string().starts_with(&format!("{}: ", path.display())));
    assert_eq!(calls.get(), 1);
    assert_eq!(parents.get(), 0);
    assert_eq!(fs::read(path.join("keep")).unwrap(), b"directory sentinel");
    assert!(!temporary(&path).try_exists().unwrap());
}

#[test]
fn failed_exclusive_open_preserves_planted_file_and_never_syncs() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    fs::write(temporary(&path), b"planted temporary").unwrap();
    assert!(publish(&path, b"new", |_| panic!("failed open cannot sync")).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read(temporary(&path)).unwrap(), b"planted temporary");
    let missing = dir.join("missing/book.xlsx");
    assert!(publish(&missing, b"new", |_| panic!("missing parent cannot sync")).is_err());
    assert!(!missing.parent().unwrap().try_exists().unwrap());
}

#[cfg(unix)]
#[test]
fn failed_exclusive_open_preserves_planted_symlink_identity() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let dir = directory();
    let path = dir.join("book.xlsx");
    let target = dir.join("sentinel");
    fs::write(&path, b"original").unwrap();
    fs::write(&target, b"target sentinel").unwrap();
    symlink("sentinel", temporary(&path)).unwrap();
    let before = fs::symlink_metadata(temporary(&path)).unwrap();
    assert!(publish(&path, b"new", |_| panic!("planted link cannot sync")).is_err());
    let after = fs::symlink_metadata(temporary(&path)).unwrap();
    assert!(after.file_type().is_symlink());
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(fs::read_link(temporary(&path)).unwrap(), Path::new("sentinel"));
    assert_eq!(fs::read(&target).unwrap(), b"target sentinel");
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[cfg(unix)]
#[test]
fn cleanup_failure_keeps_primary_sync_error_and_original() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    let calls = Cell::new(0);
    let result = publish(&path, b"new", |_| {
        calls.set(calls.get() + 1);
        fs::rename(temporary(&path), dir.join("owned-moved-temp")).unwrap();
        fs::create_dir(temporary(&path)).unwrap();
        fs::write(temporary(&path).join("keep"), b"cleanup blocker").unwrap();
        Err(Error::other("primary barrier failure"))
    });
    assert_eq!(calls.get(), 1);
    let error = result.unwrap_err().to_string();
    assert!(error.starts_with(&format!("{}: ", path.display())), "{error}");
    assert!(error.contains("primary barrier failure"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read(temporary(&path).join("keep")).unwrap(), b"cleanup blocker");
    assert_eq!(fs::read(dir.join("owned-moved-temp")).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn parent_sync_attempt_follows_publication() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    let parents = Cell::new(0);
    let attempted = Cell::new(false);
    write_file_with_sync(path.to_str().unwrap(), b"new", File::sync_all, |parent| {
        parents.set(parents.get() + 1);
        assert_eq!(parent, &*dir);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!temporary(&path).try_exists().unwrap());
        let result = File::open(parent)?.sync_all();
        attempted.set(true);
        result
    })
    .unwrap();
    assert_eq!(parents.get(), 1);
    assert!(attempted.get(), "attempt real parent sync even if the filesystem refuses it");
    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn parent_sync_failure_still_reports_published_success() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    let parents = Cell::new(0);
    write_file_with_sync(path.to_str().unwrap(), b"new", File::sync_all, |_| {
        parents.set(parents.get() + 1);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!temporary(&path).try_exists().unwrap());
        Err(Error::other("distinctive parent barrier failure"))
    })
    .unwrap();
    assert_eq!(parents.get(), 1);
    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[test]
fn sandbox_refusal_precedes_access_and_both_sync_operations() {
    let dir = directory();
    let path = dir.join("book.xlsx");
    fs::write(&path, b"original").unwrap();
    let guard = ScriptSandbox::enter();
    let result =
        write_file_with_sync(path.to_str().unwrap(), b"new", |_| panic!("sandbox cannot file-sync"), |_| panic!("sandbox cannot parent-sync"));
    drop(guard);
    assert!(result.unwrap_err().to_string().contains("can't read or write files"));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(!temporary(&path).try_exists().unwrap());
}
