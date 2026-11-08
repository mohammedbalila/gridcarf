use super::{AuditStats, Context, Kind, RecoveryError, RecoveryLimits, charge, item, malformed, string_bytes};
use super::{
    pictures::{self, Coordinates, Role},
    scratch::{Keys, Requests},
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Debug, Default)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(super) struct StructuralStats {
    pub(super) observed: AuditStats,
    pub(super) requests: Requests,
    pub(super) key_peak: usize,
    pub(super) key_comparisons: usize,
    pub(super) binary_encoded: usize,
    pub(super) binary_decoded: usize,
    pub(super) pictures: usize,
}
struct Audit<'a> {
    limits: &'a RecoveryLimits,
    stats: StructuralStats,
    keys: Keys,
    refusal: Option<RecoveryError>,
}
impl Audit<'_> {
    fn checked<T, E: de::Error>(&mut self, result: Result<T, RecoveryError>) -> Result<T, E> {
        result.map_err(|error| {
            if self.refusal.is_none() {
                self.refusal = Some(error);
            }
            E::custom("recovery structural refusal")
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Null,
    Other,
    String,
    Unsigned,
    Map,
    Seq,
}
enum Observed {
    Other,
    Object,
    Coordinate(gridcraft_core::CellRef),
    Unsigned(u32),
}
struct Seed<'a, 'b> {
    audit: &'a mut Audit<'b>,
    context: Context,
    depth: usize,
    role: Role,
    // SeqAccess invokes the seed only for a real element, never for the end probe.
    parent_item: Option<(Context, usize)>,
}
impl<'b> Seed<'_, 'b> {
    fn start<E: de::Error>(&mut self, shape: Shape) -> Result<(), E> {
        let result = (|| {
            let stats = &mut self.audit.stats.observed;
            let limits = self.audit.limits;
            charge(&mut stats.nodes, 1, limits.nodes, "nodes")?;
            if matches!(self.context, Context::List(_)) && shape != Shape::Seq {
                return Err(malformed("manifest_shape"));
            }
            match self.context {
                Context::One(Kind::NativePath) if !matches!(shape, Shape::Map | Shape::Null) => return Err(malformed("native_path")),
                Context::One(Kind::Base64) if !matches!(shape, Shape::String | Shape::Null) => return Err(malformed("base64")),
                Context::One(Kind::U8 | Kind::U16 | Kind::Group) if shape != Shape::Unsigned => return Err(malformed("unsigned_scalar")),
                _ => {}
            }
            let valid = match self.role {
                Role::Pair => shape == Shape::Seq,
                Role::Coordinate | Role::Object => shape == Shape::Map,
                Role::Row | Role::Col => shape == Shape::Unsigned,
                Role::Plain => true,
            };
            if !valid {
                return Err(malformed("picture_coordinate"));
            }
            if self.context == Context::One(Kind::Part) || (self.context == Context::One(Kind::OptionalPart) && shape != Shape::Null) {
                charge(&mut stats.parts, 1, limits.parts, "parts")?;
            }
            if matches!(shape, Shape::Map | Shape::Seq) {
                if self.depth >= limits.depth.min(64) {
                    return Err(RecoveryError::TooLarge("depth"));
                }
                stats.max_depth = stats.max_depth.max(self.depth + 1);
            }
            Ok(())
        })();
        self.audit.checked(result)
    }
    fn child(&mut self, context: Context, role: Role, parent_item: Option<(Context, usize)>) -> Seed<'_, 'b> {
        Seed { audit: self.audit, context, depth: self.depth + 1, role, parent_item }
    }
    fn scalar<E: de::Error>(mut self, shape: Shape) -> Result<Observed, E> {
        self.start(shape)?;
        Ok(Observed::Other)
    }
}
impl<'de> DeserializeSeed<'de> for Seed<'_, '_> {
    type Value = Observed;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Observed, D::Error> {
        if let Some((context, count)) = self.parent_item {
            let result = item(context, count, &mut self.audit.stats.observed, self.audit.limits);
            self.audit.checked(result)?;
        }
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_, '_> {
    type Value = Observed;
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("JSON value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Observed, E> {
        self.scalar(Shape::Null)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Observed, E> {
        self.scalar(Shape::Other)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Observed, E> {
        self.scalar(Shape::Other)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Observed, E> {
        self.scalar(Shape::Other)
    }
    fn visit_u64<E: de::Error>(mut self, value: u64) -> Result<Observed, E> {
        self.start(Shape::Unsigned)?;
        let bound = match self.context {
            Context::One(Kind::U8) => 255,
            Context::One(Kind::U16) => 65535,
            Context::One(Kind::Group) => u32::MAX as u64,
            _ => u64::MAX,
        };
        let result = if value > bound {
            Err(malformed("unsigned_scalar"))
        } else if matches!(self.role, Role::Row | Role::Col) {
            u32::try_from(value).map(Observed::Unsigned).map_err(|_| malformed("picture_coordinate"))
        } else {
            Ok(Observed::Other)
        };
        self.audit.checked(result)
    }
    fn visit_str<E: de::Error>(mut self, value: &str) -> Result<Observed, E> {
        self.start(Shape::String)?;
        let result = (|| {
            string_bytes(&mut 0, value.len(), self.context, &mut self.audit.stats.observed, self.audit.limits)?;
            self.audit.stats.observed.max_string = self.audit.stats.observed.max_string.max(value.len());
            if self.context == Context::One(Kind::Base64) {
                let decoded = base64(value)?;
                charge(&mut self.audit.stats.binary_encoded, value.len(), self.audit.limits.manifest, "base64")?;
                charge(&mut self.audit.stats.binary_decoded, decoded, self.audit.limits.envelope, "base64")?;
            }
            Ok(Observed::Other)
        })();
        self.audit.checked(result)
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Observed, E> {
        self.visit_str(&value)
    }
    fn visit_map<M: MapAccess<'de>>(mut self, mut map: M) -> Result<Observed, M::Error> {
        self.start(Shape::Map)?;
        let mark = self.audit.keys.mark();
        let result = (|| {
            let (mut count, mut row, mut col) = (0, None, None);
            while let Some((context, role)) = map.next_key_seed(Key { audit: self.audit, context: self.context, role: self.role })? {
                count += 1;
                let result = item(self.context, count, &mut self.audit.stats.observed, self.audit.limits);
                self.audit.checked(result)?;
                let observed = map.next_value_seed(self.child(context, role, None))?;
                match (role, observed) {
                    (Role::Row, Observed::Unsigned(v)) => row = Some(v),
                    (Role::Col, Observed::Unsigned(v)) => col = Some(v),
                    _ => {}
                }
            }
            let result = self.audit.keys.check(mark, self.audit.limits, &mut self.audit.stats.requests);
            self.audit.checked(result)?;
            if self.context == Context::One(Kind::NativePath) && count != 1 {
                return self.audit.checked(Err(malformed("native_path")));
            }
            if self.role == Role::Coordinate {
                return self.audit.checked(pictures::coordinate(row, col)).map(Observed::Coordinate);
            }
            Ok(Observed::Object)
        })();
        self.audit.keys.release(mark); // Unwind this scope on both parser and structural refusals.
        result
    }
    fn visit_seq<S: SeqAccess<'de>>(mut self, mut seq: S) -> Result<Observed, S::Error> {
        self.start(Shape::Seq)?;
        let picture_map = self.context == Context::List(Kind::PicturePair);
        let mut coordinates = Coordinates::default();
        let result = (|| {
            let (mut count, mut coordinate) = (0, None);
            loop {
                let context = self.context.element(count);
                let role = if picture_map { Role::Pair } else { self.role.element(count) };
                let parent = (self.context, count + 1);
                let Some(value) = seq.next_element_seed(self.child(context, role, Some(parent)))? else { break };
                if let Observed::Coordinate(value) = value {
                    if picture_map {
                        let result = coordinates.push(value, self.audit.limits, &mut self.audit.stats.requests);
                        self.audit.checked(result)?;
                        self.audit.stats.pictures += 1;
                    } else {
                        coordinate = Some(value);
                    }
                }
                count += 1;
            }
            if self.role == Role::Pair {
                if count != 2 {
                    return self.audit.checked(Err(malformed("picture_pair")));
                }
                return self.audit.checked(coordinate.ok_or_else(|| malformed("picture_coordinate"))).map(Observed::Coordinate);
            }
            let result = coordinates.check(self.audit.limits, &mut self.audit.stats.requests);
            self.audit.checked(result)?;
            Ok(Observed::Other)
        })();
        coordinates.release(&mut self.audit.stats.requests);
        result
    }
}
struct Key<'a, 'b> {
    audit: &'a mut Audit<'b>,
    context: Context,
    role: Role,
}
impl<'de> DeserializeSeed<'de> for Key<'_, '_> {
    type Value = (Context, Role);
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_, '_> {
    type Value = (Context, Role);
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("JSON key")
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let result = (|| {
            let stats = &mut self.audit.stats.observed;
            charge(&mut stats.nodes, 1, self.audit.limits.nodes, "nodes")?;
            charge(&mut stats.keys, 1, self.audit.limits.nodes, "nodes")?;
            string_bytes(&mut 0, value.len(), Context::GENERAL, stats, self.audit.limits)?;
            stats.max_string = stats.max_string.max(value.len());
            let context = self.context.field(value);
            if self.context == Context::One(Kind::NativePath) && context == Context::GENERAL {
                return Err(malformed("native_path"));
            }
            // serde may reuse the decoded string buffer at the very next token.
            self.audit.keys.push(value, self.audit.limits, &mut self.audit.stats.requests)?;
            Ok((context, self.role.field(value)))
        })();
        self.audit.checked(result)
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        self.visit_str(&value)
    }
}

fn base64(text: &str) -> Result<usize, RecoveryError> {
    let invalid = || RecoveryError::UnsupportedRepresentation("base64");
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(invalid());
    }
    let sextet = |byte| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut decoded = 0usize;
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        sextet(chunk[0]).ok_or_else(invalid)?;
        let second = sextet(chunk[1]).ok_or_else(invalid)?;
        let amount = if chunk[2] == b'=' {
            if chunk[3] != b'=' || second & 15 != 0 {
                return Err(invalid());
            }
            1
        } else {
            let third = sextet(chunk[2]).ok_or_else(invalid)?;
            if chunk[3] == b'=' {
                if third & 3 != 0 {
                    return Err(invalid());
                }
                2
            } else {
                sextet(chunk[3]).ok_or_else(invalid)?;
                3
            }
        };
        if amount != 3 && index != bytes.len() / 4 - 1 {
            return Err(invalid());
        }
        decoded = decoded.checked_add(amount).ok_or(RecoveryError::TooLarge("base64"))?;
    }
    Ok(decoded)
}

pub(super) fn audit(bytes: &[u8], limits: &RecoveryLimits) -> Result<StructuralStats, RecoveryError> {
    let mut audit = Audit { limits, stats: StructuralStats::default(), keys: Keys::default(), refusal: None };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = Seed { audit: &mut audit, context: Context::ROOT, depth: 0, role: Role::Plain, parent_item: None }
        .deserialize(&mut deserializer)
        .and_then(|_| deserializer.end());
    audit.stats.key_peak = audit.keys.peak;
    audit.stats.key_comparisons = audit.keys.comparisons;
    audit.keys.close(&mut audit.stats.requests);
    if let Some(error) = audit.refusal {
        return Err(error);
    }
    parsed.map_err(|_| malformed("manifest_syntax"))?;
    Ok(audit.stats)
}

#[cfg(test)]
mod tests;
