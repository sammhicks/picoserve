//! [UrlEncodedString] and related types.

use core::fmt;

use serde::de::Error;

/// The error returned when attempting to decode a character in a [UrlEncodedString].
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum UrlEncodedCharacterDecodeError {
    /// Percent symbol is not followed by two hex digits.
    BadlyFormattedPercentEncoding,
    /// Percent-encoded sequence does not decode into UTF-8 byte sequence.
    Utf8Error,
}

impl fmt::Display for UrlEncodedCharacterDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadlyFormattedPercentEncoding => {
                write!(f, "Percent symbol is not followed by two hex digits")
            }
            Self::Utf8Error => write!(
                f,
                "Percent-encoded sequence does not decode into UTF-8 byte sequence"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for UrlEncodedCharacterDecodeError {}

/// A decoded character.
pub enum UrlDecodedCharacter {
    /// This character was present in the encoded string.
    Literal(char),
    /// This character was percent-encoded.
    Encoded(char),
}

impl UrlDecodedCharacter {
    /// Convert into a [char], ignoring whether the character was present in the encoded string or percent-encoded.
    pub const fn into_char(self) -> char {
        match self {
            UrlDecodedCharacter::Literal(c) | UrlDecodedCharacter::Encoded(c) => c,
        }
    }
}

/// An iterator over the decoded [UrlDecodedCharacter]s of a [UrlEncodedString].
pub struct UrlDecodedCharacters<'a>(core::str::Chars<'a>);

impl<'a> UrlDecodedCharacters<'a> {
    /// Views the underlying data as a substring of the original string.
    pub fn as_str(&self) -> UrlEncodedString<'a> {
        UrlEncodedString(self.0.as_str())
    }
}

impl<'a> Iterator for UrlDecodedCharacters<'a> {
    type Item = Result<UrlDecodedCharacter, UrlEncodedCharacterDecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(Ok(match self.0.next()? {
            '+' => UrlDecodedCharacter::Encoded(' '),
            '%' => {
                fn to_hex(c: char) -> Option<u8> {
                    c.to_digit(16).map(|b| b as u8)
                }

                struct Ones(u8);

                impl Iterator for Ones {
                    type Item = ();

                    fn next(&mut self) -> Option<Self::Item> {
                        let b = (0b10000000 & self.0) > 0;

                        self.0 <<= 1;

                        b.then_some(())
                    }
                }

                let mut first_byte = {
                    let Some(first) = self.0.next().and_then(to_hex) else {
                        return Some(Err(
                            UrlEncodedCharacterDecodeError::BadlyFormattedPercentEncoding,
                        ));
                    };
                    let Some(second) = self.0.next().and_then(to_hex) else {
                        return Some(Err(
                            UrlEncodedCharacterDecodeError::BadlyFormattedPercentEncoding,
                        ));
                    };

                    first * 0x10 + second
                };

                let mut bits = Ones(first_byte);

                let code_point = if bits.next().is_some() {
                    let byte_count = 1 + bits.count();

                    if byte_count == 1 {
                        return Some(Err(UrlEncodedCharacterDecodeError::Utf8Error));
                    }

                    // Zero our the prefix bytes
                    first_byte <<= byte_count;
                    first_byte >>= byte_count;

                    let mut code_point = u32::from(first_byte);

                    for _ in 1..byte_count {
                        let Some('%') = self.0.next() else {
                            return Some(Err(UrlEncodedCharacterDecodeError::Utf8Error));
                        };

                        let next_byte = {
                            let Some(first) = self.0.next().and_then(to_hex) else {
                                return Some(Err(
                                    UrlEncodedCharacterDecodeError::BadlyFormattedPercentEncoding,
                                ));
                            };
                            let Some(second) = self.0.next().and_then(to_hex) else {
                                return Some(Err(
                                    UrlEncodedCharacterDecodeError::BadlyFormattedPercentEncoding,
                                ));
                            };

                            first * 0x10 + second
                        };

                        if (0b11000000 & next_byte) != 0b10000000 {
                            return Some(Err(UrlEncodedCharacterDecodeError::Utf8Error));
                        }

                        code_point <<= 6;
                        code_point += u32::from(0b00111111 & next_byte);
                    }

                    code_point
                } else {
                    first_byte.into()
                };

                let Some(c) = char::from_u32(code_point) else {
                    return Some(Err(UrlEncodedCharacterDecodeError::Utf8Error));
                };
                UrlDecodedCharacter::Encoded(c)
            }
            c => UrlDecodedCharacter::Literal(c),
        }))
    }
}

