//! Support for serializing JSON structures.
//!
//! # Map representations
//!
//! Certain types, such as [`BTreeMap`](https://doc.rust-lang.org/stable/std/collections/struct.BTreeMap.html)
//! and [`heapless::linear_map::LinearMap`](https://docs.rs/heapless/latest/heapless/linear_map/type.LinearMap.html)
//! serialize themselves as `serde` "map" types.
//!
//! In JSON, these are two possible ways map types can be represented:
//!
//! - As JSON objects, e.g. `{ "a": 1, "b": 2 }`
//!   - This is what `serde_json` does.
//!   - When deserializing using JavaScript's `JSON.parse`, you get a JavaScript Object, which has easy key-lookup syntax.
//!   - Keys must be strings. Certain types, such as numeric types and booleans can easily be stringified, but maps with compound types for keys are rejected.
//! - As JSON array of pairs, e.g. `[ [ "a", 1 ], [ "b", 2 ] ]`
//!   - Keys can be any type.
//!   - When deserializing using JavaScript's `JSON.parse`, you get an Array of Array's, which is harder to work with. You can convert it to a Map using `new Map(pairs)`, but if it's in a structure this is trickier.
//!
//! ## Choosing between representations
//!
//! By default, map types are serialized as JSON objects (this is different from older versions of `picoserve`).
//! The serializer can be configured with which form maps should take. Note that configuration is passed down structures,
//! so a map of maps will have the same representation (unless locally overridden).
//!
//! If you would like to choose a map representation, this can be done in the following:
//!
//! ### [`SerializeMapAsObject`] and [`SerializeMapAsArrayOfPairs`]
//!
//! These structs have a single field, and when serialized, configure the serializer.
//! These are designed to be used as wrappers around values at the point of passing or returning a [`Json`] value.
//!
//! ### [`Json::serialize_map_as_object`] and [`Json::serialize_map_as_array_of_pairs`]
//!
//! These functions are convenience methods to wrap the value with [`SerializeMapAsObject`] or [`SerializeMapAsArrayOfPairs`].
//!
//! ### [`serialize_map_as_object`] and [`serialize_map_as_array_of_pairs`]
//!
//! These functions are designed to be used with the `#[serde(serialize_with = "..."]` field annotations when deriving `serde::Serialize` on a custom type.

use core::{fmt, future::Future};

use serde::Serialize;

use crate::io::Write;

pub use crate::json::Json;

#[derive(Debug, thiserror::Error)]
enum SerializeError {
    #[error("{0}")]
    Format(#[from] fmt::Error),
    #[error("Failed to serialize value")]
    SerdeError,
}

impl serde::ser::Error for SerializeError {
    fn custom<T: fmt::Display>(_msg: T) -> Self {
        Self::SerdeError
    }
}

fn object_key_must_be_a_string() -> SerializeError {
    log_warn!("JSON object keys must be a string");

    SerializeError::SerdeError
}

struct Escaped<T>(T);

impl<W: fmt::Write> fmt::Write for Escaped<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.0.write_str(match c {
                '\x08' => "\\b",
                '\x09' => "\\t",
                '\x0A' => "\\n",
                '\x0C' => "\\f",
                '\x0D' => "\\r",
                '"' => "\\\"",
                '/' => "\\/",
                '\\' => "\\\\",
                c if c < ' ' => {
                    write!(self.0, "\\u{:04x}", c as u32)?;
                    continue;
                }
                c => {
                    self.0.write_char(c)?;
                    continue;
                }
            })?;
        }

        Ok(())
    }
}

impl<T: fmt::Display> fmt::Display for Escaped<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        write!(Escaped(f), "{}", self.0)
    }
}

struct EscapedString<T: fmt::Display>(T);

impl<T: fmt::Display> fmt::Display for EscapedString<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", Escaped(&self.0))
    }
}

const SERIALIZE_MAP_AS_OBJECT: &str = "___SERIALIZE_MAP_AS_OBJECT___";
const SERIALIZE_MAP_AS_ARRAY_OF_PAIRS: &str = "___SERIALIZE_MAP_AS_ARRAY_OF_PAIRS___";

