use super::read::{first_free_block, merge_vml};
use super::write::{NameMap, attach, part_rels_xml, remap, write_parts};
use super::*;

/// A sheet's kept content being written.
pub struct SheetKept<'a> {
    p: Option<&'a Preserved>,
    names: NameMap,
    ids: HashMap<String, String>,
}

impl<'a> SheetKept<'a> {
    /// Writes the sheet's kept parts and adds their relationships to the sheet's.
    pub fn write(sheet: &'a Sheet, rels: &mut Rels, out: &mut Out) -> SheetKept<'a> {
        let Some(p) = sheet.preserved.as_deref() else { return SheetKept { p: None, names: NameMap::new(), ids: HashMap::new() } };
        let names = write_parts(out, &p.parts);
        let ids = attach(rels, &p.rels, &names);
        SheetKept { p: Some(p), names, ids }
    }

    fn vml(&self) -> Option<&'a PreservedPart> {
        self.p?.vml.as_ref()
    }

    fn write_vml_rels(&self, vml: &PreservedPart, name: &str, out: &mut Out) {
        if !vml.rels.is_empty() {
            out.part(&rels_part_name(name), None, part_rels_xml(&vml.rels, &self.names).into_bytes());
        }
    }

    /// The number GridCraft's own VML shapes start after when they join the kept drawing
    /// ([`SheetKept::notes_vml`] numbers them `base + 1`, `base + 2`…); `None` without one.
    pub fn shape_base(&self) -> Option<u64> {
        let vml = self.vml()?;
        Some(first_free_block(&String::from_utf8_lossy(&vml.data)).saturating_mul(1024))
    }

    /// GridCraft's notes and form controls VML for part `name`, merged with the kept control and
    /// object shapes.
    pub fn notes_vml(&self, ours: String, name: &str, out: &mut Out) -> Result<String, IoError> {
        let Some(vml) = self.vml() else { return Ok(ours) };
        self.write_vml_rels(vml, name, out);
        merge_vml(&String::from_utf8_lossy(&vml.data), &ours)
    }

    /// The kept VML drawing of a sheet without notes; its new relationship id.
    pub fn lone_vml(&self, rels: &mut Rels, out: &mut Out) -> Option<String> {
        let vml = self.vml()?;
        let name = out.kept.claim(&vml.name);
        out.part(&name, Some(CT_VML), vml.data.to_vec());
        self.write_vml_rels(vml, &name, out);
        Some(rels.add("vmlDrawing", &format!("/{name}")))
    }

    /// The new relationship id for `<pageSetup r:id>` (printer settings).
    pub fn page_setup_id(&self) -> Option<&str> {
        let old = &self.p?.elements.iter().find(|e| e.slot == "pageSetup")?.xml;
        self.ids.get(old).map(String::as_str)
    }

    /// The kept sheet elements, in schema order, with GridCraft's own form control entries
    /// (`controls`, see `controls::write`) in the sheet's one `<controls>` element.
    pub fn elements(&self, controls: &str) -> Result<String, IoError> {
        let mut s = String::new();
        let mut placed = controls.is_empty();
        if let Some(p) = self.p {
            for slot in SHEET_SLOTS {
                for e in p.elements.iter().filter(|e| e.slot == *slot) {
                    if let Some(mut x) = remap(&e.xml, &self.ids) {
                        if *slot == "controls" && !placed {
                            match control_insertion(&x).ok_or_else(control_error)? {
                                ControlSelection::Insertion(at) => {
                                    x.insert_str(at.at, &controls_for_insertion(controls, at)?);
                                    placed = true;
                                }
                                ControlSelection::SelectedNoInsertion => return Err(control_error()),
                                ControlSelection::Absent => {}
                            }
                        }
                        s.push_str(&x);
                    }
                }
            }
        }
        // `<controls>` comes last of the kept elements.
        if !placed {
            s.push_str(&crate::controls::controls_element(controls));
        }
        Ok(s)
    }
}