const URL_ENCODED_KEY: &str = "____URL_ENCODED____";

fn deserializer<'a>(
    key: &'a str,
    value: UrlEncodedString<'a>,
) -> serde::de::value::MapDeserializer<
    'a,
    core::option::IntoIter<(&'a str, DeserializeUrlEncoded<'a>)>,
    DeserializationError,
> {
    serde::de::value::MapDeserializer::new(
        Some((URL_ENCODED_KEY, DeserializeUrlEncoded { key, value })).into_iter(),
    )
}

#[derive(serde::Deserialize)]
struct UrlEncodedRepresentation<'a> {
    #[serde(rename = "____URL_ENCODED____")]
    value: &'a str,
}

/// The error returned when attempting to decode a [UrlEncodedString] into a string.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DecodeError {
    /// Error during decoding a character.
    BadUrlEncodedCharacter(UrlEncodedCharacterDecodeError),
    /// The provided buffer does not have enough space to store the string.
    NoSpace,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUrlEncodedCharacter(bad_url_encoded_character) => {
                bad_url_encoded_character.fmt(f)
            }
            Self::NoSpace => write!(f, "No space to decode url-encoded string"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}

struct NamedDecodeError<'a> {
    key: &'a str,
    error: DecodeError,
}

/// A url-encoded string.
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(from = "UrlEncodedRepresentation")]
pub struct UrlEncodedString<'a>(pub &'a str);

impl<'a> fmt::Debug for UrlEncodedString<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'r> PartialEq<&'r str> for UrlEncodedString<'r> {
    fn eq(&self, other: &&'r str) -> bool {
        matches!(self.strip_prefix(other), Some(UrlEncodedString("")))
    }
}

impl<'de> From<UrlEncodedRepresentation<'de>> for UrlEncodedString<'de> {
    fn from(UrlEncodedRepresentation { value }: UrlEncodedRepresentation<'de>) -> Self {
        Self(value)
    }
}

impl<'a> UrlEncodedString<'a> {
    /// Returns an iterator over the decoded [UrlDecodedCharacter]s of the string.
    pub fn chars(self) -> UrlDecodedCharacters<'a> {
        UrlDecodedCharacters(self.0.chars())
    }

    /// Try decoding the chars into a string.
    pub fn try_into_string<const N: usize>(self) -> Result<heapless::String<N>, DecodeError> {
        let mut str = heapless::String::new();

        for c in self.chars() {
            str.push(c.map_err(DecodeError::BadUrlEncodedCharacter)?.into_char())
                .map_err(|()| DecodeError::NoSpace)?;
        }

        Ok(str)
    }

    #[cfg(feature = "std")]
    /// Try decoding the chars into a std::string::String.
    pub fn try_into_std_string(
        self,
    ) -> Result<std::string::String, UrlEncodedCharacterDecodeError> {
        self.chars()
            .map(|c| c.map(UrlDecodedCharacter::into_char))
            .collect()
    }

    /// Returns a substring with the prefix removed. A '/' in the prefix must match a literal '/' in the encoded string,
    /// all other characters ignore whether the character is literal or percent-encoded.
    pub fn strip_prefix(self, prefix: &str) -> Option<Self> {
        let mut chars = self.chars();

        for c in prefix.chars() {
            if c == '/' {
                let UrlDecodedCharacter::Literal('/') = chars.next()?.ok()? else {
                    return None;
                };
            } else if c != chars.next()?.ok()?.into_char() {
                return None;
            }
        }

        Some(chars.as_str())
    }

    /// Returns true if the string has a length of 0.
    pub const fn is_empty(self) -> bool {
        self.0.is_empty()
    }

    fn with_decoded<'d, T, E: From<NamedDecodeError<'d>>, F: FnOnce(&str) -> Result<T, E>>(
        self,
        key: &'d str,
        f: F,
    ) -> Result<T, E> {
        f(&self
            .try_into_string::<1024>()
            .map_err(|error| NamedDecodeError { key, error })?)
    }
}

#[derive(Debug)]
pub(crate) struct DeserializationError;

impl fmt::Display for DeserializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Deserialization Error")
    }
}

impl serde::de::Error for DeserializationError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        #[cfg(feature = "std")]
        println!("DeserializationError: {msg}");

        // TODO - defmt logging

        #[cfg(not(feature = "std"))]
        drop(msg);

        Self
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DeserializationError {}