/// Serialize `T`, configuring the serializer to serialize map types as JSON objects.
/// Designed to be used as `#[serde(serialize_with = "serialize_map_as_object")]`
pub fn serialize_map_as_object<T: serde::Serialize, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_newtype_struct(SERIALIZE_MAP_AS_OBJECT, value)
}

/// Configure the serializer to serialize map types as JSON objects.
pub struct SerializeMapAsObject<T: serde::Serialize>(pub T);

impl<T: serde::Serialize> serde::Serialize for SerializeMapAsObject<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_map_as_object(&self.0, serializer)
    }
}

/// Serialize `T`, configuring the serializer to serialize map types as JSON Arrays.
/// Designed to be used as `#[serde(serialize_with = "serialize_map_as_array_of_pairs")]`
pub fn serialize_map_as_array_of_pairs<T: serde::Serialize, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_newtype_struct(SERIALIZE_MAP_AS_ARRAY_OF_PAIRS, value)
}

/// Configure the serializer to serialize map types as JSON Arrays.
pub struct SerializeMapAsArrayOfPairs<T>(pub T);

impl<T: serde::Serialize> serde::Serialize for SerializeMapAsArrayOfPairs<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_map_as_array_of_pairs(&self.0, serializer)
    }
}

impl<T: serde::Serialize> Json<T> {
    pub fn serialize_map_as_object(self) -> Json<SerializeMapAsObject<T>> {
        Json(SerializeMapAsObject(self.0))
    }

    pub fn serialize_map_as_array_of_pairs(self) -> Json<SerializeMapAsArrayOfPairs<T>> {
        Json(SerializeMapAsArrayOfPairs(self.0))
    }
}

#[derive(Clone, Default)]
enum SerializeMapAs {
    #[default]
    Object,
    ArrayOfPairs,
}

struct Serializer<'a, W: fmt::Write> {
    serialize_map_as: SerializeMapAs,
    writer: &'a mut W,
}

impl<'a, W: fmt::Write> Serializer<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            serialize_map_as: SerializeMapAs::default(),
            writer,
        }
    }
}

impl<'a, W: fmt::Write> Serializer<'a, W> {
    fn reborrow(&mut self) -> Serializer<'_, W> {
        Serializer {
            serialize_map_as: self.serialize_map_as.clone(),
            writer: self.writer,
        }
    }

    fn write_str(&mut self, s: &str) -> Result<(), SerializeError> {
        Ok(self.writer.write_str(s)?)
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), SerializeError> {
        Ok(self.writer.write_fmt(args)?)
    }

    fn serialize_compound(self, _len: impl Into<Option<usize>>) -> SerializeCompound<'a, W> {
        SerializeCompound {
            serializer: self,
            is_first: true,
        }
    }
}

macro_rules! serialize_display {
    ($($f:ident $t:ty)*) => {
        $(
            fn $f(mut self, v: $t) -> Result<Self::Ok, Self::Error> {
                write!(self, "{}", v)
            }
        )*
    };
}

impl<'a, W: fmt::Write> serde::Serializer for Serializer<'a, W> {
    type Ok = ();
    type Error = SerializeError;

    type SerializeSeq = SerializeCompound<'a, W>;
    type SerializeTuple = SerializeCompound<'a, W>;
    type SerializeTupleStruct = SerializeCompound<'a, W>;
    type SerializeTupleVariant = SerializeCompound<'a, W>;
    type SerializeMap = SerializeCompound<'a, W>;
    type SerializeStruct = SerializeCompound<'a, W>;
    type SerializeStructVariant = SerializeCompound<'a, W>;

    fn serialize_bool(mut self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.write_str(if v { "true" } else { "false" })
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(v.into())
    }

    fn serialize_f64(mut self, v: f64) -> Result<Self::Ok, Self::Error> {
        match v.classify() {
            core::num::FpCategory::Nan | core::num::FpCategory::Infinite => self.serialize_none(),
            core::num::FpCategory::Zero
            | core::num::FpCategory::Subnormal
            | core::num::FpCategory::Normal => {
                let mut buffer = ryu::Buffer::new();
                self.write_str(buffer.format_finite(v))
            }
        }
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(value.encode_utf8(&mut [0; 4]))
    }