#[derive(Clone, Copy)]
struct ControlInsertion {
    at: usize,
    main_default: bool,
    r_relationship: bool,
}
#[derive(Clone, Copy)]
enum ControlSelection {
    Absent,
    SelectedNoInsertion,
    Insertion(ControlInsertion),
}
fn control_error() -> IoError {
    IoError::Format("kept controls have no supported insertion context".into())
}

/// Select exactly the reader's plain controls, first Choice's direct controls, else
/// first Fallback's direct controls. A selected empty element is not an absent one.
fn control_insertion(x: &str) -> Option<ControlSelection> {
    use quick_xml::name::{Namespace, PrefixDeclaration, ResolveResult};
    let mut reader = quick_xml::NsReader::from_str(x);
    reader.resolver_mut().set_max_declarations_per_element(x.len());
    reader.resolver_mut().add(PrefixDeclaration::Default, Namespace(crate::write::NS_MAIN.as_bytes())).ok()?;
    reader.resolver_mut().add(PrefixDeclaration::Named(b"r"), Namespace(NS_REL.as_bytes())).ok()?;
    let (mut depth, mut alternate, mut active) = (0usize, false, None);
    let mut seen = [false; 2];
    let mut fallback = None;
    loop {
        let event = reader.read_event().ok()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let local = e.local_name();
                let name = local.as_ref();
                let level = depth + 1;
                if level == 1 {
                    alternate = name == b"AlternateContent";
                }
                if alternate && level == 2 {
                    let k = match name {
                        b"Choice" => Some(0),
                        b"Fallback" => Some(1),
                        _ => None,
                    };
                    active = k.filter(|k| !seen[*k]);
                    if let Some(k) = k {
                        seen[k] = true;
                    }
                    if empty {
                        active = None;
                    }
                }
                if name == b"controls" && (level == 1 || alternate && level == 3 && active.is_some()) {
                    let main_default = matches!(reader.resolver().resolve_prefix(None, true), ResolveResult::Bound(ns) if ns.as_ref() == crate::write::NS_MAIN.as_bytes());
                    let r_relationship =
                        matches!(reader.resolver().resolve_attribute(QName(b"r:id")).0, ResolveResult::Bound(ns) if ns.as_ref() == NS_REL.as_bytes());
                    let selected = if empty {
                        ControlSelection::SelectedNoInsertion
                    } else {
                        let at = usize::try_from(reader.read_to_end(e.name()).ok()?.end).ok()?;
                        ControlSelection::Insertion(ControlInsertion { at, main_default, r_relationship })
                    };
                    if level == 1 || active == Some(0) {
                        return Some(selected);
                    }
                    if fallback.is_none() {
                        fallback = Some(selected);
                    }
                    continue;
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                if depth == 2 {
                    active = None;
                }
                depth = depth.checked_sub(1)?;
            }
            Event::Eof => return (depth == 0).then_some(fallback.unwrap_or(ControlSelection::Absent)),
            _ => {}
        }
    }
}
pub(super) fn controls_insert_at(x: &str) -> Option<usize> {
    match control_insertion(x)? {
        ControlSelection::Insertion(at) => Some(at.at),
        _ => None,
    }
}
fn controls_for_insertion(entries: &str, at: ControlInsertion) -> Result<Cow<'_, str>, IoError> {
    if at.main_default && at.r_relationship {
        return Ok(Cow::Borrowed(entries));
    }
    // controls::write emits one such root per entry; both branches inherit these bindings.
    const ROOT: &str = "<mc:AlternateContent xmlns:mc=";
    if !entries.starts_with(ROOT) {
        return Err(control_error());
    }
    let mut root = String::from("<mc:AlternateContent");
    if !at.main_default {
        let _ = write!(root, " xmlns=\"{}\"", crate::write::NS_MAIN);
    }
    if !at.r_relationship {
        let _ = write!(root, " xmlns:r=\"{NS_REL}\"");
    }
    root.push_str(" xmlns:mc=");
    Ok(Cow::Owned(entries.replace(ROOT, &root)))
}

// ---------------------------------------------------------------- linked workbooks

