//! Budgets the actual borrowed JSON shape; existing custom serializers still allocate.
mod fences;
mod key;
use super::{
    RecoveryError, RecoveryLimits,
    context::{Context, Kind},
    preflight::{self, AuditStats, charge, item, string_bytes},
};
use serde::{Serialize, Serializer, ser};
use std::fmt;

#[derive(Debug)]
struct Error(RecoveryError);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("recovery serializer refusal")
    }
}
impl std::error::Error for Error {}
impl ser::Error for Error {
    fn custom<T: fmt::Display>(_: T) -> Self {
        malformed("serializer")
    }
}
fn malformed(reason: &'static str) -> Error {
    Error(RecoveryError::Malformed(reason))
}
fn unsupported(reason: &'static str) -> Error {
    Error(RecoveryError::UnsupportedRepresentation(reason))
}

struct State<'a> {
    limits: &'a RecoveryLimits,
    stats: AuditStats,
    pending: Option<&'a [serde_json::Value]>,
}
pub(super) fn precheck(workbook: &gridcraft_model::Workbook, limits: &RecoveryLimits) -> Result<(), RecoveryError> {
    fences::check(workbook, limits).map(|_| ())
}
#[cfg(test)]
pub(super) fn observe<T: Serialize + ?Sized>(value: &T, limits: &RecoveryLimits) -> Result<AuditStats, RecoveryError> {
    with_pending(value, limits, None)
}
pub(super) fn with_pending<T: Serialize + ?Sized>(
    value: &T,
    limits: &RecoveryLimits,
    pending: Option<&[serde_json::Value]>,
) -> Result<AuditStats, RecoveryError> {
    let mut state = State { limits, stats: AuditStats::default(), pending };
    value.serialize(Observer { state: &mut state, context: Context::ROOT, depth: 0 }).map_err(|error| error.0)?;
    Ok(state.stats)
}
struct Observer<'a, 'b> {
    state: &'a mut State<'b>,
    context: Context,
    depth: usize,
}
impl<'a, 'b> Observer<'a, 'b> {
    fn node(&mut self, null: bool) -> Result<(), Error> {
        charge(&mut self.state.stats.nodes, 1, self.state.limits.nodes, "nodes").map_err(Error)?;
        if self.context == Context::One(Kind::Part) || (self.context == Context::One(Kind::OptionalPart) && !null) {
            charge(&mut self.state.stats.parts, 1, self.state.limits.parts, "parts").map_err(Error)?;
        }
        Ok(())
    }
    fn scalar(mut self, null: bool) -> Result<(), Error> {
        self.node(null)
    }
    fn begin(mut self, object: bool, length: Option<usize>) -> Result<Compound<'a, 'b>, Error> {
        self.node(false)?;
        if self.depth >= self.state.limits.depth.min(64) {
            return Err(Error(RecoveryError::TooLarge("depth")));
        }
        self.state.stats.max_depth = self.state.stats.max_depth.max(self.depth + 1);
        let minimum = length.unwrap_or(0).checked_mul(if object { 2 } else { 1 }).ok_or(Error(RecoveryError::TooLarge("nodes")))?;
        available(self.state, minimum)?;
        Ok(Compound { state: self.state, context: self.context, depth: self.depth + 1, count: 0, expected: length, key: None })
    }
    fn variant(self, variant: &str) -> Result<Self, Error> {
        let mut outer = self.begin(true, Some(1))?;
        outer.map_key(variant)?;
        let context = outer.key.take().ok_or_else(|| malformed("serializer_key"))?;
        item(outer.context, 1, &mut outer.state.stats, outer.state.limits).map_err(Error)?;
        available(outer.state, 1)?;
        Ok(Self { state: outer.state, context, depth: outer.depth })
    }
}
fn available(state: &State<'_>, minimum: usize) -> Result<(), Error> {
    if state.stats.nodes.checked_add(minimum).is_none_or(|n| n > state.limits.nodes) {
        return Err(Error(RecoveryError::TooLarge("nodes")));
    }
    Ok(())
}
macro_rules! scalar_methods {
    ($($name:ident:$ty:ty),* $(,)?) => {$(fn $name(self,_:$ty)->Result<(),Error>{self.scalar(false)})*};
}
impl<'a, 'b> Serializer for Observer<'a, 'b> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'a, 'b>;
    type SerializeTuple = Compound<'a, 'b>;
    type SerializeTupleStruct = Compound<'a, 'b>;
    type SerializeTupleVariant = Compound<'a, 'b>;
    type SerializeMap = Compound<'a, 'b>;
    type SerializeStruct = Compound<'a, 'b>;
    type SerializeStructVariant = Compound<'a, 'b>;
    scalar_methods!(serialize_bool:bool,serialize_i8:i8,serialize_i16:i16,serialize_i32:i32,serialize_i64:i64,serialize_i128:i128,
        serialize_u8:u8,serialize_u16:u16,serialize_u32:u32,serialize_u64:u64,serialize_u128:u128);
    fn serialize_f32(self, value: f32) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(unsupported("nonfinite"));
        }
        self.scalar(false)
    }
    fn serialize_f64(self, value: f64) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(unsupported("nonfinite"));
        }
        self.scalar(false)
    }
    fn serialize_char(self, value: char) -> Result<(), Error> {
        self.serialize_str(value.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(mut self, value: &str) -> Result<(), Error> {
        self.node(false)?;
        string_bytes(&mut 0, value.len(), self.context, &mut self.state.stats, self.state.limits).map_err(Error)?;
        self.state.stats.max_string = self.state.stats.max_string.max(value.len());
        Ok(())
    }
    fn serialize_bytes(self, value: &[u8]) -> Result<(), Error> {
        let mut array = self.begin(false, Some(value.len()))?;
        for byte in value {
            array.element(byte)?;
        }
        array.finish()
    }
    fn serialize_none(self) -> Result<(), Error> {
        self.scalar(true)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        self.scalar(true)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        self.scalar(true)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<(), Error> {
        self.serialize_str(variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, variant: &'static str, value: &T) -> Result<(), Error> {
        value.serialize(self.variant(variant)?)
    }
    fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        self.begin(false, length)
    }
    fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Error> {
        self.begin(false, Some(length))
    }
    fn serialize_tuple_struct(self, _: &'static str, length: usize) -> Result<Self::SerializeTupleStruct, Error> {
        self.begin(false, Some(length))
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, variant: &'static str, length: usize) -> Result<Self::SerializeTupleVariant, Error> {
        self.variant(variant)?.begin(false, Some(length))
    }
    fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Error> {
        self.begin(true, length)
    }
    fn serialize_struct(self, _: &'static str, length: usize) -> Result<Self::SerializeStruct, Error> {
        self.begin(true, Some(length))
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, variant: &'static str, length: usize) -> Result<Self::SerializeStructVariant, Error> {
        self.variant(variant)?.begin(true, Some(length))
    }
    fn collect_str<T: fmt::Display + ?Sized>(self, _: &T) -> Result<(), Error> {
        Err(unsupported("collect_str"))
    }
}

struct Compound<'a, 'b> {
    state: &'a mut State<'b>,
    context: Context,
    depth: usize,
    count: usize,
    expected: Option<usize>,
    key: Option<Context>,
}
impl Compound<'_, '_> {
    fn next(&mut self) -> Result<(), Error> {
        if self.expected.is_some_and(|n| self.count >= n) {
            return Err(malformed("serializer_length"));
        }
        available(self.state, 1)?;
        item(self.context, self.count + 1, &mut self.state.stats, self.state.limits).map_err(Error)?;
        self.count += 1;
        Ok(())
    }
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let context = self.context.element(self.count);
        self.next()?;
        value.serialize(Observer { state: self.state, context, depth: self.depth })
    }
    fn map_key<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        if self.key.is_some() {
            return Err(malformed("serializer_key"));
        }
        if self.expected.is_some_and(|n| self.count >= n) {
            return Err(malformed("serializer_length"));
        }
        available(self.state, 2)?;
        self.key = Some(value.serialize(key::Key { state: self.state, context: self.context })?);
        Ok(())
    }
    fn map_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let context = self.key.take().ok_or_else(|| malformed("serializer_key"))?;
        self.next()?;
        value.serialize(Observer { state: self.state, context, depth: self.depth })
    }
    fn finish(mut self) -> Result<(), Error> {
        if self.key.is_some() {
            return Err(malformed("serializer_key"));
        }
        if self.expected.is_some_and(|n| n != self.count) {
            return Err(malformed("serializer_length"));
        }
        if self.context == Context::One(Kind::Workbook)
            && let Some(pending) = self.state.pending.take()
        {
            // Pending is an extra field in this actual Workbook object, not a second root.
            self.expected = None;
            self.map_key("macros")?;
            self.map_value(pending)?;
        }
        Ok(())
    }
}
impl ser::SerializeSeq for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.element(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeTuple for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.element(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeTupleStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.element(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeTupleVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.element(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeMap for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.map_key(value)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.map_value(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        self.map_key(key)?;
        self.map_value(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        self.map_key(key)?;
        self.map_value(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

#[cfg(test)]
mod tests;
