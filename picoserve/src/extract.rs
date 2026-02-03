//! Types and traits for extracting data from requests.
//!
//! A handler function is an async function that takes any number of "extractors" as arguments. All arguments must implement [`FromRequestParts`], and the final extractory may optionally implement [`FromRequest`].
//!
//! For example:
//!
//! + [`State<T>`] will extract part or all of the application state.
//! + [`Form<T: serde::DeserializeOwned>`] will extract the body of a request as Form data.
//!
//! For an example of how to implement [`FromRequest`], see [custom_extractor](https://github.com/sammhicks/picoserve/blob/main/examples/custom_extractor/src/main.rs)
//!
//! ## Requests and Borrowing
//!
//! Although [`RequestHandlerFunctions`](crate::routing::RequestHandlerFunction) may not borrow from request due to restrictions with Higher-Order-Lifetime-Bounds, by using [`from_request`](crate::from_request) and [`from_request_parts`](crate::from_request_parts), [`RequestHandlerServices`](crate::routing::RequestHandlerService) and [`PathRouterServices`](crate::routing::PathRouterService) may do so.

use crate::{
    self as picoserve,
    io::{Error, Read, ReadExt},
    request::{ReadAllBodyError, RequestBody, RequestParts},
    response::{ErrorWithStatusCode, IntoResponse},
};

#[cfg(feature = "json")]
pub mod json {
    pub use crate::json::Json;

    pub use serde_json_core::str;

    /// A JSON encoded value. `UNESCAPE_BUFFER_SIZE` is the size of the temporary buffer used for unescaping strings.
    pub struct JsonWithUnescapeBufferSize<T, const UNESCAPE_BUFFER_SIZE: usize>(pub T);
}

#[cfg(feature = "json")]
pub use json::{Json, JsonWithUnescapeBufferSize};

mod private {
    pub struct ViaRequest;
    pub struct ViaParts;
}

/// Types that can be created from requests parts (everything except the request body).
pub trait FromRequestParts<'r, State>: Sized {
    /// If the extractor fails this “rejection” type is returned, which converted into a response and returned.
    type Rejection: IntoResponse + 'static;

    /// Attempt to extract from the request parts.
    async fn from_request_parts(
        state: &'r State,
        request_parts: &RequestParts<'r>,
    ) -> Result<Self, Self::Rejection>;
}

/// Extract values from Request Parts. Each `$name` must implement [`FromRequestParts`], but may borrow from the request.
/// If extraction is rejected, the rejection is written to `$response_writer` and the function returns.
#[macro_export]
macro_rules! from_request_parts {
    ($state:ident, $request:ident, $response_writer:ident $(,$name:ty)* $(,)?) => {
        (
            $(
                match <$name as $crate::extract::FromRequestParts<_>>::from_request_parts($state, &$request.parts).await {
                    Ok(value) => value,
                    Err(err) => return err.write_to($request.body_connection.finalize().await?, $response_writer).await,
                }
            ),*
        )
    };
}

/// Types that can be created from requests.
pub trait FromRequest<'r, State, M = private::ViaRequest>: Sized {
    /// If the extractor fails this “rejection” type is returned, which converted into a response and returned.
    type Rejection: IntoResponse + 'static;

    /// Attempt to extract from the request.
    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection>;
}

/// Extract a value from a request. `$name` must implement [`FromRequest`], but may borrow from the request.
/// If extraction is rejected, the rejection is written to `$response_writer` and the function returns.
#[macro_export]
macro_rules! from_request {
    ($state:ident, $request:ident, $response_writer:ident, $name:ty $(,)?) => {
        match <$name as $crate::extract::FromRequest<_, _>>::from_request(
            $state,
            $request.parts,
            $request.body_connection.body(),
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return err
                    .write_to($request.body_connection.finalize().await?, $response_writer)
                    .await
            }
        }
    };
}

impl<'r, State> FromRequest<'r, State> for &'r mut [u8] {
    type Rejection = ReadAllBodyError;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        request_body.read_all().await
    }
}

impl<'r, State> FromRequest<'r, State> for &'r [u8] {
    type Rejection = ReadAllBodyError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        <&'r mut [u8]>::from_request(state, request_parts, request_body)
            .await
            .map(|body| &*body)
    }
}

impl<'r, State, const N: usize> FromRequest<'r, State> for heapless::Vec<u8, N> {
    type Rejection = ReadAllBodyError;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        let mut buffer = Self::new();

        let content_length = request_body.content_length();

        buffer
            .resize(request_body.content_length(), 0)
            .map_err(|()| ReadAllBodyError::BufferIsTooSmall {
                content_length,
                buffer_length: N,
            })?;

        request_body
            .reader()
            .read_exact(buffer.as_mut_slice())
            .await
            .map_err(|error| match error {
                embedded_io_async::ReadExactError::UnexpectedEof => ReadAllBodyError::UnexpectedEof,
                embedded_io_async::ReadExactError::Other(error) => {
                    ReadAllBodyError::IO(error.kind())
                }
            })?;

