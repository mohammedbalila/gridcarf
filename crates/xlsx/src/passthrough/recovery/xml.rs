//! Streaming owner slots, opaque references and the writer's VML/chart representability.
use super::scanner::{self, Failure, Name, RecoveryToken as Token, Tag};
use super::{Allowance, Checked, Error, MAX_TEXT, OwnerIndex, reserve};
use gridcraft_model::{Preserved, PreservedPart, PreservedRel, Workbook};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const VML: &str = "urn:schemas-microsoft-com:vml";
const OFFICE: &str = "urn:schemas-microsoft-com:office:office";
const CALC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/calcfeatures";
fn push<T>(v: &mut Vec<T>, x: T) -> Checked<()> {
    reserve(v, 1)?;
    v.push(x);
    Ok(())
}
fn copy(s: &str) -> Checked<String> {
    let mut v = String::new();
    v.try_reserve_exact(s.len()).map_err(|_| Error::Allocation)?;
    v.push_str(s);
    Ok(v)
}
fn scan<'a>(data: &'a [u8], fragment: bool, remap: bool, mut visit: impl FnMut(&[Name], Token<'a>) -> Checked<()>) -> Checked<()> {
    let mut callback = None;
    scanner::scan_recovery(data, fragment, remap, |p, t| {
        visit(p, t).map_err(|e| {
            callback = Some(e);
            Failure::Malformed
        })
    })
    .map_err(|e| {
        callback.unwrap_or(match e {
            Failure::Malformed => Error::Xml,
            Failure::Unsupported => Error::UnsupportedXml,
            Failure::Limit => Error::XmlLimit,
            Failure::Allocation => Error::Allocation,
        })
    })
}
fn is(n: &Name, ns: &str, name: &str) -> bool {
    n.ns == ns && n.local == name
}
fn relationship<'a>(rels: &[&'a PreservedRel], id: &str) -> Checked<&'a PreservedRel> {
    rels.binary_search_by(|r| r.id.as_str().cmp(id)).ok().and_then(|i| rels.get(i).copied()).ok_or(Error::MissingTarget)
}
fn expected(r: &PreservedRel, kinds: &[&str]) -> Checked<()> {
    if kinds.is_empty() || super::super::is_kind(r, kinds) { Ok(()) } else { Err(Error::Relationship) }
}
fn reference_kind(name: &Name) -> &'static [&'static str] {
    match (name.ns.as_str(), name.local.as_str()) {
        (MAIN, "picture") | (MAIN, "objectPr") | (VML, "imagedata") => &["image"],
        (MAIN, "oleObject") => &["oleObject", "package"],
        (MAIN, "control") => &["ctrlProp", "control"],
        (MAIN, "pageSetup") => &["printerSettings"],
        (MAIN, "legacyDrawing") | (MAIN, "legacyDrawingHF") => &["vmlDrawing"],
        (MAIN, "drawing") => &["drawing"],
        (MAIN, "externalBook") => &["externalLinkPath"],
        _ => &[],
    }
}
fn references(tag: &Tag, name: &Name, rels: &[&PreservedRel]) -> Checked<()> {
    for (n, f) in &tag.attrs {
        if (n.ns == REL || n.ns == super::super::NS_REL_STRICT) || (n.ns == OFFICE && n.local == "relid") {
            let r = relationship(rels, &f.value)?;
            expected(r, reference_kind(name))?;
        }
    }
    Ok(())
}

pub(crate) struct Roots<'a> {
    pub ordered: Vec<(&'a PreservedRel, Allowance)>,
    pub shape_base: Option<u64>,
}
struct Claims<'a, 'i> {
    index: &'i OwnerIndex<'a>,
    roots: Vec<(&'a PreservedRel, Allowance)>,
    claimed: Vec<u8>,
}
impl<'a> Claims<'a, '_> {
    fn claim(&mut self, id: &str, allowance: Allowance, kinds: &[&str]) -> Checked<()> {
        let i = self.index.rels.binary_search_by(|r| r.id.as_str().cmp(id)).map_err(|_| Error::MissingTarget)?;
        let r = self.index.rels[i];
        expected(r, kinds)?;
        // Repeated Choice/Fallback references are checked each time. Only their identical
        // root/context expansion is coalesced; first occurrence retains the reader's order.
        if self.claimed[i] & (allowance as u8) == 0 {
            push(&mut self.roots, (r, allowance))?;
            self.claimed[i] |= allowance as u8;
        }
        Ok(())
    }
}

