//! Bounded, namespace-aware tokens and byte patches for the preserved-reference handlers.

use std::ops::Range;

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, PrefixDeclaration, ResolveResult};

use super::{Checked, MAIN, require};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub ns: String,
    pub local: String,
}

#[derive(Clone)]
pub(crate) struct Field {
    pub value: String,
    pub span: Range<usize>,
}

pub(crate) struct Tag {
    pub span: Range<usize>,
    pub attrs: Vec<(Name, Field)>,
}

impl Tag {
    pub fn field(&self, ns: &str, name: &str) -> Option<&Field> {
        self.attrs.iter().find(|(n, _)| n.ns == ns && n.local == name).map(|(_, f)| f)
    }

    pub fn only(&self, names: &[(&str, &str)]) -> Checked<()> {
        require(self.attrs.iter().all(|(n, _)| names.iter().any(|(ns, local)| n.ns == *ns && n.local == *local)))
    }

    pub fn value(&self, name: &str) -> Checked<&str> {
        self.field("", name).map(|f| f.value.as_str()).ok_or_else(super::unsupported)
    }
}

pub(crate) enum Token {
    Start(Tag),
    Text(Field),
    End,
}

pub(super) fn path_is(path: &[Name], expected: &[(&str, &str)]) -> bool {
    path.len() == expected.len() && path.iter().zip(expected).all(|(n, (ns, local))| n.ns == *ns && n.local == *local)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Malformed,
    Unsupported,
    Limit,
    Allocation,
}

type Scanned<T> = Result<T, Failure>;
pub(crate) enum RecoveryToken<'a> {
    Start(Tag),
    Text(&'a str, Range<usize>),
    Reference(char),
    End(Range<usize>),
}

fn copied(s: &str) -> Scanned<String> {
    let mut out = String::new();
    out.try_reserve_exact(s.len()).map_err(|_| Failure::Allocation)?;
    out.push_str(s);
    Ok(out)
}

fn pushed<T>(v: &mut Vec<T>, value: T) -> Scanned<()> {
    v.try_reserve(1).map_err(|_| Failure::Allocation)?;
    v.push(value);
    Ok(())
}

fn namespace(value: ResolveResult<'_>) -> Scanned<String> {
    match value {
        ResolveResult::Bound(ns) => copied(std::str::from_utf8(ns.as_ref()).map_err(|_| Failure::Malformed)?),
        ResolveResult::Unbound => Ok(String::new()),
        ResolveResult::Unknown(_) => Err(Failure::Malformed),
    }
}

