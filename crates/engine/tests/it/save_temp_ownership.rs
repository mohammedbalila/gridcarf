//! Ordinary saves may clean up only temporary files they created themselves.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gridcraft_engine::{DocState, Session, core::CellRef, core::Value, io};
use serde_json::json;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let dir = std::env::temp_dir().join(format!("gridcraft-save-ownership-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&dir) {
                Ok(()) => return Self(dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create owned fixture directory: {e}"),
            }
        }
    }

    fn destination(&self) -> PathBuf {
        self.0.join("book.xlsx")
    }

    fn temporary(&self) -> PathBuf {
        self.0.join("book.xlsx.sctmp")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This instance owns the directory only after create_dir succeeded exclusively.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn plant_file(path: &Path, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

fn saved_then_edited(fixture: &Fixture) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"original"})).unwrap();
    s.execute_headless("file.save", json!({"path":fixture.destination()})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    assert_eq!(s.doc().unwrap().path.as_deref(), fixture.destination().to_str());
    s.execute("cell.set", json!({"cell":"A1", "input":"edited"})).unwrap();
    assert_dirty_edit(s.doc().unwrap());
    s
}

fn assert_dirty_edit(doc: &DocState) {
    assert!(doc.is_dirty(), "a failed save must not mark the edited document clean");
    assert_eq!(doc.wb.sheets[0].value(CellRef::new(0, 0)), Value::text("edited"));
}

#[test]
fn save_does_not_remove_existing_temporary_file() {
    let fixture = Fixture::new();
    let mut s = saved_then_edited(&fixture);
    let original = fs::read(fixture.destination()).unwrap();
    plant_file(&fixture.temporary(), b"pre-existing temporary file");

    for _ in 0..2 {
        assert!(s.execute_headless("file.save", json!({})).is_err());
        assert!(fixture.temporary().is_file(), "save removed the pre-existing temporary file");
        assert_eq!(fs::read(fixture.temporary()).unwrap(), b"pre-existing temporary file");
        assert_eq!(fs::read(fixture.destination()).unwrap(), original);
        assert_dirty_edit(s.doc().unwrap());
    }
}

#[test]
fn autosave_does_not_remove_existing_temporary_file() {
    let fixture = Fixture::new();
    let mut s = saved_then_edited(&fixture);
    s.execute("selection.set", json!({"range":"B2:C3"})).unwrap();
    let target = s.doc().unwrap().uid;
    let original = fs::read(fixture.destination()).unwrap();
    plant_file(&fixture.temporary(), b"pre-existing AutoSave temporary file");

    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1", "input":"active workbook"})).unwrap();
    s.execute("selection.set", json!({"range":"D4:E5"})).unwrap();
    let active = s.doc().unwrap().uid;
    let window = s.active_window();
    let windows = s.window_list();

    for _ in 0..2 {
        assert!(s.autosave_document(target).is_err());
        assert!(fixture.temporary().is_file(), "AutoSave removed the pre-existing temporary file");
        assert_eq!(fs::read(fixture.temporary()).unwrap(), b"pre-existing AutoSave temporary file");
        assert_eq!(fs::read(fixture.destination()).unwrap(), original);
        let doc = s.documents().iter().find(|d| d.uid == target).unwrap();
        assert_dirty_edit(doc);
        assert_eq!(doc.selection.a1(), "B2:C3");
        assert_eq!(s.doc().unwrap().uid, active);
        assert_eq!(s.active_window(), window);
        assert_eq!(s.window_list(), windows);
        assert_eq!(s.doc().unwrap().selection.a1(), "D4:E5");
        assert_eq!(s.doc().unwrap().wb.sheets[0].value(CellRef::new(0, 0)), Value::text("active workbook"));
    }
}

#[cfg(unix)]
#[test]
fn save_does_not_remove_existing_temporary_symlink() {
    use std::os::unix::fs::{MetadataExt, symlink};

    let fixture = Fixture::new();
    let mut s = saved_then_edited(&fixture);
    let original = fs::read(fixture.destination()).unwrap();
    let target = fixture.0.join("sentinel.bin");
    plant_file(&target, b"symlink target bytes");
    symlink("sentinel.bin", fixture.temporary()).unwrap();
    let before = fs::symlink_metadata(fixture.temporary()).unwrap();

    for _ in 0..2 {
        assert!(s.execute_headless("file.save", json!({})).is_err());
        let after = fs::symlink_metadata(fixture.temporary()).expect("save removed the pre-existing temporary symlink");
        assert!(after.file_type().is_symlink());
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(fs::read_link(fixture.temporary()).unwrap(), Path::new("sentinel.bin"));
        assert_eq!(fs::read(&target).unwrap(), b"symlink target bytes");
        assert_eq!(fs::read(fixture.destination()).unwrap(), original);
        assert_dirty_edit(s.doc().unwrap());
    }
}

#[test]
fn successful_save_cleans_owned_temporary() {
    let fixture = Fixture::new();
    let mut s = saved_then_edited(&fixture);

    s.execute_headless("file.save", json!({})).unwrap();

    assert!(!s.doc().unwrap().is_dirty());
    assert!(!fixture.temporary().try_exists().unwrap());
    let (reopened, _) = io::open_bytes("book.xlsx", &fs::read(fixture.destination()).unwrap()).unwrap();
    assert_eq!(reopened.sheets[0].value(CellRef::new(0, 0)), Value::text("edited"));
}

#[test]
fn failed_replace_cleans_only_its_owned_temporary() {
    let fixture = Fixture::new();
    let mut s = saved_then_edited(&fixture);
    fs::remove_file(fixture.destination()).unwrap();
    fs::create_dir(fixture.destination()).unwrap();
    let child = fixture.destination().join("keep.bin");
    plant_file(&child, b"destination directory contents");
    let unrelated = fixture.0.join("other.xlsx.sctmp");
    plant_file(&unrelated, b"unrelated temporary file");

    for _ in 0..2 {
        assert!(s.execute_headless("file.save", json!({})).is_err());
        assert!(!fixture.temporary().try_exists().unwrap(), "failed replacement left its owned temporary file");
        assert!(fixture.destination().is_dir());
        assert_eq!(fs::read(&child).unwrap(), b"destination directory contents");
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated temporary file");
        assert_dirty_edit(s.doc().unwrap());
    }
}
