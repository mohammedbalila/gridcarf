//! Style references used by preserved native PivotTable parts.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Reader};

use super::references::xml::{Failure, RecoveryToken, Tag, scan_recovery};
use super::{MAX_KEPT_PART, MAX_RELS};
use crate::IoError;
use crate::write::NS_MAIN;
use crate::xml::esc_attr;

const MC: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Source differential styles stay at their original indices. Only the prefix needed by the
/// native parts and table styles is kept, so generated sheet formats do not accumulate on saves.
pub(crate) struct NativeStyles {
    root_attrs: String,
    dxfs: Option<Dxfs>,
    table_styles: Option<String>,
    formats: BTreeMap<u32, String>,
}

struct Dxfs {
    count: u32,
    opening: String,
    count_span: Option<Range<usize>>,
    body: String,
    closing: String,
    main_default: bool,
}

#[derive(Default)]
struct SourceStyles {
    root: Option<Range<usize>>,
    dxfs: Option<Tag>,
    dxf_body: Range<usize>,
    dxf_count: u32,
    dxf_end: Range<usize>,
    table_styles: Option<Tag>,
    table_style_count: u32,
    table_styles_end: usize,
    formats_tag: Option<Tag>,
    formats: BTreeMap<u32, String>,
    indexed_palette: bool,
    first_indexed_dxf: Option<u32>,
    first_unmapped_dxf: Option<u32>,
    needed: u32,
}