        Ok(buffer)
    }
}

#[cfg(any(test, feature = "alloc"))]
impl<'r, State> FromRequest<'r, State> for alloc::vec::Vec<u8> {
    type Rejection = ReadAllBodyError;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        let mut buffer = alloc::vec::Vec::new();

        let content_length = request_body.content_length();

        buffer.try_reserve_exact(content_length).map_err(|_| {
            ReadAllBodyError::BufferIsTooSmall {
                content_length,
                buffer_length: request_body.buffer_length(),
            }
        })?;

        buffer.resize(content_length, 0);

        request_body
            .reader()
            .read_exact(buffer.as_mut_slice())
            .await
            .map_err(|error| match error {
                embedded_io_async::ReadExactError::UnexpectedEof => ReadAllBodyError::UnexpectedEof,
                embedded_io_async::ReadExactError::Other(error) => {
                    ReadAllBodyError::IO(error.kind())
                }
            })?;

        Ok(buffer)
    }
}

#[cfg(any(test, feature = "alloc"))]
impl<'r, State> FromRequest<'r, State> for alloc::borrow::Cow<'r, [u8]> {
    type Rejection = ReadAllBodyError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        if request_body.entire_body_fits_into_buffer() {
            <&'r [u8]>::from_request(state, request_parts, request_body)
                .await
                .map(alloc::borrow::Cow::Borrowed)
        } else {
            alloc::vec::Vec::<u8>::from_request(state, request_parts, request_body)
                .await
                .map(alloc::borrow::Cow::Owned)
        }
    }
}

/// Errors arising while reading the entire body as a UTF-8 String.
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FailedToExtractEntireBodyAsStringError {
    #[error(transparent)]
    #[status_code(transparent)]
    FailedToExtractEntireBody(ReadAllBodyError),
    #[error("Body is not UTF-8: {0}")]
    #[status_code(BAD_REQUEST)]
    StringIsNotUtf8(#[cfg_attr(feature = "defmt", defmt(Debug2Format))] core::str::Utf8Error),
}

impl<'r, State> FromRequest<'r, State> for &'r mut str {
    type Rejection = FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        core::str::from_utf8_mut(
            <&'r mut [u8]>::from_request(state, request_parts, request_body)
                .await
                .map_err(FailedToExtractEntireBodyAsStringError::FailedToExtractEntireBody)?,
        )
        .map_err(FailedToExtractEntireBodyAsStringError::StringIsNotUtf8)
    }
}

impl<'r, State> FromRequest<'r, State> for &'r str {
    type Rejection = FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        <&'r mut str>::from_request(state, request_parts, request_body)
            .await
            .map(|body| &*body)
    }
}

impl<'r, State, const N: usize> FromRequest<'r, State> for heapless::String<N> {
    type Rejection = FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        heapless::String::from_utf8(
            heapless::Vec::from_request(state, request_parts, request_body)
                .await
                .map_err(FailedToExtractEntireBodyAsStringError::FailedToExtractEntireBody)?,
        )
        .map_err(FailedToExtractEntireBodyAsStringError::StringIsNotUtf8)
    }
}

#[cfg(any(test, feature = "alloc"))]
impl<'r, State> FromRequest<'r, State> for alloc::string::String {
    type Rejection = FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        alloc::string::String::from_utf8(
            alloc::vec::Vec::from_request(state, request_parts, request_body)
                .await
                .map_err(FailedToExtractEntireBodyAsStringError::FailedToExtractEntireBody)?,
        )
        .map_err(|err| FailedToExtractEntireBodyAsStringError::StringIsNotUtf8(err.utf8_error()))
    }
}

#[cfg(any(test, feature = "alloc"))]
impl<'r, State> FromRequest<'r, State> for alloc::borrow::Cow<'r, str> {
    type Rejection = FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        if request_body.entire_body_fits_into_buffer() {
            <&'r str>::from_request(state, request_parts, request_body)
                .await
                .map(alloc::borrow::Cow::Borrowed)
        } else {
            alloc::string::String::from_request(state, request_parts, request_body)
                .await
                .map(alloc::borrow::Cow::Owned)
        }
    }
}

impl<'r, State, T: FromRequestParts<'r, State>> FromRequest<'r, State, private::ViaParts> for T
where
    T::Rejection: 'static,
{
    type Rejection = <Self as FromRequestParts<'r, State>>::Rejection;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        _request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        Self::from_request_parts(state, &request_parts).await
    }
}

/// Extractor that deserializes query strings into some type.
pub struct Query<T: serde::de::DeserializeOwned>(pub T);

impl<T: serde::de::DeserializeOwned> core::ops::Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: serde::de::DeserializeOwned> core::ops::DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Rejection used for [`Query`].
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("Bad Query")]
#[status_code(BAD_REQUEST)]
pub struct QueryRejection;

