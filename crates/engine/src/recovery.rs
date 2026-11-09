//! Bounded recovery snapshot encoding and validation.
//!
//! Decoding checks codec, model and preserved-graph consistency. It does not restore a
//! session or establish trust, source provenance or storage authority. In particular,
//! untrusted `Plain` metadata never grants runtime Plain provenance. Source paths in
//! another platform's native representation are refused. Live capture construction
//! remains reserved for the later policy-aware capture boundary.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};

mod manifest;
mod provenance;
pub(crate) use provenance::OriginBinding;

const RECOVERY_MAGIC: &[u8; 8] = b"GCRCV001";
const RECOVERY_VERSION: u32 = 1;
const RECOVERY_HEADER: usize = 28;
const RECOVERY_FRAMES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Source origin lattice. Decoded labels alone confer no runtime authority.
pub enum RecoveryProvenance {
    Plain,
    EncryptedOrigin,
    Unknown,
}

impl RecoveryProvenance {
    /// Receive source content without ever lowering retained origin restrictions.
    pub fn receive(self, source: Self) -> Self {
        use RecoveryProvenance::*;
        match (self, source) {
            (EncryptedOrigin, _) | (_, EncryptedOrigin) => EncryptedOrigin,
            (Unknown, _) | (_, Unknown) => Unknown,
            (Plain, Plain) => Plain,
        }
    }
}

// Live capture policy must check eligibility before issuing any capture ticket.
#[allow(dead_code, reason = "private eligibility precedes the later live capture seam")]
fn eligible_origin(doc: &crate::DocState) -> Result<RecoveryProvenance, RecoveryError> {
    if doc.import_issues().iter().any(|issue| issue.effect == gridcraft_model::CompatibilityEffect::EncryptionRemoved) {
        return Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin));
    }
    match doc.recovery_provenance() {
        RecoveryProvenance::Plain => Ok(RecoveryProvenance::Plain),
        origin => Err(RecoveryError::Ineligible(origin)),
    }
}