    fn serialize_str(self, s: &str) -> Result<Self::Ok, Self::Error> {
        self.collect_str(s)
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        v.serialize(self)
    }

    fn serialize_none(mut self) -> Result<Self::Ok, Self::Error> {
        self.write_str("null")
    }

    fn serialize_some<T: serde::Serialize + ?Sized>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.serialize_none()
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
        mut self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.serialize_map_as = match name {
            SERIALIZE_MAP_AS_OBJECT => SerializeMapAs::Object,
            SERIALIZE_MAP_AS_ARRAY_OF_PAIRS => SerializeMapAs::ArrayOfPairs,
            _ => self.serialize_map_as,
        };

        value.serialize(self)
    }

    fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        write!(self, "{{{}:", EscapedString(variant))?;
        value.serialize(self.reborrow())?;
        write!(self, "}}")?;

        Ok(())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(self.serialize_compound(len))
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(self.serialize_compound(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(self.serialize_compound(len))
    }

    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        write!(self, "{{{}:", EscapedString(variant))?;
        Ok(self.serialize_compound(len))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(self.serialize_compound(len))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(self.serialize_compound(len))
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.serialize_tuple_variant(name, variant_index, variant, len)
    }

    fn collect_str<T: fmt::Display + ?Sized>(mut self, value: &T) -> Result<Self::Ok, Self::Error> {
        write!(self, "{}", EscapedString(value))
    }

    serialize_display!(
        serialize_i8 i8 serialize_i16 i16 serialize_i32 i32 serialize_i64 i64
        serialize_u8 u8 serialize_u16 u16 serialize_u32 u32 serialize_u64 u64
    );
}

macro_rules! serialize_forward_to_serializer {
    ($($f:ident $t:ty)*) => {
        $(
            fn $f(self, v: $t) -> Result<Self::Ok, Self::Error> {
                v.serialize(self.serializer)
            }
        )*
    };
}

macro_rules! serialize_collect_str {
    ($($f:ident $t:ty)*) => {
        $(
            fn $f(self, v: $t) -> Result<Self::Ok, Self::Error> {
                self.collect_str(&format_args!("{}", v))
            }
        )*
    };
}

struct SerializeObjectKey<'a, W: fmt::Write> {
    serializer: Serializer<'a, W>,
}

impl<'a, W: fmt::Write> serde::Serializer for SerializeObjectKey<'a, W> {
    type Ok = ();
    type Error = SerializeError;

    type SerializeSeq = serde::ser::Impossible<(), SerializeError>;
    type SerializeTuple = serde::ser::Impossible<(), SerializeError>;
    type SerializeTupleStruct = serde::ser::Impossible<(), SerializeError>;
    type SerializeTupleVariant = serde::ser::Impossible<(), SerializeError>;
    type SerializeMap = serde::ser::Impossible<(), SerializeError>;
    type SerializeStruct = serde::ser::Impossible<(), SerializeError>;
    type SerializeStructVariant = serde::ser::Impossible<(), SerializeError>;

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.serializer.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(object_key_must_be_a_string())
    }

    fn collect_str<T: fmt::Display + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.serializer.collect_str(value)
    }

    serialize_forward_to_serializer! {
        serialize_bool bool
        serialize_f32 f32 serialize_f64 f64
        serialize_char char
        serialize_str &str
    }

    serialize_collect_str!(
        serialize_i8 i8 serialize_i16 i16 serialize_i32 i32 serialize_i64 i64
        serialize_u8 u8 serialize_u16 u16 serialize_u32 u32 serialize_u64 u64
    );
}

struct SerializeCompound<'a, W: fmt::Write> {
    serializer: Serializer<'a, W>,
    is_first: bool,
}

impl<W: fmt::Write> serde::ser::SerializeSeq for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_element<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.serializer
            .write_str(if self.is_first { "[" } else { "," })?;

        self.is_first = false;

        value.serialize(self.serializer.reborrow())?;

        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(if self.is_first { "[]" } else { "]" })
    }
}