impl<'de> From<NamedDecodeError<'de>> for DeserializationError {
    fn from(NamedDecodeError { key, error }: NamedDecodeError) -> Self {
        Self::custom(format_args!("No space to decode {key}: {error}"))
    }
}

struct DeserializeUrlEncoded<'de> {
    pub key: &'de str,
    pub value: UrlEncodedString<'de>,
}

impl<'de> serde::de::IntoDeserializer<'de, DeserializationError> for DeserializeUrlEncoded<'de> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self::Deserializer {
        self
    }
}

macro_rules! deserialize_parse_value {
    ($this:ident, $key_value:expr; $($deserialize:ident $visit:ident)*) => {
        $(
            fn $deserialize<V: serde::de::Visitor<'de>>(
                $this,
                visitor: V,
            ) -> Result<V::Value, Self::Error> {
                let (key, value) = $key_value;

                value.with_decoded(key, |value| {
                    visitor.$visit(value.parse().map_err(|err| {
                        DeserializationError::custom(format_args!("Failed to parse {}: {}", key, err))
                    })?)
                })
            }
        )*
    };
}

impl<'de> serde::Deserializer<'de> for DeserializeUrlEncoded<'de> {
    type Error = DeserializationError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.value.with_decoded(self.key, |v| visitor.visit_str(v))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name == "UrlEncodedString" && fields == [URL_ENCODED_KEY] {
            deserializer(self.key, self.value).deserialize_struct(name, fields, visitor)
        } else {
            Err(DeserializationError::custom("paths items must be atomic"))
        }
    }

    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.value.with_decoded(self.key, |value| {
            visitor.visit_enum(serde::de::value::StrDeserializer::new(value))
        })
    }

    fn deserialize_option<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    deserialize_parse_value!(
        self, (self.key, self.value);
        deserialize_bool visit_bool
        deserialize_f32 visit_f32 deserialize_f64 visit_f64
        deserialize_i8 visit_i8 deserialize_i16 visit_i16 deserialize_i32 visit_i32 deserialize_i64 visit_i64
        deserialize_u8 visit_u8 deserialize_u16 visit_u16 deserialize_u32 visit_u32 deserialize_u64 visit_u64
    );

    serde::forward_to_deserialize_any! {
        char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple
        tuple_struct map identifier ignored_any
    }
}

/// Failed to deserialize a URL-Encoded form
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FormDeserializationError;

impl fmt::Display for FormDeserializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to deserialize Url Encoded Form")
    }
}

impl serde::de::Error for FormDeserializationError {
    fn custom<T: fmt::Display>(_msg: T) -> Self {
        Self
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FormDeserializationError {}

impl From<super::url_encoded::DeserializationError> for FormDeserializationError {
    fn from(
        super::url_encoded::DeserializationError: super::url_encoded::DeserializationError,
    ) -> Self {
        Self
    }
}

struct DeserializeUrlEncodedForm<'r, T> {
    pairs: T,
    value: (&'r str, UrlEncodedString<'r>),
}

/// Deserialize the given URL-Encoded Form.
pub fn deserialize_form<T: serde::de::DeserializeOwned>(
    UrlEncodedString(form): UrlEncodedString,
) -> Result<T, FormDeserializationError> {
    T::deserialize(DeserializeUrlEncodedForm {
        pairs: form.split('&').filter(|s| !s.is_empty()),
        value: ("", UrlEncodedString("")),
    })
}

impl<'de, T: Iterator<Item = &'de str>> serde::de::Deserializer<'de>
    for DeserializeUrlEncodedForm<'de, T>
{
    type Error = FormDeserializationError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_map(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de, T: Iterator<Item = &'de str>> serde::de::MapAccess<'de>
    for DeserializeUrlEncodedForm<'de, T>
{
    type Error = FormDeserializationError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
    {
        self.pairs
            .next()
            .map(|value| {
                let (key, value) = value.split_once('=').ok_or(FormDeserializationError)?;

                self.value = (key, UrlEncodedString(value));

                Ok(seed.deserialize(DeserializeUrlEncoded {
                    key,
                    value: UrlEncodedString(key),
                })?)
            })
            .transpose()
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        let (name, value) = self.value;

        Ok(seed.deserialize(DeserializeUrlEncoded { key: name, value })?)
    }
}