impl NativeStyles {
    pub(crate) fn read(data: &[u8], pivot_dxf_count: u32) -> Result<Self, IoError> {
        let mut src = SourceStyles { needed: pivot_dxf_count, ..SourceStyles::default() };
        // Table styles occur after dxfs. Collect child boundaries to choose the prefix only
        // after validating both sections and all the table styles' differential-format references.
        let mut dxf_ends = Vec::new();
        scan_recovery(data, false, false, |path, token| {
            let name = path.last().ok_or(Failure::Malformed)?;
            let parent = path.iter().rev().nth(1);
            match token {
                RecoveryToken::Start(tag) => match path.len() {
                    1 => {
                        if name.ns != NS_MAIN || name.local != "styleSheet" {
                            return Err(Failure::Malformed);
                        }
                        src.root = Some(tag.span);
                    }
                    2 if name.ns == NS_MAIN => {
                        let target = match name.local.as_str() {
                            "dxfs" => &mut src.dxfs,
                            "tableStyles" => &mut src.table_styles,
                            "numFmts" => &mut src.formats_tag,
                            _ => return Ok(()),
                        };
                        if target.replace(tag).is_some() {
                            return Err(Failure::Malformed);
                        }
                    }
                    3 => match parent.filter(|p| p.ns == NS_MAIN).map(|p| p.local.as_str()) {
                        Some("dxfs") => {
                            require_name(name, "dxf")?;
                            if src.dxf_count == 0 {
                                src.dxf_body.start = tag.span.start;
                            }
                            src.dxf_count = src.dxf_count.checked_add(1).ok_or(Failure::Limit)?;
                        }
                        Some("numFmts") => {
                            require_name(name, "numFmt")?;
                            let id = id(&tag, "numFmtId")?.ok_or(Failure::Malformed)?;
                            let code = tag.value("formatCode").map_err(|_| Failure::Malformed)?;
                            if src.formats.insert(id, code.to_owned()).is_some() {
                                return Err(Failure::Malformed);
                            }
                        }
                        Some("tableStyles") => {
                            require_name(name, "tableStyle")?;
                            src.table_style_count = src.table_style_count.checked_add(1).ok_or(Failure::Limit)?;
                        }
                        Some("colors") if name.ns == NS_MAIN && name.local == "indexedColors" => {
                            src.indexed_palette = true;
                        }
                        _ => {}
                    },
                    4.. if path[1].ns == NS_MAIN => match path[1].local.as_str() {
                        "tableStyles" => {
                            if let Some(dxf) = id(&tag, "dxfId")? {
                                src.needed = src.needed.max(dxf.checked_add(1).ok_or(Failure::Limit)?);
                            }
                        }
                        "dxfs" if name.ns == NS_MAIN => match name.local.as_str() {
                            // A differential format's own code is authoritative. An ID alone
                            // could refer to a different global format after renumbering.
                            "numFmt" if path.len() == 4 && (tag.field("", "formatCode").is_none() || id(&tag, "numFmtId")?.is_none()) => {
                                src.first_unmapped_dxf.get_or_insert(src.dxf_count);
                            }
                            "color" | "fgColor" | "bgColor" if id(&tag, "indexed")?.is_some_and(|index| index < 64) => {
                                src.first_indexed_dxf.get_or_insert(src.dxf_count);
                            }
                            _ => {}
                        },
                        _ => {}
                    },
                    _ => {}
                },
                RecoveryToken::End(span) if name.ns == NS_MAIN => match (path.len(), name.local.as_str()) {
                    (3, "dxf") if parent.is_some_and(|p| p.ns == NS_MAIN && p.local == "dxfs") => {
                        dxf_ends.try_reserve(1).map_err(|_| Failure::Allocation)?;
                        dxf_ends.push(span.end);
                    }
                    (2, "dxfs") => src.dxf_end = span,
                    (2, "tableStyles") => src.table_styles_end = span.end,
                    _ => {}
                },
                RecoveryToken::Text(text, _)
                    if path.len() == 2
                        && name.ns == NS_MAIN
                        && matches!(name.local.as_str(), "dxfs" | "tableStyles" | "numFmts")
                        && !text.trim().is_empty() =>
                {
                    return Err(Failure::Malformed);
                }
                _ => {}
            }
            Ok(())
        })
        .map_err(style_scan_error)?;

        check_count(src.dxfs.as_ref(), src.dxf_count as usize)?;
        check_count(src.table_styles.as_ref(), src.table_style_count as usize)?;
        check_count(src.formats_tag.as_ref(), src.formats.len())?;
        if src.needed > src.dxf_count {
            return Err(IoError::Format("native XLSX analysis refers to a missing differential style".into()));
        }

        if src.first_unmapped_dxf.is_some_and(|first| first <= src.needed) {
            return Err(IoError::Format("native differential style has no inline number format code".into()));
        }
        if src.indexed_palette && src.first_indexed_dxf.is_some_and(|first| first <= src.needed) {
            return Err(IoError::Format("native differential style refers to a custom indexed color palette".into()));
        }

        let source = std::str::from_utf8(data).map_err(|_| invalid_styles())?;
        let root = source.get(src.root.ok_or_else(invalid_styles)?).ok_or_else(invalid_styles)?;
        let (source_default, root_attrs) = root_scope(root)?;
        let dxfs = if let Some(tag) = src.dxfs.filter(|_| src.needed != 0) {
            let open = source.get(tag.span.clone()).ok_or_else(invalid_styles)?;
            let main_default = default_namespace(open)?.as_deref().unwrap_or(&source_default) == NS_MAIN;
            let count_span = tag.field("", "count").map(|a| a.span.start - tag.span.start..a.span.end - tag.span.start);
            src.dxf_body.end = *dxf_ends.get((src.needed - 1) as usize).ok_or_else(invalid_styles)?;
            Some(Dxfs {
                count: src.needed,
                opening: with_default(open, &source_default)?,
                count_span,
                body: source.get(src.dxf_body).ok_or_else(invalid_styles)?.to_string(),
                closing: source.get(src.dxf_end).ok_or_else(invalid_styles)?.to_string(),
                main_default,
            })
        } else {
            None
        };
        let table_styles = src
            .table_styles
            .map(|tag| {
                let open = source.get(tag.span.clone()).ok_or_else(invalid_styles)?;
                let body = source.get(tag.span.end..src.table_styles_end).ok_or_else(invalid_styles)?;
                Ok::<_, IoError>(with_default(open, &source_default)? + body)
            })
            .transpose()?;
        Ok(Self { root_attrs, dxfs, table_styles, formats: src.formats })
    }

    pub(crate) fn dxf_count(&self) -> u32 {
        self.dxfs.as_ref().map_or(0, |dxfs| dxfs.count)
    }

    pub(crate) fn format_codes(&self) -> Vec<String> {
        self.formats.values().cloned().collect()
    }