impl<'r, State, T: serde::de::DeserializeOwned> FromRequestParts<'r, State> for Query<T> {
    type Rejection = QueryRejection;

    async fn from_request_parts(
        _state: &'r State,
        request_parts: &RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        super::url_encoded::deserialize_form(request_parts.query().unwrap_or_default())
            .map(Self)
            .map_err(|super::url_encoded::FormDeserializationError| QueryRejection)
    }
}

/// URL encoded extractor.
pub struct Form<T: serde::de::DeserializeOwned>(pub T);

impl<T: serde::de::DeserializeOwned> core::ops::Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: serde::de::DeserializeOwned> core::ops::DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Rejection used for [`Form`].
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[status_code(BAD_REQUEST)]
pub enum FormRejection {
    /// Error decoding the body as UTF-8
    #[error("Body is not UTF-8")]
    BodyIsNotUtf8,
    /// Error deserializing Form
    #[error("Bad Form")]
    BadForm,
}

impl<'r, State, T: serde::de::DeserializeOwned> FromRequest<'r, State> for Form<T> {
    type Rejection = FormRejection;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        super::url_encoded::deserialize_form(crate::url_encoded::UrlEncodedString(
            core::str::from_utf8(
                request_body
                    .read_all()
                    .await
                    .map_err(|_| FormRejection::BadForm)?,
            )
            .map_err(|core::str::Utf8Error { .. }| FormRejection::BodyIsNotUtf8)?,
        ))
        .map(Self)
        .map_err(|super::url_encoded::FormDeserializationError| FormRejection::BadForm)
    }
}

/// Rejection used for [`Json`].
#[cfg(feature = "json")]
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum JsonRejection {
    #[error("IO Error")]
    #[status_code(INTERNAL_SERVER_ERROR)]
    IoError,
    #[error("Failed to parse JSON body: {0}")]
    #[status_code(BAD_REQUEST)]
    #[cfg(feature = "json")]
    DeserializationError(serde_json_core::de::Error),
}

#[cfg(feature = "json")]
impl<'r, State, T: serde::Deserialize<'r>, const UNESCAPE_BUFFER_SIZE: usize>
    FromRequest<'r, State, T> for JsonWithUnescapeBufferSize<T, UNESCAPE_BUFFER_SIZE>
{
    type Rejection = JsonRejection;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        serde_json_core::from_slice_escaped(
            request_body
                .read_all()
                .await
                .map_err(|_| JsonRejection::IoError)?,
            &mut [0; UNESCAPE_BUFFER_SIZE],
        )
        .map(|(value, _)| Self(value))
        .map_err(JsonRejection::DeserializationError)
    }
}

#[cfg(feature = "json")]
impl<'r, State, T: serde::Deserialize<'r>> FromRequest<'r, State, T> for Json<T> {
    type Rejection = JsonRejection;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        JsonWithUnescapeBufferSize::<T, 32>::from_request(state, request_parts, request_body)
            .await
            .map(|JsonWithUnescapeBufferSize(payload)| Self(payload))
    }
}

/// Used to do reference to value conversions, mainly used with the [`State`] extractor to extract parts of the application state.
pub trait FromRef<T> {
    /// Perform the reference to value conversion
    fn from_ref(input: &T) -> Self;
}

impl<T: Clone> FromRef<T> for T {
    fn from_ref(input: &T) -> Self {
        input.clone()
    }
}

/// Extracts part of the application state.
///
/// `T` must implement [`FromRef<S>`] for application state `S`.
pub struct State<T>(
    /// The value extracted from the application state
    pub T,
);

impl<'r, S, T: FromRef<S>> FromRequestParts<'r, S> for State<T> {
    type Rejection = core::convert::Infallible;

    async fn from_request_parts(
        state: &'r S,
        _request_parts: &RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}

/// The Connection could not be upgraded because the "Upgrade" headed was missing.
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("Connection header did not include `upgrade`")]
#[status_code(BAD_REQUEST)]
pub struct NoUpgradeHeaderError;

/// A token which allows a connection to be upgraded. Verifies that the "Upgrade" header has been set.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UpgradeToken(());

impl<'r, State> FromRequestParts<'r, State> for UpgradeToken {
    type Rejection = NoUpgradeHeaderError;

    async fn from_request_parts(
        _state: &'r State,
        request_parts: &RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        request_parts
            .headers()
            .get("upgrade")
            .map(|_| Self(()))
            .ok_or(NoUpgradeHeaderError)
    }
}

impl UpgradeToken {
    pub(crate) async fn discard_all_data<R: Read>(
        connection: crate::response::Connection<'_, R>,
    ) -> Result<(), R::Error> {
        // Consumes and discards all data, so cannot gain access to the next requests data,
        // and the connection is consumed so cannot be upgraded after this call

        connection
            .upgrade(UpgradeToken(()))
            .discard_all_data()
            .await
    }
}
