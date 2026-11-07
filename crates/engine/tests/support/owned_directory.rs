//! Exclusive ownership and checked cleanup for synthetic test directories.

// The tuple constructor is used only immediately after an independent successful create_dir.
// Never construct an owner from a path that already exists or was acquired by another fixture.
pub(super) struct Directory(pub(super) std::path::PathBuf);

impl Directory {
    pub(super) fn acquire(candidates: impl IntoIterator<Item = std::path::PathBuf>) -> std::io::Result<Self> {
        for path in candidates {
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "fixture candidates exhausted"))
    }
}

impl std::ops::Deref for Directory {
    type Target = std::path::Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // Only exclusively acquired synthetic fixture directories may be cleaned up.
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if std::thread::panicking() {
                use std::io::Write;
                // Keep the original panic. Unlike eprintln!, this cannot panic on a stderr error;
                // a failed diagnostic write has no reliable secondary reporting channel here.
                let _ = writeln!(std::io::stderr().lock(), "cannot clean up owned fixture directory {}: {error:?}", self.0.display());
            } else {
                panic!("cannot clean up owned fixture directory {}: {error:?}", self.0.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Directory;

    fn directory(name: &str) -> Directory {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Directory::acquire((0..).map(|_| {
            std::env::temp_dir().join(format!("gridcraft-owned-directory-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
        }))
        .unwrap()
    }

    #[test]
    fn fixture_directory_cleanup_failure_fails_test() {
        let dir = directory("cleanup-error");
        let path = dir.to_path_buf();
        std::fs::remove_dir(&path).unwrap();
        let failure = std::panic::catch_unwind(|| drop(dir)).expect_err("cleanup failure must fail the test");
        let message = failure.downcast_ref::<String>().unwrap();
        assert!(message.contains(path.to_str().unwrap()), "cleanup failure must identify its directory: {message}");
        assert!(message.contains("NotFound"), "cleanup failure must retain the filesystem error: {message}");
    }

    #[test]
    fn fixture_directory_cleanup_failure_during_unwind_is_reported() {
        const CHILD: &str = "GRIDCRAFT_FIXTURE_CLEANUP_UNWIND_CHILD";
        let thread = std::thread::current();
        let test = thread.name().unwrap();
        if std::env::var(CHILD).as_deref() == Ok(test) {
            let dir = directory("cleanup-unwind");
            std::fs::remove_dir(&*dir).unwrap();
            println!("fixture_cleanup_target={}", dir.display());
            panic!("original fixture unwind failure");
        }
        // Only the child gets the marker: no process-global environment or panic hook changes.
        let output =
            std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", test, "--nocapture"]).env(CHILD, test).output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(101), "original test panic must survive without abort: {stdout}\n{stderr}");
        assert!(stderr.contains("original fixture unwind failure"), "original panic must remain visible: {stderr}");
        let path = stdout.lines().find_map(|line| line.strip_prefix("fixture_cleanup_target=")).unwrap();
        assert!(stderr.contains(&format!("cannot clean up owned fixture directory {path}:")), "cleanup error must be visible: {stderr}");
        assert!(stderr.contains("NotFound"), "cleanup diagnostic must retain the filesystem error: {stderr}");
    }
}