pub(crate) fn owner_roots<'a>(
    p: &'a Preserved,
    index: &OwnerIndex<'a>,
    workbook: bool,
    native_shapes: usize,
    native_controls: bool,
) -> Checked<Roots<'a>> {
    let mut claimed = Vec::new();
    reserve(&mut claimed, index.rels.len())?;
    claimed.resize(index.rels.len(), 0);
    let mut c = Claims { index, roots: Vec::new(), claimed };
    for r in &p.package_rels {
        push(&mut c.roots, (relationship(&index.package, &r.id)?, Allowance::Ordinary))?;
    }
    for r in &p.rels {
        if super::super::is_kind(r, if workbook { super::super::WORKBOOK_ROOTS } else { super::super::SHEET_ROOTS }) {
            c.claim(&r.id, Allowance::Ordinary, &[])?;
        }
    }
    let mut shape_base = None;
    let mut numbers = Vec::new();
    if workbook {
        for sheet in &p.sheets {
            c.claim(&sheet.rel_id, Allowance::Chart, &["chartsheet"])?;
            let r = relationship(&index.rels, &sheet.rel_id)?;
            if r.external {
                return Err(Error::ChartSheet);
            }
            super::part_name(&r.target)?;
            index.part(&r.target).ok_or(Error::MissingTarget)?;
        }
    } else {
        if let Some(e) = p.elements.iter().find(|e| e.slot == "pageSetup") {
            c.claim(&e.xml, Allowance::Ordinary, &["printerSettings"])?;
        }
        if let Some(e) = p.elements.iter().find(|e| e.slot == "legacyDrawingHF") {
            slot(e, &mut c, &numbers, native_controls)?;
        }
        if let Some(q) = &p.vml {
            let v = vml(q, &index.vml_rels, native_shapes != 0)?;
            numbers = v.numbers;
            let block = v.raw_max / 1024 + 1;
            let base = block.checked_mul(1024).ok_or(Error::VmlLimit)?;
            shape_base = Some(base);
            block.checked_add((native_shapes as u64).checked_add(1).ok_or(Error::VmlLimit)?.div_ceil(1024).max(1)).ok_or(Error::VmlLimit)?;
            base.checked_add(native_shapes as u64).ok_or(Error::VmlLimit)?;
            for r in &q.rels {
                push(&mut c.roots, (r, Allowance::Ordinary))?;
            }
        }
    }
    for e in &p.elements {
        if e.slot != "pageSetup" && e.slot != "legacyDrawingHF" {
            slot(e, &mut c, &numbers, native_controls)?;
        }
    }
    if c.claimed.contains(&0) {
        return Err(Error::Slot);
    }
    Ok(Roots { ordered: c.roots, shape_base })
}