impl<W: fmt::Write> serde::ser::SerializeTuple for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_element<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl<W: fmt::Write> serde::ser::SerializeTupleStruct for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl<W: fmt::Write> serde::ser::SerializeTupleVariant for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(if self.is_first { "[]}" } else { "]}" })
    }
}

impl<W: fmt::Write> serde::ser::SerializeMap for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_key<T: serde::Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.serializer.write_str(if self.is_first {
            match self.serializer.serialize_map_as {
                SerializeMapAs::Object => "{",
                SerializeMapAs::ArrayOfPairs => "[",
            }
        } else {
            ","
        })?;

        self.is_first = false;

        match self.serializer.serialize_map_as {
            SerializeMapAs::Object => key.serialize(SerializeObjectKey {
                serializer: self.serializer.reborrow(),
            }),
            SerializeMapAs::ArrayOfPairs => {
                self.serializer.write_str("[")?;

                key.serialize(self.serializer.reborrow())
            }
        }
    }

    fn serialize_value<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        match self.serializer.serialize_map_as {
            SerializeMapAs::Object => {
                self.serializer.write_str(":")?;
                value.serialize(self.serializer.reborrow())
            }
            SerializeMapAs::ArrayOfPairs => {
                self.serializer.write_str(",")?;
                value.serialize(self.serializer.reborrow())?;
                self.serializer.write_str("]")
            }
        }
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(match self.serializer.serialize_map_as {
                SerializeMapAs::Object => {
                    if self.is_first {
                        "{}"
                    } else {
                        "}"
                    }
                }
                SerializeMapAs::ArrayOfPairs => {
                    if self.is_first {
                        "[]"
                    } else {
                        "]"
                    }
                }
            })
    }
}

impl<W: fmt::Write> serde::ser::SerializeStruct for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.serializer
            .write_str(if self.is_first { "{" } else { "," })?;

        self.is_first = false;

        write!(self.serializer, "{}:", EscapedString(key))?;

        value.serialize(self.serializer.reborrow())?;

        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(if self.is_first { "{}" } else { "}" })
    }
}

impl<W: fmt::Write> serde::ser::SerializeStructVariant for SerializeCompound<'_, W> {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        serde::ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(if self.is_first { "{}}" } else { "}}" })
    }
}

impl<T: serde::Serialize> Json<T> {
    pub(crate) fn display(self) -> impl core::fmt::Display {
        core::fmt::from_fn(move |f| {
            self.0
                .serialize(Serializer::new(f))
                .or_else(|error| match error {
                    SerializeError::Format(error) => Err(error),
                    // Ignore serde errors as it's too late to report this sensibly.
                    SerializeError::SerdeError => Ok(()),
                })
        })
    }

    pub(crate) async fn write_to<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        write!(writer, "{}", Json(self.0).display()).await
    }
}

struct JsonBody<T>(T);

impl<T: serde::Serialize> super::Content for JsonBody<T> {
    fn content_type(&self) -> &'static str {
        "application/json"
    }

    fn content_length(&self) -> usize {
        let mut content_length = 0;
        self.0
            .serialize(Serializer::new(&mut super::MeasureFormatSize(
                &mut content_length,
            )))
            .map_or(0, |()| content_length)
    }

    fn write_content<W: Write>(self, writer: W) -> impl Future<Output = Result<(), W::Error>> {
        Json(self.0).write_to(writer)
    }
}

impl<T: serde::Serialize> Json<T> {
    /// Convert JSON payload into a [`Response`](super::Response) with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        super::Response::ok(JsonBody(self.0))
    }
}

impl<T: serde::Serialize> super::IntoResponse for Json<T> {
    async fn write_to<R: crate::io::Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        response_writer
            .write_response(connection, self.into_response())
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::{string::ToString, vec::Vec};

    use serde_json::Value;

    #[derive(strum::VariantArray)]
    enum JsonValueType {
        Null,
        Bool,
        Number,
        String,
        Array,
        Object,
    }