    pub(crate) fn merge(&self, mut generated: String) -> Result<String, IoError> {
        // The generated stylesheet always writes these elements. User-supplied text is XML-escaped.
        let table_span = if self.table_styles.is_some() {
            let start = generated.rfind("<tableStyles ").ok_or_else(invalid_styles)?;
            let end = generated[start..].find("/>").ok_or_else(invalid_styles)? + start + 2;
            Some(start..end)
        } else {
            None
        };
        if let Some(dxfs) = &self.dxfs {
            let marker = "<dxfs count=\"";
            let start = generated.rfind(marker).ok_or_else(invalid_styles)?;
            let (count, rest) = generated[start + marker.len()..].split_once("\">").ok_or_else(invalid_styles)?;
            let (body, _) = rest.split_once("</dxfs>").ok_or_else(invalid_styles)?;
            let end = start + marker.len() + count.len() + 2 + body.len() + "</dxfs>".len();
            let total = count.parse::<u32>().ok().and_then(|n| n.checked_add(dxfs.count)).ok_or_else(style_limit)?;
            let mut merged = dxfs.opening.clone();
            if let Some(span) = &dxfs.count_span {
                merged.replace_range(span.clone(), &total.to_string());
            } else {
                merged.pop();
                let _ = write!(merged, " count=\"{total}\">");
            }
            merged.push_str(&dxfs.body);
            if dxfs.main_default {
                merged.push_str(body);
            } else {
                // The copied container can reset the default namespace. Restore it for generated DXFs.
                merged.push_str(&body.replace("<dxf>", &format!("<dxf xmlns=\"{NS_MAIN}\">")));
            }
            merged.push_str(&dxfs.closing);
            if let (Some(span), Some(styles)) = (table_span, &self.table_styles) {
                if span.start < end {
                    return Err(invalid_styles());
                }
                generated.replace_range(span, styles);
            }
            generated.replace_range(start..end, &merged);
        } else if let (Some(span), Some(styles)) = (table_span, &self.table_styles) {
            generated.replace_range(span, styles);
        }
        let root = generated.find("<styleSheet ").ok_or_else(invalid_styles)?;
        let open_end = generated[root..].find('>').ok_or_else(invalid_styles)? + root;
        generated.insert_str(open_end, &self.root_attrs);
        if generated.len() > MAX_KEPT_PART as usize {
            return Err(style_limit());
        }
        Ok(generated)
    }

    pub(crate) fn remap_pivot(&self, data: &[u8], fmt_id: &dyn Fn(&str) -> Option<u32>) -> Result<Vec<u8>, IoError> {
        let mut edits = Vec::new();
        let mut unresolved = None;
        scan_recovery(data, false, false, |path, token| {
            let RecoveryToken::Start(tag) = token else { return Ok(()) };
            let root = path.first().ok_or(Failure::Malformed)?;
            if root.ns != NS_MAIN || !matches!(root.local.as_str(), "pivotTableDefinition" | "pivotCacheDefinition" | "pivotCacheRecords") {
                return Err(Failure::Malformed);
            }
            if let Some(dxf) = id(&tag, "dxfId")?
                && dxf >= self.dxf_count()
            {
                unresolved = Some(format!("native XLSX analysis refers to missing differential style {dxf}"));
                return Err(Failure::Malformed);
            }
            if let Some(field) = tag.field("", "numFmtId") {
                let old = parse_id(&field.value)?;
                let new = self.formats.get(&old).map_or_else(|| (old < 164).then_some(old), |code| fmt_id(code));
                let Some(new) = new else {
                    unresolved = Some(format!("native XLSX analysis refers to unresolved number format {old}"));
                    return Err(Failure::Malformed);
                };
                if new != old {
                    if edits.len() >= MAX_RELS {
                        return Err(Failure::Limit);
                    }
                    edits.try_reserve(1).map_err(|_| Failure::Allocation)?;
                    edits.push((field.span.clone(), new.to_string()));
                }
            }
            Ok(())
        })
        .map_err(|err| unresolved.map_or_else(|| style_scan_error(err), IoError::Format))?;

        let size = edits.iter().try_fold(data.len(), |size, (span, value)| {
            size.checked_sub(span.len()).and_then(|n| n.checked_add(value.len())).ok_or_else(style_limit)
        })?;
        if size > MAX_KEPT_PART as usize {
            return Err(style_limit());
        }
        let mut output = Vec::new();
        output.try_reserve_exact(size).map_err(|_| style_limit())?;
        let mut pos = 0;
        for (span, value) in edits {
            output.extend_from_slice(data.get(pos..span.start).ok_or_else(invalid_styles)?);
            output.extend_from_slice(value.as_bytes());
            pos = span.end;
        }
        output.extend_from_slice(data.get(pos..).ok_or_else(invalid_styles)?);
        Ok(output)
    }
}