#[derive(Debug)]
/// Finite codec refusals and sink error kinds, without input or operating-system text.
pub enum RecoveryError {
    UnsupportedVersion(u32),
    Malformed(&'static str),
    TooLarge(&'static str),
    UnsupportedRepresentation(&'static str),
    Ineligible(RecoveryProvenance),
    Io(std::io::ErrorKind),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(formatter, "unsupported recovery version {version}"),
            Self::Malformed(reason) => write!(formatter, "malformed recovery data ({reason})"),
            Self::TooLarge(reason) => write!(formatter, "recovery limit exceeded ({reason})"),
            Self::UnsupportedRepresentation(reason) => write!(formatter, "unsupported recovery representation ({reason})"),
            Self::Ineligible(provenance) => write!(formatter, "ineligible recovery provenance ({provenance:?})"),
            Self::Io(kind) => write!(formatter, "recovery sink error ({kind:?})"),
        }
    }
}
impl std::error::Error for RecoveryError {}

struct RecoveryLimits {
    envelope: usize,
    manifest: usize,
    part_bytes: usize,
    parts: usize,
    owner_relationships: usize,
    relationships: usize,
    xml: usize,
    sheets: usize,
    cells: usize,
    nodes: usize,
    depth: usize,
    metadata: usize,
    native_units: usize,
}

impl Default for RecoveryLimits {
    fn default() -> Self {
        Self {
            envelope: 256 * 1024 * 1024,
            manifest: 64 * 1024 * 1024,
            part_bytes: 64 * 1024 * 1024,
            parts: 4096,
            owner_relationships: 10_000,
            relationships: 100_000,
            xml: 4 * 1024 * 1024,
            sheets: 1024,
            cells: 1_000_000,
            nodes: 4_000_000,
            depth: 64,
            metadata: 32 * 1024,
            native_units: 32 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Inert snapshot identifier, parsed as 32 lowercase hexadecimal digits.
pub struct RecoveryId([u8; 16]);

impl std::str::FromStr for RecoveryId {
    type Err = RecoveryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(RecoveryError::Malformed("recovery_id"));
        }
        let mut bytes = [0; 16];
        for (out, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
            let digit = |b| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
            *out = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for RecoveryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut text = [0; 32];
        for (byte, pair) in self.0.iter().zip(text.chunks_exact_mut(2)) {
            pair[0] = b"0123456789abcdef"[(byte >> 4) as usize];
            pair[1] = b"0123456789abcdef"[(byte & 15) as usize];
        }
        serializer.serialize_str(std::str::from_utf8(&text).map_err(|_| ser::Error::custom("recovery_id"))?)
    }
}

impl<'de> Deserialize<'de> for RecoveryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor;
        impl de::Visitor<'_> for IdVisitor {
            type Value = RecoveryId;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("32 lowercase hexadecimal digits")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map_err(|_| E::custom("recovery_id"))
            }
        }
        deserializer.deserialize_str(IdVisitor)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum RecoveryFileKind {
    Xlsx,
    Xls,
    Xlsb,
    Ods,
    Csv,
    Tsv,
    Json,
    Html,
    Sylk,
}

impl From<crate::io::FileKind> for RecoveryFileKind {
    fn from(kind: crate::io::FileKind) -> Self {
        use crate::io::FileKind;
        match kind {
            FileKind::Xlsx => Self::Xlsx,
            FileKind::Xls => Self::Xls,
            FileKind::Xlsb => Self::Xlsb,
            FileKind::Ods => Self::Ods,
            FileKind::Csv => Self::Csv,
            FileKind::Tsv => Self::Tsv,
            FileKind::Json => Self::Json,
            FileKind::Html => Self::Html,
            FileKind::Sylk => Self::Sylk,
        }
    }
}

impl From<RecoveryFileKind> for crate::io::FileKind {
    fn from(kind: RecoveryFileKind) -> Self {
        match kind {
            RecoveryFileKind::Xlsx => Self::Xlsx,
            RecoveryFileKind::Xls => Self::Xls,
            RecoveryFileKind::Xlsb => Self::Xlsb,
            RecoveryFileKind::Ods => Self::Ods,
            RecoveryFileKind::Csv => Self::Csv,
            RecoveryFileKind::Tsv => Self::Tsv,
            RecoveryFileKind::Json => Self::Json,
            RecoveryFileKind::Html => Self::Html,
            RecoveryFileKind::Sylk => Self::Sylk,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum NativePath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

/// Borrowed capture supplied only by the later policy-aware live capture boundary.
/// No general workbook constructor or provenance bypass is exposed.
pub struct RecoveryCapture<'a> {
    workbook: &'a gridcraft_model::Workbook,
    metadata: &'a manifest::Metadata,
}
/// Frozen manifest and borrowed payload plan, ready for bounded streaming.
/// Writing does not establish durable or atomic storage.
pub struct PreparedRecovery<'a> {
    manifest: Vec<u8>,
    frames: manifest::frame::WriteParts<'a>,
}
/// Owned codec, model and preserved-graph validated data, without restore authority.
/// Its workbook remains private pending the actual session admission boundary.
pub struct ValidatedRecovery {
    workbook: gridcraft_model::Workbook,
    metadata: manifest::Metadata,
}
impl std::fmt::Debug for ValidatedRecovery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedRecovery")
            .field("id", &self.id())
            .field("sequence", &self.sequence())
            .field("captured_revision", &self.captured_revision())
            .field("worksheets", &self.workbook.sheets.len())
            .finish()
    }
}
impl<'a> RecoveryCapture<'a> {
    /// Validate and freeze this capture using the codec limits.
    pub fn prepare(&self) -> Result<PreparedRecovery<'a>, RecoveryError> {
        manifest::complete::prepare(self, &RecoveryLimits::default())
    }
}
impl PreparedRecovery<'_> {
    /// Exact encoded byte length, including the mandatory header.
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    /// Whether the encoded snapshot has zero bytes; complete snapshots include a header.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Stream frozen bytes without allocation or semantic work; failures report sink kinds.
    pub fn write_to(&self, mut sink: &mut dyn std::io::Write) -> Result<(), RecoveryError> {
        self.frames.write_to(&self.manifest, &mut sink)
    }
}
/// Validate a complete snapshot with fixed codec limits, without restoring a session.
/// Platform-native source paths may be refused on another platform.
pub fn decode(bytes: &[u8]) -> Result<ValidatedRecovery, RecoveryError> {
    manifest::complete::decode(bytes, &RecoveryLimits::default())
}