fn scalar(c: char) -> bool {
    matches!(c as u32, 9 | 10 | 13 | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}
fn legal(s: &str) -> Scanned<()> {
    if s.chars().all(scalar) { Ok(()) } else { Err(Failure::Malformed) }
}
fn ncname(s: &str) -> bool {
    let start = |c: char| matches!(c as u32, 0x41..=0x5A | 0x5F | 0x61..=0x7A | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x2FF | 0x370..=0x37D | 0x37F..=0x1FFF | 0x200C..=0x200D | 0x2070..=0x218F | 0x2C00..=0x2FEF | 0x3001..=0xD7FF | 0xF900..=0xFDCF | 0xFDF0..=0xFFFD | 0x10000..=0xEFFFF);
    let mut chars = s.chars();
    chars.next().is_some_and(start)
        && chars.all(|c| start(c) || matches!(c as u32, 0x2D | 0x2E | 0x30..=0x39 | 0xB7 | 0x300..=0x36F | 0x203F..=0x2040))
}
fn qname(s: &str) -> bool {
    s.split_once(':').map_or_else(|| ncname(s), |(a, b)| ncname(a) && ncname(b))
}
fn reference(raw: &str) -> Scanned<char> {
    match raw {
        "amp" => Ok('&'),
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "quot" => Ok('"'),
        "apos" => Ok('\''),
        _ => {
            let n = raw.strip_prefix('#').ok_or(Failure::Malformed)?;
            let (digits, radix) = n.strip_prefix('x').map_or((n, 10), |n| (n, 16));
            if digits.starts_with(['+', '-']) {
                return Err(Failure::Malformed);
            }
            let n = u32::from_str_radix(digits, radix).map_err(|_| Failure::Malformed)?;
            if n == 0 {
                return Err(Failure::Malformed);
            }
            char::from_u32(n).ok_or(Failure::Malformed)
        }
    }
}
fn xml_space(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}
fn attribute_text(out: &mut String, s: &str, recovery: bool) {
    if !recovery {
        out.push_str(s);
        return;
    }
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
}
fn encoding_name(s: &[u8]) -> bool {
    s.first().is_some_and(u8::is_ascii_alphabetic) && s.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn declaration(input: &[u8], spans: &Spans) -> Scanned<()> {
    let mut encoding = false;
    let mut standalone = false;
    for (i, (key, span)) in spans.iter().enumerate() {
        let key = &input[key.clone()];
        let value = &input[span.clone()];
        match key {
            b"version" if i == 0 => {
                if value != b"1.0" {
                    if value.starts_with(b"1.") && value.len() > 2 && value[2..].iter().all(u8::is_ascii_digit) {
                        return Err(Failure::Unsupported);
                    }
                    return Err(Failure::Malformed);
                }
            }
            b"encoding" if i != 0 && !encoding && !standalone => {
                if !encoding_name(value) {
                    return Err(Failure::Malformed);
                }
                encoding = true;
            }
            b"standalone" if i != 0 && !standalone => {
                if !matches!(value, b"yes" | b"no") {
                    return Err(Failure::Malformed);
                }
                standalone = true;
            }
            _ => return Err(Failure::Malformed),
        }
    }
    if spans.is_empty() {
        return Err(Failure::Malformed);
    }
    Ok(())
}

fn value(raw: &str, recovery: bool) -> Scanned<String> {
    // Entity replacement cannot expand this UTF-8 spelling. Legacy S05 retains its raw
    // whitespace and quick-xml unescape policy; recovery normalizes literal attribute whitespace
    // and additionally checks XML1.0 scalars.
    if recovery {
        legal(raw)?;
        if raw.contains('<') {
            return Err(Failure::Malformed);
        }
    }
    let mut out = String::new();
    out.try_reserve_exact(raw.len()).map_err(|_| Failure::Allocation)?;
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        attribute_text(&mut out, &rest[..at], recovery);
        let body = &rest[at + 1..];
        let end = body.find(';').ok_or(Failure::Malformed)?;
        let c = reference(&body[..end])?;
        if recovery && !scalar(c) {
            return Err(Failure::Malformed);
        }
        out.push(c);
        rest = &body[end + 1..];
    }
    attribute_text(&mut out, rest, recovery);
    Ok(out)
}

type Spans = Vec<(Range<usize>, Range<usize>)>;
/// The existing span lexer now runs before NsReader can copy namespace declarations.
/// It bounds each name/value/count first; quick-xml remains the XML parser and cross-check.
fn start_spans(input: &[u8], start: usize, recovery: bool) -> Scanned<Option<(usize, usize, Spans)>> {
    let decl = recovery
        && input.get(start..).is_some_and(|s| s.starts_with(b"<?xml"))
        && input.get(start + 5).is_some_and(|b| b.is_ascii_whitespace() || *b == b'?');
    if input.get(start) != Some(&b'<') || input.get(start + 1).is_none_or(|b| matches!(b, b'/' | b'!') || (*b == b'?' && !decl)) {
        return Ok(None);
    }
    let name_start = start + 1 + usize::from(decl);
    let mut at = name_start;
    while input.get(at).is_some_and(|b| !(b.is_ascii_whitespace() || matches!(b, b'/' | b'>') || decl && *b == b'?')) {
        at += 1;
        if at - name_start > 1024 {
            return Err(Failure::Limit);
        }
    }
    let len = at - name_start;
    let mut spans = Vec::new();
    loop {
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        match input.get(at) {
            Some(b'>') => return Ok(Some((len, at + 1, spans))),
            Some(b'?') if decl && input.get(at + 1) == Some(&b'>') => return Ok(Some((len, at + 2, spans))),
            Some(b'/') if input.get(at + 1) == Some(&b'>') => return Ok(Some((len, at + 2, spans))),
            None => return Err(Failure::Malformed),
            _ => {}
        }
        if spans.len() == 128 {
            return Err(Failure::Limit);
        }
        let key = at;
        while input.get(at).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'=') {
            at += 1;
            if at - key > 1024 {
                return Err(Failure::Limit);
            }
        }
        let key_end = at;
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if input.get(at) != Some(&b'=') {
            return Err(Failure::Malformed);
        }
        at += 1;
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let quote = *input.get(at).ok_or(Failure::Malformed)?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(Failure::Malformed);
        }
        at += 1;
        let from = at;
        while input.get(at).is_some_and(|b| *b != quote) {
            at += 1;
            if at - from > 8192 {
                return Err(Failure::Limit);
            }
        }
        if input.get(at).is_none() {
            return Err(Failure::Malformed);
        }
        pushed(&mut spans, (key..key_end, from..at))?;
        at += 1;
    }
}