    impl crate::tests::fuzz::TestValue<usize> for Value {
        fn generate(test_data: &mut crate::tests::fuzz::TestData, fuel: usize) -> Self {
            let Some(fuel) = fuel.checked_sub(1) else {
                return Self::Null;
            };

            fn generate_array(
                test_data: &mut crate::tests::fuzz::TestData,
                fuel: usize,
            ) -> Vec<Value> {
                let length = test_data.generate_value_with_parameter::<usize, _>(0..=(fuel / 2));

                std::iter::from_fn(|| Some(test_data.generate_value_with_parameter(fuel / length)))
                    .take(length)
                    .collect()
            }

            match test_data.choose_value(strum::VariantArray::VARIANTS) {
                JsonValueType::Null => Value::Null,
                JsonValueType::Bool => Value::Bool(test_data.generate_value()),
                JsonValueType::Number => Value::Number(serde_json::value::Number::from(
                    test_data.generate_value::<u32>(),
                )),
                JsonValueType::String => Value::String(test_data.generate_string(0..100)),
                JsonValueType::Array => Value::Array(generate_array(test_data, fuel)),
                JsonValueType::Object => Value::Object(
                    generate_array(test_data, fuel)
                        .into_iter()
                        .map(|value| (test_data.generate_string(0..100), value))
                        .collect(),
                ),
            }
        }
    }

    fn verify_json(input: &impl serde::Serialize, expected: &Value) {
        let mut buffer = std::string::String::new();

        input
            .serialize(super::Serializer::new(&mut buffer))
            .unwrap();

        let actual = serde_json::from_str::<Value>(&buffer).unwrap();

        assert_eq!(&actual, expected)
    }

    #[tokio::test]
    async fn common_json_is_serialized_correctly() {
        crate::tests::fuzz::run_async("json_is_serialized_correctly", async |test_data| {
            let value = test_data.generate_value_with_parameter::<Value, _>(100);

            verify_json(&value, &value);
        })
        .await
    }

    #[test]
    fn struct_is_serialized_correctly() {
        #[derive(serde::Serialize)]
        struct TestStruct {
            a: i32,
            b: bool,
        }

        let a = 42;
        let b = true;

        verify_json(&TestStruct { a, b }, &serde_json::json!({ "a": a, "b": b }));
    }

    #[test]
    fn serialize_map_as_array_of_pairs() {
        verify_json(
            &super::SerializeMapAsArrayOfPairs(std::collections::BTreeMap::from([
                ((1, 2), (3, 4)),
                ((5, 6), (7, 8)),
            ])),
            &serde_json::json!([[[1, 2], [3, 4]], [[5, 6], [7, 8]]]),
        );

        #[derive(serde::Serialize)]
        struct TestStruct {
            #[serde(serialize_with = "super::serialize_map_as_array_of_pairs")]
            map: std::collections::BTreeMap<(i32, i32), i32>,
        }

        verify_json(
            &TestStruct {
                map: [((1, 2), 3), ((4, 5), 6)].into(),
            },
            &serde_json::json!({ "map" : [[[1, 2], 3], [[4, 5], 6]] }),
        );
    }

    #[test]
    fn serialize_map_as_both() {
        #[derive(serde::Serialize)]
        struct TestStruct(
            #[serde(serialize_with = "super::serialize_map_as_object")]
            std::collections::BTreeMap<i32, i32>,
        );

        verify_json(
            &super::SerializeMapAsArrayOfPairs(std::collections::BTreeMap::from([
                (1, TestStruct([(2, 3), (4, 5)].into())),
                (6, TestStruct([(7, 8)].into())),
            ])),
            &serde_json::json!([[ 1, { "2": 3, "4": 5 } ], [ 6, { "7": 8 } ]]),
        );
    }

    #[tokio::test]
    async fn json_is_serialized_without_newlines() {
        crate::tests::fuzz::run_async("json_is_serialized_without_newlines", async |test_data| {
            let json_value = super::Json(test_data.generate_value_with_parameter::<Value, _>(100))
                .display()
                .to_string();

            if json_value.contains('\n') {
                panic!("JSON values must not contain '\n'");
            }
        })
        .await
    }
}