/// Restore an owned validated snapshot as a detached, untrusted, unsaved document.
/// This does not install it in a Session or grant source/capture/storage authority.
pub fn restore(validated: ValidatedRecovery) -> Result<crate::DocState, RecoveryError> {
    manifest::complete::restore(validated)
}

// Retained opaquely for later recovery policy. No path conversion, constructor or public getter.
#[allow(dead_code, reason = "inert context is retained even when current policy has no consumer")]
pub(crate) struct RestoreContext {
    original_path_hint: Option<NativePath>,
    id: RecoveryId,
    captured_revision: u64,
    sequence: u64,
    dirty_at_capture: bool,
    provenance: RecoveryProvenance,
}
impl std::fmt::Debug for RestoreContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("RestoreContext").finish_non_exhaustive()
    }
}
impl RestoreContext {
    pub(crate) fn id(&self) -> RecoveryId {
        self.id
    }
    pub(crate) fn captured_revision(&self) -> u64 {
        self.captured_revision
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    #[test]
    fn lattice_all_pairs() {
        use RecoveryProvenance::*;
        for (a, b, want) in [
            (Plain, Plain, Plain),
            (Plain, Unknown, Unknown),
            (Plain, EncryptedOrigin, EncryptedOrigin),
            (Unknown, Plain, Unknown),
            (Unknown, Unknown, Unknown),
            (Unknown, EncryptedOrigin, EncryptedOrigin),
            (EncryptedOrigin, Plain, EncryptedOrigin),
            (EncryptedOrigin, Unknown, EncryptedOrigin),
            (EncryptedOrigin, EncryptedOrigin, EncryptedOrigin),
        ] {
            assert_eq!(a.receive(b), want);
        }
    }
    #[test]
    fn unknown_entry_cannot_capture() {
        let doc = crate::DocState::new(gridcraft_model::Workbook::new(), None, "unknown".into(), crate::Trust::Local);
        assert!(matches!(eligible_origin(&doc), Err(RecoveryError::Ineligible(RecoveryProvenance::Unknown))));
    }
    #[test]
    fn encrypted_source_never_writes_plaintext_recovery() {
        let mut s = crate::Session::new();
        let bytes =
            gridcraft_xlsx::testing::encrypt_workbook(&crate::io::save_bytes(&gridcraft_model::Workbook::new(), "source.xlsx").unwrap(), "synthetic");
        s.execute("file.open", serde_json::json!({"name":"source.dat","base64":crate::io::base64_encode(&bytes),"password":"synthetic"})).unwrap();
        assert_eq!(s.doc().unwrap().recovery_provenance(), RecoveryProvenance::EncryptedOrigin);
        assert!(matches!(eligible_origin(s.doc().unwrap()), Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))));
    }
    #[test]
    fn retained_encryption_removed_fact_refuses_plain_origin() {
        let mut s = crate::Session::new();
        s.new_workbook();
        assert_eq!(eligible_origin(s.doc().unwrap()).unwrap(), RecoveryProvenance::Plain);
        s.doc_mut().unwrap().retain_import_issues(vec![gridcraft_model::CompatibilityIssue::new(
            "encryption",
            None,
            gridcraft_model::CompatibilityEffect::EncryptionRemoved,
            gridcraft_model::Editability::Editable,
            "synthetic",
        )]);
        assert!(matches!(eligible_origin(s.doc().unwrap()), Err(RecoveryError::Ineligible(RecoveryProvenance::EncryptedOrigin))));
    }
}