/// One namespace-aware scanner serves edit and recovery handlers. quick-xml0.41 internally
/// grows its open-name/namespace buffers infallibly. Input/depth/start-tag material is bounded
/// before those operations; app-owned path/attribute/entity/prefix storage is fallible.
fn scan_inner<'i>(
    input: &'i [u8],
    fragment: bool,
    recovery: bool,
    remap: bool,
    mut visit: impl FnMut(&[Name], RecoveryToken<'i>) -> Scanned<()>,
) -> Scanned<()> {
    let cap = if fragment { super::super::MAX_FRAGMENT } else { super::super::MAX_KEPT_PART as usize };
    if input.len() > cap {
        return Err(Failure::Limit);
    }
    if recovery
        && (input.starts_with(&[0xff, 0xfe]) || input.starts_with(&[0xfe, 0xff]) || input.starts_with(&[b'<', 0]) || input.starts_with(&[0, b'<']))
    {
        return Err(Failure::Unsupported);
    }
    if !recovery {
        std::str::from_utf8(input).map_err(|_| Failure::Malformed)?;
    }
    let mut reader = NsReader::from_reader(input);
    reader.resolver_mut().set_max_declarations_per_element(128);
    reader.config_mut().check_comments = recovery;
    if fragment {
        reader.resolver_mut().add(PrefixDeclaration::Default, Namespace(MAIN.as_bytes())).map_err(|_| Failure::Malformed)?;
    }
    let mut path = Vec::new();
    let mut roots = 0;
    let mut count = 0usize;
    let mut declared = false;
    loop {
        count += 1;
        if count > crate::xml::MAX_XML_NODES {
            return Err(Failure::Limit);
        }
        let mut start = usize::try_from(reader.buffer_position()).map_err(|_| Failure::Limit)?;
        if start == 0 && input.starts_with(&[0xef, 0xbb, 0xbf]) {
            if recovery {
                start = 3;
            } else if !input[3..].starts_with(b"<?xml") {
                return Err(Failure::Malformed);
            }
        }
        let spans = start_spans(input, start, recovery)?;
        if spans.is_some() && path.len() >= super::super::MAX_FRAGMENT_DEPTH {
            return Err(Failure::Limit);
        }
        let event = reader.read_event().map_err(|_| Failure::Malformed)?;
        let end = usize::try_from(reader.buffer_position()).map_err(|_| Failure::Limit)?;
        // Inspect a declaration before default UTF-8 decoding: an excluded declared encoding
        // refuses honestly even when later bytes are not UTF-8. No transcoding occurs.
        if let Event::Decl(e) = &event
            && let Some(encoding) = e.encoding()
        {
            let encoding = encoding.map_err(|_| Failure::Malformed)?;
            if recovery && !encoding_name(&encoding) {
                return Err(Failure::Malformed);
            }
            if !encoding.eq_ignore_ascii_case(b"utf-8") {
                return Err(Failure::Unsupported);
            }
        }
        let raw = std::str::from_utf8(input.get(start..end).ok_or(Failure::Malformed)?).map_err(|_| Failure::Malformed)?;
        if recovery {
            legal(raw)?;
        }
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = input.get(end.saturating_sub(2)) == Some(&b'/');
                if path.is_empty() {
                    roots += 1;
                    if roots != 1 {
                        return Err(Failure::Malformed);
                    }
                }
                let (len, expected, spans) = spans.ok_or(Failure::Malformed)?;
                if expected != end || len != e.name().as_ref().len() {
                    return Err(Failure::Malformed);
                }
                let tag_name = e.name();
                let lexical = std::str::from_utf8(tag_name.as_ref()).map_err(|_| Failure::Malformed)?;
                if recovery && !qname(lexical) {
                    return Err(Failure::Malformed);
                }
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let name = Name { ns: namespace(ns)?, local: copied(std::str::from_utf8(local.as_ref()).map_err(|_| Failure::Malformed)?)? };
                if recovery && name.ns == "http://www.w3.org/2000/xmlns/" {
                    return Err(Failure::Malformed);
                }
                let mut attrs: Vec<(Name, Field)> = Vec::new();
                let mut n = 0;
                for item in e.attributes() {
                    let a = item.map_err(|_| Failure::Malformed)?;
                    let (key, span) = spans.get(n).ok_or(Failure::Malformed)?;
                    n += 1;
                    if input.get(key.clone()) != Some(a.key.as_ref()) || input.get(span.clone()) != Some(a.value.as_ref()) {
                        return Err(Failure::Malformed);
                    }
                    let lexical = std::str::from_utf8(&input[key.clone()]).map_err(|_| Failure::Malformed)?;
                    let raw = std::str::from_utf8(&input[span.clone()]).map_err(|_| Failure::Malformed)?;
                    if recovery && !qname(lexical) {
                        return Err(Failure::Malformed);
                    }
                    if lexical == "xmlns" || lexical.starts_with("xmlns:") {
                        if recovery {
                            if remap && fragment && raw.contains('"') {
                                return Err(Failure::Unsupported);
                            }
                            legal(raw)?;
                            if raw.contains('<') {
                                return Err(Failure::Malformed);
                            }
                            if raw.contains(['\n', '\r', '\t']) {
                                return Err(Failure::Unsupported);
                            }
                            if raw.contains('&') {
                                value(raw, true)?;
                                return Err(Failure::Unsupported);
                            }
                            if lexical.starts_with("xmlns:") {
                                if raw.is_empty() {
                                    return Err(Failure::Malformed);
                                }
                            } else if matches!(raw, "http://www.w3.org/XML/1998/namespace" | "http://www.w3.org/2000/xmlns/") {
                                return Err(Failure::Malformed);
                            }
                        }
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    let name = Name { ns: namespace(ns)?, local: copied(std::str::from_utf8(local.as_ref()).map_err(|_| Failure::Malformed)?)? };
                    if attrs.iter().any(|(n, _)| *n == name) {
                        return Err(Failure::Malformed);
                    }
                    let decoded = value(raw, recovery)?;
                    if recovery && remap && fragment && name.ns == super::REL && (raw != decoded || raw.contains(['\n', '\r', '\t'])) {
                        return Err(Failure::Unsupported);
                    }
                    if recovery && remap && fragment && name.ns == "http://purl.oclc.org/ooxml/officeDocument/relationships" {
                        return Err(Failure::Unsupported);
                    }
                    pushed(&mut attrs, (name, Field { value: decoded, span: span.clone() }))?;
                }
                if n != spans.len() {
                    return Err(Failure::Malformed);
                }
                pushed(&mut path, name)?;
                visit(&path, RecoveryToken::Start(Tag { span: start..end, attrs }))?;
                if empty {
                    visit(&path, RecoveryToken::End(start..end))?;
                    path.pop();
                }
            }
            Event::End(_) => {
                if path.is_empty() {
                    return Err(Failure::Malformed);
                }
                visit(&path, RecoveryToken::End(start..end))?;
                path.pop();
            }
            Event::Text(_) => {
                if recovery && raw.contains("]]>") {
                    return Err(Failure::Malformed);
                }
                if path.is_empty() {
                    if if recovery { !xml_space(raw) } else { !raw.trim().is_empty() } {
                        return Err(Failure::Malformed);
                    }
                } else {
                    visit(&path, RecoveryToken::Text(raw, start..end))?;
                }
            }
            Event::GeneralRef(e) if recovery => {
                if path.is_empty() {
                    return Err(Failure::Malformed);
                }
                let c = reference(std::str::from_utf8(e.as_ref()).map_err(|_| Failure::Malformed)?)?;
                if !scalar(c) {
                    return Err(Failure::Malformed);
                }
                visit(&path, RecoveryToken::Reference(c))?;
            }
            Event::Decl(_) => {
                if roots != 0 || (recovery && (declared || !(start == 0 || (start == 3 && input.starts_with(&[0xef, 0xbb, 0xbf]))))) {
                    return Err(Failure::Malformed);
                }
                if recovery {
                    let (_, expected, attrs) = spans.as_ref().ok_or(Failure::Malformed)?;
                    if *expected != end {
                        return Err(Failure::Malformed);
                    }
                    declaration(input, attrs)?;
                }
                declared = true;
            }
            Event::Comment(_) => {}
            Event::PI(e) if recovery && e.target().eq_ignore_ascii_case(b"xml") => return Err(Failure::Malformed),
            Event::CData(_) | Event::PI(_) | Event::DocType(_) if recovery => return Err(Failure::Unsupported),
            Event::Eof => {
                if !path.is_empty() || roots != 1 {
                    return Err(Failure::Malformed);
                }
                return Ok(());
            }
            _ => return Err(Failure::Unsupported),
        }
    }
}