fn slot(e: &gridcraft_model::PreservedXml, c: &mut Claims<'_, '_>, shapes: &[u64], native_controls: bool) -> Checked<()> {
    let mut found = false;
    let mut refs = Vec::new();
    let merge = native_controls && e.slot == "controls";
    let chosen = if merge { control_insertion(&e.xml) } else { None };
    let (mut root_controls, mut root_close, mut root_context) = (false, None, false);
    let mut contexts = [false; 2];
    let (mut seen, mut active, mut waiting) = ([false; 2], [false; 2], [false; 2]);
    let mut containers: [Option<Option<usize>>; 2] = [None, None];
    let branch = |n: &Name| {
        if is(n, MC, "Choice") {
            Some(0)
        } else if is(n, MC, "Fallback") {
            Some(1)
        } else {
            None
        }
    };
    scan(e.xml.as_bytes(), true, !matches!(e.slot.as_str(), "fileVersion" | "ext"), |path, token| {
        let name = path.last().ok_or(Error::Slot)?;
        match token {
            Token::Start(tag) => {
                if merge {
                    if path.len() == 1 {
                        root_controls = is(name, MAIN, "controls");
                        root_context = tag.main_default && tag.r_rel;
                    }
                    if path.len() == 2
                        && let Some(k) = branch(name)
                    {
                        active[k] = !seen[k];
                        seen[k] = true;
                    }
                    if path.len() == 3
                        && is(name, MAIN, "controls")
                        && let Some(k) = branch(&path[1])
                        && active[k]
                        && containers[k].is_none()
                    {
                        containers[k] = Some(None);
                        contexts[k] = tag.main_default && tag.r_rel;
                        waiting[k] = true;
                    }
                }

                let inside = path.iter().any(|n| is(n, MAIN, &e.slot));
                if path.len() == 1
                    && !is(name, MAIN, &e.slot)
                    && !(matches!(e.slot.as_str(), "controls" | "oleObjects") && is(name, MC, "AlternateContent"))
                {
                    return Err(Error::Slot);
                }
                if matches!(e.slot.as_str(), "controls" | "oleObjects")
                    && !inside
                    && !(name.ns == MC && matches!(name.local.as_str(), "AlternateContent" | "Choice" | "Fallback"))
                {
                    return Err(Error::Slot);
                }
                if is(name, MAIN, &e.slot) {
                    found = true;
                }
                if e.slot == "ext" {
                    if path.len() == 1 && !tag.field("", "uri").is_some_and(|f| f.value.eq_ignore_ascii_case(super::super::CALC_FEATURES_URI)) {
                        return Err(Error::Slot);
                    }
                    if path.len() > 1 && !(name.ns == CALC && matches!(name.local.as_str(), "calcFeatures" | "feature")) {
                        return Err(Error::Slot);
                    }
                }
                if e.slot == "fileVersion" && path.len() != 1 {
                    return Err(Error::Slot);
                }
                if e.slot == "externalReferences" && path.len() > 1 && !(path.len() == 2 && is(name, MAIN, "externalReference")) {
                    return Err(Error::Slot);
                }
                if is(name, MAIN, "control") || is(name, MAIN, "oleObject") {
                    let id = tag.field("", "shapeId").ok_or(Error::Vml)?.value.trim().parse::<u64>().map_err(|_| Error::Vml)?;
                    if id == 0 {
                        return Err(Error::Vml);
                    }
                    push(&mut refs, id)?;
                }
                let mut used = false;
                for (n, f) in &tag.attrs {
                    if n.ns == REL {
                        let kinds = match e.slot.as_str() {
                            "externalReferences" if is(name, MAIN, "externalReference") => &["externalLink"][..],
                            "legacyDrawingHF" => &["vmlDrawing"][..],
                            "picture" => &["image"][..],
                            "fileVersion" | "ext" => return Err(Error::Slot),
                            _ => reference_kind(name),
                        };
                        c.claim(&f.value, if e.slot == "legacyDrawingHF" { Allowance::HeaderVml } else { Allowance::Ordinary }, kinds)?;
                        used = true;
                    }
                }
                if ((e.slot == "picture" || e.slot == "legacyDrawingHF") && path.len() == 1 || is(name, MAIN, "externalReference")) && !used {
                    return Err(Error::Slot);
                }
            }
            Token::End(span) if merge => {
                let close = e.xml.get(span.clone()).is_some_and(|s| s.starts_with("</")).then_some(span.start);
                if path.len() == 1 && root_controls {
                    root_close = close;
                }
                if path.len() == 3
                    && is(name, MAIN, "controls")
                    && let Some(k) = branch(&path[1])
                    && waiting[k]
                {
                    containers[k] = Some(close);
                    waiting[k] = false;
                }
                if path.len() == 2
                    && let Some(k) = branch(name)
                {
                    active[k] = false;
                }
            }
            Token::Text(s, _)
                if !(s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
                    || matches!(e.slot.as_str(), "oleObjects" | "controls") && path.iter().any(|n| is(n, MAIN, &e.slot))) =>
            {
                return Err(Error::Slot);
            }
            Token::Reference(ch)
                if !(matches!(ch, ' ' | '\t' | '\n' | '\r')
                    || matches!(e.slot.as_str(), "oleObjects" | "controls") && path.iter().any(|n| is(n, MAIN, &e.slot))) =>
            {
                return Err(Error::Slot);
            }
            _ => {}
        }
        Ok(())
    })?;
    if merge {
        let expected = if root_controls { root_close } else { containers[0].unwrap_or_else(|| containers[1].flatten()) };
        let default = if root_controls {
            root_context
        } else if containers[0].is_some() {
            contexts[0]
        } else {
            contexts[1]
        };
        if !default || expected.is_none() || expected != chosen {
            return Err(Error::UnsupportedXml);
        }
    }
    if !found {
        return Err(Error::Slot);
    }
    if refs.iter().any(|n| shapes.binary_search(n).is_err()) {
        return Err(Error::Vml);
    }
    Ok(())
}