fn id(tag: &Tag, name: &str) -> Result<Option<u32>, Failure> {
    tag.field("", name).map(|f| parse_id(&f.value)).transpose()
}

fn parse_id(value: &str) -> Result<u32, Failure> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Failure::Malformed);
    }
    value.parse().map_err(|_| Failure::Malformed)
}

fn require_name(name: &super::references::xml::Name, local: &str) -> Result<(), Failure> {
    if name.ns == NS_MAIN && name.local == local { Ok(()) } else { Err(Failure::Malformed) }
}

fn check_count(tag: Option<&Tag>, count: usize) -> Result<(), IoError> {
    if let Some(tag) = tag
        && let Some(stated) = id(tag, "count").map_err(style_scan_error)?
        && stated as usize != count
    {
        return Err(invalid_styles());
    }
    Ok(())
}

fn root_scope(open: &str) -> Result<(String, String), IoError> {
    let mut reader = NsReader::from_str(open);
    let tag = start(reader.read_event())?;
    let mut attrs = String::new();
    let mut default = String::new();
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| invalid_styles())?;
        let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| invalid_styles())?;
        let value = std::str::from_utf8(&attr.value).map_err(|_| invalid_styles())?;
        if key == "xmlns" {
            default = value.to_string();
        } else if key.starts_with("xmlns:")
            || matches!(reader.resolver().resolve_attribute(attr.key).0, ResolveResult::Bound(ns) if ns.as_ref() == MC)
        {
            // Copy the already validated lexical value, choosing a quote it did not contain.
            let quote = if value.contains('"') { '\'' } else { '"' };
            let _ = write!(attrs, " {key}={quote}{value}{quote}");
        }
    }
    Ok((default, attrs))
}