/// `wb` as it is saved into the folder `to` (absolute): the relative paths of its linked
/// workbooks, relative to the folder it was read from, are rewritten relative to `to`, as Excel
/// does on Save As, or made absolute when no relative path exists (another drive). `None` when
/// nothing changes.
pub fn rebase_links(wb: &Workbook, to: &Path) -> Option<Workbook> {
    let p = wb.preserved.as_deref()?;
    let from = Folder::of(wb.source_dir.as_deref()?)?;
    let to = Folder::of(to)?;
    if from == to {
        return None;
    }
    let mut parts = p.parts.clone();
    let mut changed = false;
    for r in parts.iter_mut().flat_map(|part| part.rels.iter_mut()) {
        if r.external
            && is_kind(r, &["externalLinkPath"])
            && let Some(target) = rebase(&r.target, &from, &to)
        {
            r.target = target;
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let mut out = wb.clone();
    out.preserved = Some(Arc::new(Preserved { parts, ..p.clone() }));
    Some(out)
}

/// An absolute folder, lexically normalized.
#[derive(PartialEq)]
struct Folder {
    /// Drive or share and root separator (`/`, `C:\`, `\\server\share\`).
    root: String,
    names: Vec<String>,
}

impl Folder {
    fn of(dir: &Path) -> Option<Folder> {
        if !dir.is_absolute() {
            return None;
        }
        let mut f = Folder { root: String::new(), names: vec![] };
        for c in dir.components() {
            match c {
                Component::Prefix(_) | Component::RootDir => f.root.push_str(&c.as_os_str().to_string_lossy()),
                Component::CurDir => {}
                Component::ParentDir => {
                    f.names.pop();
                }
                Component::Normal(n) => f.names.push(n.to_string_lossy().into_owned()),
            }
        }
        Some(f)
    }
}

/// A relative link target read from `from`, rewritten for `to`. `None` keeps it as it is: an
/// absolute path or URL, or a path that climbs above the root.
fn rebase(target: &str, from: &Folder, to: &Folder) -> Option<String> {
    if !is_relative_path(target) {
        return None;
    }
    // Each name as the file system knows it, and as it is written in a target. Names taken from
    // the original target keep their spelling.
    let mut path: Vec<(String, String)> = from.names.iter().map(|n| (n.clone(), encode_name(n))).collect();
    for seg in target.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                path.pop()?;
            }
            _ => path.push((decode_name(seg), seg.to_string())),
        }
    }
    let folders = path.len().checked_sub(1)?;
    if !from.root.eq_ignore_ascii_case(&to.root) {
        let sep = if from.root.contains('\\') { "\\" } else { "/" };
        let names: Vec<&str> = path.iter().map(|(name, _)| name.as_str()).collect();
        return Some(format!("file:///{}{}", from.root, names.join(sep)));
    }
    let common = to.names.iter().zip(&path).take(folders).take_while(|(a, (b, _))| *a == b).count();
    let up = "../".repeat(to.names.len().saturating_sub(common));
    let down: Vec<&str> = path.iter().skip(common).map(|(_, written)| written.as_str()).collect();
    Some(format!("{up}{}", down.join("/")))
}

/// Not empty, no root, no scheme or drive (`file:`, `http:`, `C:`).
fn is_relative_path(target: &str) -> bool {
    let scheme = target.split_once(':').is_some_and(|(s, _)| {
        s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    !target.is_empty() && !target.starts_with(['/', '\\']) && !scheme
}

/// A folder name as a segment of a relationship target (a URI): ASCII characters a URI path
/// can't hold are percent-encoded, other characters are written as they are.
fn encode_name(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for c in name.chars() {
        if !c.is_ascii() || c.is_ascii_alphanumeric() || "-._~!$&'()+,;=@".contains(c) {
            s.push(c);
        } else {
            let _ = write!(s, "%{:02X}", c as u32);
        }
    }
    s
}

/// A target segment with its percent-escapes decoded; as written when they don't decode to UTF-8.
fn decode_name(seg: &str) -> String {
    let b = seg.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) if c == b'%' => {
                out.push(byte);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| seg.to_string())
}
