//! Support for serializing JSON structures

use core::fmt;

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

struct Serializer<'a, W: fmt::Write>(&'a mut W);

impl<'a, W: fmt::Write> Serializer<'a, W> {
    fn reborrow(&mut self) -> Serializer<'_, W> {
        Serializer(self.0)
    }

    fn write_str(&mut self, s: &str) -> Result<(), SerializeError> {
        Ok(self.0.write_str(s)?)
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), SerializeError> {
        Ok(self.0.write_fmt(args)?)
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
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
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
        self.serializer
            .write_str(if self.is_first { "[" } else { "," })?;

        self.is_first = false;

        self.serializer.write_str("[")?;

        key.serialize(self.serializer.reborrow())?;

        Ok(())
    }

    fn serialize_value<T: serde::Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.serializer.write_str(",")?;
        value.serialize(self.serializer.reborrow())?;
        self.serializer.write_str("]")?;

        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        self.serializer
            .write_str(if self.is_first { "[]" } else { "]" })
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
                .serialize(Serializer(f))
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
            .serialize(Serializer(&mut super::MeasureFormatSize(
                &mut content_length,
            )))
            .map_or(0, |()| content_length)
    }

    fn write_content<W: Write>(
        self,
        writer: W,
    ) -> impl core::future::Future<Output = Result<(), W::Error>> {
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
    use std::{string::String, string::ToString, vec::Vec};

    #[derive(Debug, strum::EnumDiscriminants)]
    #[strum_discriminants(derive(strum::VariantArray))]
    enum JsonValue {
        Null,
        Bool(bool),
        Number(u64),
        String(String),
        Array(Vec<JsonValue>),
        Object(Vec<(&'static str, JsonValue)>),
    }

    impl crate::tests::fuzz::TestValue<usize> for JsonValue {
        fn generate(test_data: &mut crate::tests::fuzz::TestData, fuel: usize) -> Self {
            let Some(fuel) = fuel.checked_sub(1) else {
                return Self::Null;
            };

            fn generate_array(
                test_data: &mut crate::tests::fuzz::TestData,
                fuel: usize,
            ) -> Vec<JsonValue> {
                let length = test_data.generate_value_with_parameter::<usize, _>(0..=(fuel / 2));

                std::iter::from_fn(|| Some(test_data.generate_value_with_parameter(fuel / length)))
                    .take(length)
                    .collect()
            }

            match test_data.choose_value(strum::VariantArray::VARIANTS) {
                JsonValueDiscriminants::Null => JsonValue::Null,
                JsonValueDiscriminants::Bool => JsonValue::Bool(test_data.generate_value()),
                JsonValueDiscriminants::Number => JsonValue::Number(test_data.generate_value()),
                JsonValueDiscriminants::String => {
                    JsonValue::String(test_data.generate_string(0..100))
                }
                JsonValueDiscriminants::Array => JsonValue::Array(generate_array(test_data, fuel)),
                JsonValueDiscriminants::Object => JsonValue::Object(
                    generate_array(test_data, fuel)
                        .into_iter()
                        .map(|value| {
                            (
                                *test_data.choose_value(&["a", "b", "c", "d", "e", "f", "g", "h"]),
                                value,
                            )
                        })
                        .collect(),
                ),
            }
        }
    }

    impl From<&JsonValue> for serde_json::Value {
        fn from(value: &JsonValue) -> Self {
            match value {
                JsonValue::Null => serde_json::Value::Null,
                &JsonValue::Bool(b) => serde_json::Value::Bool(b),
                &JsonValue::Number(n) => serde_json::Value::Number(n.into()),
                JsonValue::String(s) => serde_json::Value::String(s.clone()),
                JsonValue::Array(json_values) => {
                    serde_json::Value::Array(json_values.iter().map(From::from).collect())
                }
                JsonValue::Object(items) => serde_json::Value::Object(
                    items
                        .iter()
                        .map(|&(k, ref v)| (k.into(), serde_json::Value::from(v)))
                        .collect(),
                ),
            }
        }
    }

    impl serde::Serialize for JsonValue {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match self {
                JsonValue::Null => serializer.serialize_none(),
                JsonValue::Bool(b) => b.serialize(serializer),
                JsonValue::Number(n) => n.serialize(serializer),
                JsonValue::String(s) => s.serialize(serializer),
                JsonValue::Array(a) => serializer.collect_seq(a),
                JsonValue::Object(o) => {
                    use serde::ser::SerializeStruct;

                    let mut serializer = serializer.serialize_struct("", o.len())?;

                    o.iter()
                        .try_for_each(|(k, v)| serializer.serialize_field(k, v))?;

                    serializer.end()
                }
            }
        }
    }

    #[tokio::test]
    async fn json_is_serialized_correctly() {
        crate::tests::fuzz::run_async("json_is_serialized_correctly", async |test_data| {
            let value = test_data.generate_value_with_parameter::<JsonValue, _>(100);

            let serde_json_value = serde_json::Value::from(&value);

            let parsed_value = serde_json::from_str::<serde_json::Value>(
                &super::Json(&value).display().to_string(),
            )
            .unwrap();

            assert_eq!(parsed_value, serde_json_value)
        })
        .await
    }

    #[tokio::test]
    async fn json_is_serialized_without_newlines() {
        crate::tests::fuzz::run_async("json_is_serialized_without_newlines", async |test_data| {
            let json_value =
                super::Json(test_data.generate_value_with_parameter::<JsonValue, _>(100))
                    .display()
                    .to_string();

            if json_value.contains('\n') {
                panic!("JSON values must not contain '\n'");
            }
        })
        .await
    }
}
