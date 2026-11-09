//! Recovery-only projection: delegate Workbook serde, replacing only its shared atomic scalar.
use gridcraft_model::Workbook;
use serde::{Serialize, Serializer, ser::SerializeStruct};
pub(super) struct FrozenWorkbook<'a> {
    pub(super) workbook: &'a Workbook,
    pub(super) active_sheet: usize,
}
impl Serialize for FrozenWorkbook<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[cfg(test)]
        super::CAPTURE_WORK.with(|work| {
            let (metadata, serialization) = work.get();
            work.set((metadata, serialization + 1));
        });
        self.workbook.serialize(Freeze { serializer, active_sheet: self.active_sheet })
    }
}
struct Freeze<S> {
    serializer: S,
    active_sheet: usize,
}
struct Fields<S> {
    fields: S,
    active_sheet: usize,
}
impl<S: SerializeStruct> SerializeStruct for Fields<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
        if key == "active_sheet" { self.fields.serialize_field(key, &self.active_sheet) } else { self.fields.serialize_field(key, value) }
    }
    fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
        self.fields.skip_field(key)
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.fields.end()
    }
}
impl<S: Serializer> Serializer for Freeze<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = S::SerializeSeq;
    type SerializeTuple = S::SerializeTuple;
    type SerializeTupleStruct = S::SerializeTupleStruct;
    type SerializeTupleVariant = S::SerializeTupleVariant;
    type SerializeMap = S::SerializeMap;
    type SerializeStructVariant = S::SerializeStructVariant;
    type SerializeStruct = Fields<S::SerializeStruct>;
    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_bool(value)
    }
    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_i8(value)
    }
    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_i16(value)
    }
    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_i32(value)
    }
    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_i64(value)
    }
    fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_i128(value)
    }
    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_u8(value)
    }
    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_u16(value)
    }
    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_u32(value)
    }
    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_u64(value)
    }
    fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_u128(value)
    }
    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_f32(value)
    }
    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_f64(value)
    }
    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_char(value)
    }
    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_str(value)
    }
    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_bytes(value)
    }
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_none()
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_unit()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_some(value)
    }
    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_unit_struct(name)
    }
    fn serialize_unit_variant(self, name: &'static str, index: u32, variant: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_unit_variant(name, index, variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, name: &'static str, value: &T) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_newtype_struct(name, value)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_newtype_variant(name, index, variant, value)
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.serializer.serialize_seq(len)
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serializer.serialize_tuple(len)
    }
    fn serialize_tuple_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serializer.serialize_tuple_struct(name, len)
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.serializer.serialize_tuple_variant(name, index, variant, len)
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.serializer.serialize_map(len)
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.serializer.serialize_struct_variant(name, index, variant, len)
    }
    fn serialize_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(Fields { fields: self.serializer.serialize_struct(name, len)?, active_sheet: self.active_sheet })
    }
    fn is_human_readable(&self) -> bool {
        self.serializer.is_human_readable()
    }
}