/// S05 keeps its original String refusal, callback errors, raw text and byte spans.
pub(crate) fn scan(input: &[u8], fragment: bool, mut visit: impl FnMut(&[Name], Token) -> Checked<()>) -> Checked<()> {
    let mut callback = None;
    scan_inner(input, fragment, false, false, |path, token| {
        let token = match token {
            RecoveryToken::Start(t) => Token::Start(t),
            RecoveryToken::End(_) => Token::End,
            RecoveryToken::Text(text, span) => Token::Text(Field { value: copied(text)?, span }),
            RecoveryToken::Reference(_) => return Err(Failure::Unsupported),
        };
        visit(path, token).map_err(|e| {
            callback = Some(e);
            Failure::Malformed
        })
    })
    .map_err(|_| callback.unwrap_or_else(super::unsupported))
}

pub(crate) fn scan_recovery<'i>(
    input: &'i [u8],
    fragment: bool,
    remap: bool,
    visit: impl FnMut(&[Name], RecoveryToken<'i>) -> Scanned<()>,
) -> Scanned<()> {
    scan_inner(input, fragment, true, remap, visit)
}

pub(super) struct Patch {
    pub span: Range<usize>,
    pub value: String,
}

pub(super) fn patch(input: &[u8], mut patches: Vec<Patch>) -> Checked<Option<Vec<u8>>> {
    if patches.is_empty() {
        return Ok(None);
    }
    require(patches.len() <= super::super::MAX_RELS)?;
    patches.sort_by_key(|p| p.span.start);
    let mut size = input.len();
    let mut last = 0;
    for p in &patches {
        require(p.span.start >= last && p.span.end >= p.span.start && p.span.end <= input.len())?;
        size = size.checked_sub(p.span.len()).and_then(|n| n.checked_add(p.value.len())).ok_or_else(super::unsupported)?;
        require(size <= super::super::MAX_KEPT_PART as usize)?;
        last = p.span.end;
    }
    let mut out = Vec::with_capacity(size);
    last = 0;
    for p in patches {
        out.extend_from_slice(input.get(last..p.span.start).ok_or_else(super::unsupported)?);
        out.extend_from_slice(p.value.as_bytes());
        last = p.span.end;
    }
    out.extend_from_slice(input.get(last..).ok_or_else(super::unsupported)?);
    Ok(Some(out))
}