fn default_namespace(open: &str) -> Result<Option<String>, IoError> {
    let mut reader = Reader::from_str(open);
    let tag = start(reader.read_event())?;
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| invalid_styles())?;
        if attr.key.as_ref() == b"xmlns" {
            let value = std::str::from_utf8(&attr.value).map_err(|_| invalid_styles())?;
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

fn with_default(open: &str, source_default: &str) -> Result<String, IoError> {
    if source_default == NS_MAIN || default_namespace(open)?.is_some() {
        return Ok(open.to_string());
    }
    let tail = if open.ends_with("/>") { "/>" } else { ">" };
    let head = open.strip_suffix(tail).ok_or_else(invalid_styles)?;
    Ok(format!("{head} xmlns=\"{}\"{tail}", esc_attr(source_default)))
}

fn start(event: Result<Event<'_>, quick_xml::Error>) -> Result<BytesStart<'_>, IoError> {
    match event {
        Ok(Event::Start(tag) | Event::Empty(tag)) => Ok(tag),
        _ => Err(invalid_styles()),
    }
}

fn invalid_styles() -> IoError {
    IoError::Xml("invalid XML or style metadata for native XLSX analysis".into())
}

fn style_limit() -> IoError {
    IoError::TooLarge("native XLSX analysis style metadata exceeds its budget".into())
}

fn style_scan_error(error: Failure) -> IoError {
    match error {
        Failure::Malformed | Failure::Unsupported => invalid_styles(),
        Failure::Limit | Failure::Allocation => style_limit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::write::NS_MAIN;

    fn styles(body: &str) -> String {
        format!("<styleSheet xmlns=\"{MAIN}\">{body}</styleSheet>")
    }

    #[test]
    fn native_styles_keep_referenced_prefix_and_inherited_namespaces() {
        let source = format!(
            r#"<styleSheet xmlns="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:e="urn:example" mc:Ignorable="e">
                <dxfs count="4"><dxf><font/></dxf><dxf><fill/></dxf><dxf><border e:ref="1"/></dxf><dxf><alignment/></dxf></dxfs>
                <tableStyles count="1"><tableStyle name="Custom" count="1"><tableStyleElement type="wholeTable" dxfId="2"/></tableStyle></tableStyles>
            </styleSheet>"#
        );
        let generated = styles(r#"<dxfs count="1"><dxf><protection/></dxf></dxfs><tableStyles count="0"/>"#);
        let native = NativeStyles::read(source.as_bytes(), 1).unwrap();
        assert_eq!(native.dxf_count(), 3);
        let merged = native.merge(generated.clone()).unwrap();
        assert!(
            merged.contains(r#"<dxfs count="4"><dxf><font/></dxf><dxf><fill/></dxf><dxf><border e:ref="1"/></dxf><dxf><protection/></dxf></dxfs>"#)
        );
        assert!(merged.contains(r#"<tableStyle name="Custom" count="1"><tableStyleElement type="wholeTable" dxfId="2"/></tableStyle>"#));
        assert!(merged.contains(r#"mc:Ignorable="e""#));
        assert_eq!(NativeStyles::read(merged.as_bytes(), 1).unwrap().merge(generated).unwrap(), merged);
    }

    #[test]
    fn native_pivot_formats_follow_source_declarations() {
        let source = styles(
            r#"<numFmts count="2"><numFmt numFmtId="6" formatCode="&quot;£&quot;0"/><numFmt numFmtId="201" formatCode="0.0%"/></numFmts><dxfs count="1"><dxf/></dxfs>"#,
        );
        let native = NativeStyles::read(source.as_bytes(), 1).unwrap();
        assert_eq!(native.format_codes(), ["\"£\"0", "0.0%"]);
        let pivot = format!(
            r#"<pivotTableDefinition xmlns="{MAIN}"><dataField numFmtId='6'/><dataField numFmtId="201"/><dataField numFmtId="14"/><format dxfId="0"/></pivotTableDefinition>"#
        );
        let merged = native
            .remap_pivot(pivot.as_bytes(), &|code| match code {
                "\"£\"0" => Some(165),
                "0.0%" => Some(164),
                _ => None,
            })
            .unwrap();
        assert_eq!(String::from_utf8(merged).unwrap(), pivot.replace("'6'", "'165'").replace("\"201\"", "\"164\""));
    }

    #[test]
    fn native_styles_refuse_retained_dxfs_with_unmapped_style_dependencies() {
        let no_inline_code = styles(
            r#"<numFmts count="1"><numFmt numFmtId="201" formatCode="0.0%"/></numFmts>
            <dxfs count="1"><dxf><numFmt numFmtId="201"/></dxf></dxfs>"#,
        );
        assert!(matches!(NativeStyles::read(no_inline_code.as_bytes(), 1), Err(IoError::Format(_))));
        assert!(NativeStyles::read(no_inline_code.as_bytes(), 0).is_ok());
    }

    #[test]
    fn native_styles_refuse_retained_dxfs_with_custom_indexed_colors() {
        let custom_palette = styles(
            r#"<dxfs count="1"><dxf><font><color indexed="0"/></font></dxf></dxfs>
            <colors><indexedColors><rgbColor rgb="FF123456"/></indexedColors></colors>"#,
        );
        assert!(matches!(NativeStyles::read(custom_palette.as_bytes(), 1), Err(IoError::Format(_))));
        assert!(NativeStyles::read(custom_palette.as_bytes(), 0).is_ok());
    }

    #[test]
    fn native_styles_refuse_unresolved_refs_and_malformed_xml() {
        let source = styles(r#"<dxfs count="1"><dxf/></dxfs>"#);
        assert!(NativeStyles::read(source.as_bytes(), 2).is_err());
        for invalid in [
            r#"<dxfs count="1"/>"#,
            r#"<dxfs count="1"><dxf><bad:font/></dxf></dxfs>"#,
            r#"<dxfs count="1"><dxf><font></dxf></font></dxfs>"#,
            r#"<tableStyles count="1"><tableStyle name="Custom"><tableStyleElement type="wholeTable" dxfId="0"/></tableStyle></tableStyles>"#,
            r#"<numFmts><numFmt numFmtId="164" formatCode="a"/><numFmt numFmtId="164" formatCode="b"/></numFmts>"#,
        ] {
            assert!(NativeStyles::read(styles(invalid).as_bytes(), 0).is_err(), "{invalid}");
        }
        let native = NativeStyles::read(source.as_bytes(), 1).unwrap();
        for invalid in [
            r#"<dataField numFmtId="164"/>"#,
            r#"<format dxfId="1"/>"#,
            r#"<format dxfId="-1"/>"#,
            r#"<bad:format dxfId="0"/>"#,
            r#"<format dxfId="0">"#,
        ] {
            let pivot = format!(r#"<pivotTableDefinition xmlns="{MAIN}">{invalid}</pivotTableDefinition>"#);
            assert!(native.remap_pivot(pivot.as_bytes(), &|_| None).is_err(), "{invalid}");
        }
    }
}
