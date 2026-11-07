//! Bounded, namespace-aware tokens and byte patches for the preserved-reference handlers.

use std::collections::HashSet;
use std::ops::Range;

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, PrefixDeclaration, ResolveResult};

use super::{Checked, MAIN, require};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Name {
    pub ns: String,
    pub local: String,
}

#[derive(Clone)]
pub(super) struct Field {
    pub value: String,
    pub span: Range<usize>,
}

pub(super) struct Tag {
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

pub(super) enum Token {
    Start(Tag),
    Text(Field),
    End,
}

pub(super) fn path_is(path: &[Name], expected: &[(&str, &str)]) -> bool {
    path.len() == expected.len() && path.iter().zip(expected).all(|(n, (ns, local))| n.ns == *ns && n.local == *local)
}

fn namespace(value: ResolveResult<'_>) -> Checked<String> {
    match value {
        ResolveResult::Bound(ns) => Ok(std::str::from_utf8(ns.as_ref()).map_err(|_| super::unsupported())?.to_string()),
        ResolveResult::Unbound => Ok(String::new()),
        ResolveResult::Unknown(_) => Err(super::unsupported()),
    }
}

/// Locate quoted values only inside a start tag already validated by quick-xml. The caller
/// cross-checks each key/value against its parsed attribute before a span can be patched.
fn attribute_spans(input: &[u8], start: usize, end: usize, name_len: usize) -> Checked<Vec<(Range<usize>, Range<usize>)>> {
    let mut at = start.checked_add(1 + name_len).ok_or_else(super::unsupported)?;
    let mut spans = Vec::new();
    while at < end {
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if matches!(input.get(at), Some(b'/' | b'>')) {
            break;
        }
        let key_start = at;
        while input.get(at).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'=') {
            at += 1;
        }
        let key_end = at;
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        require(input.get(at) == Some(&b'='))?;
        at += 1;
        while input.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let quote = *input.get(at).ok_or_else(super::unsupported)?;
        require(matches!(quote, b'\'' | b'"'))?;
        at += 1;
        let from = at;
        while at < end && input.get(at) != Some(&quote) {
            at += 1;
        }
        require(at < end && spans.len() < 128 && at - from <= 8192 && key_end - key_start <= 1024)?;
        spans.push((key_start..key_end, from..at));
        at += 1;
    }
    Ok(spans)
}

/// Streaming validation; handlers retain only their small identity/reference records, not XML trees.
pub(super) fn scan(input: &[u8], fragment: bool, mut visit: impl FnMut(&[Name], Token) -> Checked<()>) -> Checked<()> {
    let cap = if fragment { super::super::MAX_FRAGMENT } else { super::super::MAX_KEPT_PART as usize };
    require(input.len() <= cap)?;
    let text = std::str::from_utf8(input).map_err(|_| super::unsupported())?;
    let mut reader = NsReader::from_str(text);
    if fragment {
        // Sheet fragments inherit the writer's worksheet default namespace. An explicit xmlns
        // on the fragment still overrides this, so a spoofed or reset namespace is not accepted.
        reader.resolver_mut().add(PrefixDeclaration::Default, Namespace(MAIN.as_bytes())).map_err(|_| super::unsupported())?;
    }
    let mut path = Vec::new();
    let mut roots = 0;
    let mut count = 0usize;
    loop {
        count += 1;
        require(count <= crate::xml::MAX_XML_NODES)?;
        let start = usize::try_from(reader.buffer_position()).map_err(|_| super::unsupported())?;
        let event = reader.read_event().map_err(|_| super::unsupported())?;
        let end = usize::try_from(reader.buffer_position()).map_err(|_| super::unsupported())?;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = input.get(end.saturating_sub(2)) == Some(&b'/');
                if path.is_empty() {
                    roots += 1;
                    require(roots == 1)?;
                }
                require(path.len() < super::super::MAX_FRAGMENT_DEPTH && e.name().as_ref().len() <= 1024)?;
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let name = Name { ns: namespace(ns)?, local: std::str::from_utf8(local.as_ref()).map_err(|_| super::unsupported())?.into() };
                let spans = attribute_spans(input, start, end, e.name().as_ref().len())?;
                let mut attrs = Vec::new();
                let mut unique = HashSet::new();
                let mut n = 0;
                for item in e.attributes() {
                    let a = item.map_err(|_| super::unsupported())?;
                    let (key, span) = spans.get(n).ok_or_else(super::unsupported)?;
                    n += 1;
                    require(input.get(key.clone()) == Some(a.key.as_ref()) && input.get(span.clone()) == Some(a.value.as_ref()))?;
                    if a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:") {
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    let name = Name { ns: namespace(ns)?, local: std::str::from_utf8(local.as_ref()).map_err(|_| super::unsupported())?.into() };
                    require(unique.insert((name.ns.clone(), name.local.clone())))?;
                    let raw = std::str::from_utf8(a.value.as_ref()).map_err(|_| super::unsupported())?;
                    let value = quick_xml::escape::unescape(raw).map_err(|_| super::unsupported())?.into_owned();
                    attrs.push((name, Field { value, span: span.clone() }));
                }
                require(n == spans.len())?;
                path.push(name);
                visit(&path, Token::Start(Tag { attrs }))?;
                if empty {
                    visit(&path, Token::End)?;
                    path.pop();
                }
            }
            Event::End(_) => {
                require(!path.is_empty())?;
                visit(&path, Token::End)?;
                path.pop();
            }
            Event::Text(e) => {
                let value = std::str::from_utf8(e.as_ref()).map_err(|_| super::unsupported())?;
                if path.is_empty() {
                    require(value.trim().is_empty())?;
                } else {
                    visit(&path, Token::Text(Field { value: value.into(), span: start..end }))?;
                }
            }
            Event::Decl(e) => {
                require(roots == 0)?;
                if let Some(encoding) = e.encoding() {
                    require(encoding.map_err(|_| super::unsupported())?.eq_ignore_ascii_case(b"utf-8"))?;
                }
            }
            Event::Comment(_) => {}
            Event::Eof => {
                require(path.is_empty() && roots == 1)?;
                break;
            }
            _ => return Err(super::unsupported()),
        }
    }
    Ok(())
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
