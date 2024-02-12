use core::fmt;

use serde::Serialize;

use crate::io::{FormatBuffer, FormatBufferWriteError, Write};

#[derive(Debug)]
struct SerializeError;

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl serde::ser::Error for SerializeError {
    fn custom<T: fmt::Display>(_msg: T) -> Self {
        Self
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SerializeError {}

impl From<fmt::Error> for SerializeError {
    fn from(fmt::Error: fmt::Error) -> Self {
        Self
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
        self.0.write_str(s).map_err(|fmt::Error| SerializeError)
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), SerializeError> {
        self.0.write_fmt(args).map_err(|fmt::Error| SerializeError)
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

impl<'a, W: fmt::Write> serde::ser::SerializeSeq for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeTuple for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeTupleStruct for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeTupleVariant for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeMap for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeStruct for SerializeCompound<'a, W> {
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

impl<'a, W: fmt::Write> serde::ser::SerializeStructVariant for SerializeCompound<'a, W> {
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

enum JsonStream<T> {
    Short { buffer: FormatBuffer },
    Long { buffer: FormatBuffer, value: T },
}

impl<T: serde::Serialize> JsonStream<T> {
    fn new(value: T) -> Self {
        let mut buffer = FormatBuffer::new(0);
        match value.serialize(Serializer(&mut buffer)) {
            Ok(()) => JsonStream::Short { buffer },
            Err(SerializeError) => match buffer.error_state {
                FormatBufferWriteError::FormatError => JsonStream::Long {
                    buffer: FormatBuffer::new(0),
                    value,
                },
                FormatBufferWriteError::OutOfSpace(()) => JsonStream::Long { buffer, value },
            },
        }
    }
}

impl<T: serde::Serialize> JsonStream<T> {
    async fn write_json_value<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        match self {
            JsonStream::Short { buffer } => writer.write_all(&buffer.data).await,
            JsonStream::Long { mut buffer, value } => {
                writer.write_all(&buffer.data).await?;

                let mut ignore_count = buffer.data.len();

                loop {
                    buffer.data.clear();
                    buffer.ignore_count = ignore_count;
                    buffer.error_state = FormatBufferWriteError::FormatError;

                    match value.serialize(Serializer(&mut buffer)) {
                        Ok(()) => return writer.write_all(&buffer.data).await,
                        Err(SerializeError) => match buffer.error_state {
                            FormatBufferWriteError::FormatError => {
                                return writer.write_all(b"\r\n\r\nFailed to serialize JSON").await
                            }
                            FormatBufferWriteError::OutOfSpace(()) => {
                                writer.write_all(&buffer.data).await?;
                                ignore_count += buffer.data.len();
                            }
                        },
                    }
                }
            }
        }
    }
}

struct JsonBody<T>(JsonStream<T>);

impl<T: serde::Serialize> super::Content for JsonBody<T> {
    fn content_type(&self) -> &'static str {
        "application/json"
    }

    fn content_length(&self) -> usize {
        match &self.0 {
            JsonStream::Short { buffer } => buffer.data.len(),
            JsonStream::Long { buffer: _, value } => {
                let mut content_length = super::MeasureFormatSize(0);
                value
                    .serialize(Serializer(&mut content_length))
                    .map_or(0, |()| content_length.0)
            }
        }
    }

    async fn write_content<R: crate::io::Read, W: Write>(
        self,
        _connection: super::Connection<'_, R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        self.0.write_json_value(&mut writer).await
    }
}

/// Serializes the value in JSON form. The value might be serialized several times during sending, so the value must be serialized in the same way each time.
pub struct Json<T>(pub T);

impl<T: serde::Serialize> Json<T> {
    pub(crate) async fn do_write_to<W: Write>(&self, writer: &mut W) -> Result<(), W::Error> {
        JsonStream::new(&self.0).write_json_value(writer).await
    }

    /// Convert JSON payload into a [super::Response] with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        super::Response::ok(JsonBody(JsonStream::new(self.0)))
    }
}

impl<T: serde::Serialize> super::IntoResponse for Json<T> {
    async fn write_to<R: embedded_io_async::Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        response_writer
            .write_response(connection, self.into_response())
            .await
    }
}

impl<T: serde::Serialize> core::future::IntoFuture for Json<T> {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}