/// Same bounded raw selector as SheetKept, without rebuilding XML. After remap, start
/// names keep their spelling; comments/end tags keep their raw bytes. Valid attributes
/// contain no literal '<', generated IDs are markup-safe, and the shared scanner refuses
/// raw nonrel double quotes that rebuild unsafely. Candidate identity/order therefore
/// survives byte shifts: compare the selected original token, never rewritten offsets.
fn control_insertion(text: &str) -> Option<usize> {
    fn local(s: &str) -> &str {
        s.rsplit(':').next().unwrap_or("")
    }
    let from = text
        .match_indices('<')
        .find_map(|(i, _)| {
            let rest = &text[i + 1..];
            let end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
            (local(&rest[..end]) == "Choice").then_some(i)
        })
        .unwrap_or(0);
    text[from..].match_indices("</").find_map(|(i, _)| {
        let rest = &text[from + i + 2..];
        let end = rest.find('>')?;
        (local(rest[..end].trim_end()) == "controls").then_some(from + i)
    })
}

struct VmlFacts {
    numbers: Vec<u64>,
    raw_max: u64,
    reserved: u64,
}
fn numeric(s: &str) -> Checked<Option<u64>> {
    let s = s.trim();
    let digits = s.strip_prefix("_x0000_s").unwrap_or(s);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        let n = digits.parse().map_err(|_| Error::Vml)?;
        if digits.len() > 18 {
            return Err(Error::UnsupportedXml);
        }
        if n == 0 {
            return Err(Error::Vml);
        }
        Ok(Some(n))
    } else if s.starts_with("_x0000_s") {
        Err(Error::Vml)
    } else {
        Ok(None)
    }
}
fn vml(q: &PreservedPart, rels: &[&PreservedRel], merge: bool) -> Checked<VmlFacts> {
    if q.data.len() > super::super::MAX_META as usize {
        return Err(Error::ByteLimit);
    }
    let mut ids = Vec::new();
    let mut numbers = Vec::new();
    let mut claimed = 0u64;
    let mut close = None;
    scan(&q.data, false, false, |path, token| {
        let name = path.last().ok_or(Error::Vml)?;
        match token {
            Token::Start(tag) => {
                if path.len() == 1 && !is(name, "", "xml") {
                    return Err(Error::Vml);
                }
                if path.len() == 1 && merge && !tag.vml_native {
                    return Err(Error::UnsupportedXml);
                }
                references(&tag, name, rels)?;
                if is(name, VML, "shape") {
                    let id = tag.field("", "id");
                    let spid = tag.field(OFFICE, "spid");
                    let a = id.map(|f| numeric(&f.value)).transpose()?.flatten();
                    let b = spid.map(|f| numeric(&f.value)).transpose()?.flatten();
                    if a.is_some() && b.is_some() && a != b {
                        return Err(Error::Vml);
                    }
                    if spid.is_some() && b.is_none() {
                        return Err(Error::Vml);
                    }
                    if let Some(f) = id {
                        push(&mut ids, copy(&f.value)?)?;
                    }
                    for f in id.into_iter().chain(spid) {
                        if numeric(&f.value)?.is_some() && q.data.get(f.span.clone()) != Some(f.value.as_bytes()) {
                            return Err(Error::UnsupportedXml);
                        }
                    }
                    if let Some(n) = a.or(b) {
                        if merge && !q.data[tag.span.clone()].windows(8).any(|s| s == b"_x0000_s") {
                            return Err(Error::UnsupportedXml);
                        }
                        push(&mut numbers, n)?;
                    }
                }
                if is(name, OFFICE, "idmap") {
                    let f = tag.field("", "data").ok_or(Error::Vml)?;
                    for n in f.value.split(',') {
                        let n = n.trim().parse::<u64>().map_err(|_| Error::Vml)?;
                        claimed = claimed.max(n);
                    }
                }
            }
            Token::End(span) if path.len() == 1 => {
                close = if q.data.get(span.clone()) == Some(b"</xml>".as_slice()) { Some(span.start) } else { None };
            }
            _ => {}
        }
        Ok(())
    })?;
    let text = std::str::from_utf8(&q.data).map_err(|_| Error::Xml)?;
    ids.sort_unstable();
    numbers.sort_unstable();
    if ids.windows(2).any(|p| p[0] == p[1]) || numbers.windows(2).any(|p| p[0] == p[1]) {
        return Err(Error::Vml);
    }
    if merge && (close.is_none() || text.rfind("</xml>") != close) {
        return Err(Error::UnsupportedXml);
    }
    // Match the actual helpers' raw decimal scan and literal first idmap/data search.
    let mut raw_max = 0u64;
    for (at, m) in text.match_indices("_x0000_s") {
        let rest = &text[at + m.len()..];
        let end = rest.bytes().take_while(u8::is_ascii_digit).count();
        if end > 18 {
            return Err(Error::UnsupportedXml);
        }
        if end != 0 {
            raw_max = raw_max.max(rest[..end].parse().map_err(|_| Error::Vml)?);
        }
    }
    let mut reserved = raw_max / 1024;
    if let Some(at) = text.find("<o:idmap").and_then(|i| text[i..].find("data=\"").map(|j| i + j + 6))
        && let Some(end) = text[at..].find('"')
    {
        for n in text[at..at + end].split(',') {
            reserved = reserved.max(n.trim().parse::<u64>().map_err(|_| Error::Vml)?);
        }
    }
    if claimed > reserved {
        return Err(Error::UnsupportedXml);
    }
    Ok(VmlFacts { numbers, raw_max, reserved })
}

