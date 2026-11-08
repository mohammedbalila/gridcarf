use super::{Context, Error, Serialize, Serializer, State, charge, fmt, malformed, string_bytes, unsupported};
use serde::ser::Impossible;

pub(super) struct Key<'a, 'b> {
    pub(super) state: &'a mut State<'b>,
    pub(super) context: Context,
}
impl Key<'_, '_> {
    fn text(self, text: &str) -> Result<Context, Error> {
        charge(&mut self.state.stats.nodes, 1, self.state.limits.nodes, "nodes").map_err(Error)?;
        charge(&mut self.state.stats.keys, 1, self.state.limits.nodes, "nodes").map_err(Error)?;
        string_bytes(&mut 0, text.len(), Context::GENERAL, &mut self.state.stats, self.state.limits).map_err(Error)?;
        self.state.stats.max_string = self.state.stats.max_string.max(text.len());
        Ok(self.context.field(text))
    }
    fn integer(self, mut value: u128, negative: bool) -> Result<Context, Error> {
        let mut buffer = [0u8; 40];
        let mut start = buffer.len();
        loop {
            start -= 1;
            buffer[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        if negative {
            start -= 1;
            buffer[start] = b'-';
        }
        self.text(std::str::from_utf8(&buffer[start..]).map_err(|_| malformed("integer_key"))?)
    }
}
macro_rules! integers {
    (signed $($method:ident:$ty:ty),* $(,)?)=>{$(fn $method(self,value:$ty)->Result<Context,Error>{self.integer((value as i128).unsigned_abs(),value<0)})*};
    (unsigned $($method:ident:$ty:ty),* $(,)?)=>{$(fn $method(self,value:$ty)->Result<Context,Error>{self.integer(value as u128,false)})*};
}
impl Serializer for Key<'_, '_> {
    type Ok = Context;
    type Error = Error;
    type SerializeSeq = Impossible<Context, Error>;
    type SerializeTuple = Impossible<Context, Error>;
    type SerializeTupleStruct = Impossible<Context, Error>;
    type SerializeTupleVariant = Impossible<Context, Error>;
    type SerializeMap = Impossible<Context, Error>;
    type SerializeStruct = Impossible<Context, Error>;
    type SerializeStructVariant = Impossible<Context, Error>;
    integers!(signed serialize_i8:i8,serialize_i16:i16,serialize_i32:i32,serialize_i64:i64,serialize_i128:i128);
    integers!(unsigned serialize_u8:u8,serialize_u16:u16,serialize_u32:u32,serialize_u64:u64,serialize_u128:u128);
    fn serialize_str(self, value: &str) -> Result<Context, Error> {
        self.text(value)
    }
    fn serialize_char(self, value: char) -> Result<Context, Error> {
        self.text(value.encode_utf8(&mut [0; 4]))
    }
    fn serialize_bool(self, value: bool) -> Result<Context, Error> {
        self.text(if value { "true" } else { "false" })
    }
    fn serialize_f32(self, _: f32) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_f64(self, _: f64) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_none(self) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Context, Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<Context, Error> {
        self.text(variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<Context, Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, _: &'static str, _: &T) -> Result<Context, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeTupleVariant, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(unsupported("map_key"))
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeStructVariant, Error> {
        Err(unsupported("map_key"))
    }
    fn collect_str<T: fmt::Display + ?Sized>(self, _: &T) -> Result<Context, Error> {
        Err(unsupported("collect_str"))
    }
}