fn sheet_formula(wb: &Workbook, text: &str) -> Checked<()> {
    super::super::write::visit_sheet_names(text.trim(), |raw, quoted| {
        let same = |name: &str| {
            let mut chars = raw.chars().peekable();
            let decoded = std::iter::from_fn(|| {
                let c = chars.next()?;
                if quoted && c == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                }
                Some(c.to_ascii_lowercase())
            });
            decoded.take(31).eq(name.chars().take(31).map(|c| c.to_ascii_lowercase()))
        };
        if wb.sheets.iter().any(|s| same(&s.name)) { Ok(()) } else { Err(Error::ChartSheet) }
    })
}
fn formula_text(out: &mut String, s: &str, normalize: bool) -> Checked<()> {
    if out.len().checked_add(s.len()).is_none_or(|n| n > MAX_TEXT) {
        return Err(Error::TextLimit);
    }
    out.try_reserve(s.len()).map_err(|_| Error::Allocation)?;
    if normalize {
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            } else {
                out.push(c);
            }
        }
    } else {
        out.push_str(s);
    }
    Ok(())
}
fn xml_bearing(q: &PreservedPart) -> bool {
    let mime = q.content_type.split(';').next().unwrap_or("").trim();
    mime.eq_ignore_ascii_case("application/xml")
        || mime.eq_ignore_ascii_case("text/xml")
        || mime.get(mime.len().saturating_sub(4)..).is_some_and(|s| s.eq_ignore_ascii_case("+xml"))
        || q.name.get(q.name.len().saturating_sub(4)..).is_some_and(|s| s.eq_ignore_ascii_case(".xml"))
}
/// Every XML-bearing retained part is scanned once. Chart roles come from actual local edges;
/// the visited Chart bit selects the writer's chart-formula scan without a root-by-root rescan.
pub(crate) fn parts(wb: &Workbook, p: &Preserved, index: &OwnerIndex<'_>, visited: &[u8]) -> Checked<u64> {
    let mut roles = Vec::new();
    reserve(&mut roles, index.parts.len())?;
    roles.resize(index.parts.len(), 0u8);
    for r in p.rels.iter().chain(&p.package_rels).chain(p.parts.iter().chain(p.vml.iter()).flat_map(|q| &q.rels)) {
        if r.external {
            continue;
        }
        let role = [("chartsheet", 1), ("drawing", 2), ("chart", 4), ("chartex", 8), ("vmlDrawing", 16)]
            .into_iter()
            .find(|(k, _)| super::super::kind(&r.rel_type).eq_ignore_ascii_case(k))
            .map_or(0, |(_, v)| v);
        if role != 0 {
            let i = index.parts.binary_search_by(|q| super::ascii_cmp(&q.key, &r.target)).map_err(|_| Error::MissingTarget)?;
            roles[i] |= role;
        }
    }
    let mut header = 0;
    for (i, part) in index.parts.iter().enumerate() {
        let q = part.part;
        if q.content_type.eq_ignore_ascii_case(super::super::CT_VML) || roles[i] & 16 != 0 {
            if roles[i] & !16 != 0 {
                return Err(Error::Xml);
            }
            header = header.max(vml(q, &part.rels, false)?.reserved);
            continue;
        }
        if !xml_bearing(q) && roles[i] == 0 {
            continue;
        }
        let chart = visited[i] & (Allowance::Chart as u8) != 0 && (roles[i] & 12 != 0 || q.content_type.contains("chart"));
        let mut formulas: Vec<(usize, String)> = Vec::new();
        scan(&q.data, false, false, |path, token| {
            let name = path.last().ok_or(Error::Xml)?;
            match token {
                Token::Start(tag) => {
                    if path.len() == 1 {
                        let role = roles[i];
                        let root = match role {
                            0 => None,
                            1 => Some((MAIN, "chartsheet")),
                            2 => Some(("http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing", "wsDr")),
                            4 => Some(("http://schemas.openxmlformats.org/drawingml/2006/chart", "chartSpace")),
                            8 => Some(("http://schemas.microsoft.com/office/drawing/2014/chartex", "chartSpace")),
                            _ => return Err(Error::Xml),
                        };
                        if root.is_some_and(|(ns, n)| !is(name, ns, n)) {
                            return Err(Error::Xml);
                        }
                    }
                    references(&tag, name, &part.rels)?;
                    if chart && name.local == "f" {
                        push(&mut formulas, (path.len(), String::new()))?;
                    }
                }
                Token::Text(s, _) if chart && name.local == "f" => {
                    if let Some((_, out)) = formulas.last_mut() {
                        formula_text(out, s, true)?;
                    }
                }
                Token::Reference(c) if chart && name.local == "f" => {
                    if let Some((_, out)) = formulas.last_mut() {
                        formula_text(out, c.encode_utf8(&mut [0; 4]), false)?;
                    }
                }
                Token::End(_) if chart && name.local == "f" => {
                    if let Some((depth, out)) = formulas.pop() {
                        if depth != path.len() {
                            return Err(Error::Xml);
                        }
                        sheet_formula(wb, &out)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(header)
}
