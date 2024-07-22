//! Types and traits for extracting data from requests.
//!
//! A handler function is an async function that takes any number of "extractors" as arguments. All arguments must implement [FromRequestParts], and the final extractory may optionally implement [FromRequest].
//!
//! For example:
//!
//! + [`State<T>`] will extract part or all of the application state.
//! + [`Form<T: serde::DeserializeOwned>`] will extract the body of a request as Form data.
//!
//! For an example of how to implement [FromRequest], see [custom_extractor](https://github.com/sammhicks/picoserve/blob/main/examples/custom_extractor/src/main.rs)
//!
//! ## Requests and Borrowing
//!
//! Although [RequestHandlerFunctions](crate::routing::RequestHandlerFunction) may not borrow from request due to restrictions with Higher-Order-Lifetime-Bounds, by using [from_request](crate::from_request) and [from_request_parts](crate::from_request_parts), [RequestHandlerServices](crate::routing::RequestHandlerService) and [PathRouterServices](crate::routing::PathRouterService) may do so.

use crate::{
    io::{Read, ReadExt},
    request::{RequestBody, RequestParts},
    response::{IntoResponse, StatusCode},
    ResponseSent,
};

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

#[macro_export]
/// Extract values from Request Parts. Each `$name` must implement [FromRequestParts], but may borrow from the request.
/// If extraction is rejected, the rejection is written to `$response_writer` and the function returns.
macro_rules! from_request_parts {
    ($state:ident, $request:ident, $response_writer:ident $(,$name:ty)* $(,)?) => {
        (
            $(
                match <$name as $crate::extract::FromRequestParts>::from_request_parts($state, &$request.parts).await {
                    Ok(value) => value,
                    Err(err) => return err.write_to($request.body.finalize().await?, $response_writer).await,
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

#[macro_export]
/// Extract a value from a request. `$name` must implement [FromRequest], but may borrow from the request.
/// If extraction is rejected, the rejection is written to `$response_writer` and the function returns.
macro_rules! from_request {
    ($state:ident, $request:ident, $response_writer:ident, $name:ty $(,)?) => {
        match <$name as $crate::extract::FromRequest>::from_request(
            $state,
            $request.parts,
            $request.body.body(),
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return err
                    .write_to($request.body.finalize().await?, $response_writer)
                    .await
            }
        }
    };
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Errors arising while reading the entire body
pub enum FailedToExtractEntireBodyError {
    BufferIsTooSmall {
        content_length: usize,
        buffer_length: usize,
    },
    IoError,
}

impl IntoResponse for FailedToExtractEntireBodyError {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            FailedToExtractEntireBodyError::BufferIsTooSmall {
                content_length,
                buffer_length,
            } => {
                (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format_args!(
                        "No space to extract entire body. Content Length: {}. Buffer Length: {}.",
                        content_length, buffer_length,
                    ),
                )
                    .write_to(connection, response_writer)
                    .await
            }
            FailedToExtractEntireBodyError::IoError => {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "IO Error while reading body",
                )
                    .write_to(connection, response_writer)
                    .await
            }
        }
    }
}

impl<'r, State> FromRequest<'r, State> for &'r mut [u8] {
    type Rejection = FailedToExtractEntireBodyError;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        if !request_body.entire_body_fits_into_buffer() {
            return Err(FailedToExtractEntireBodyError::BufferIsTooSmall {
                content_length: request_body.content_length(),
                buffer_length: request_body.buffer_length(),
            });
        }

        request_body.read_all().await.map_err(|err| {
            log_error!(
                "Failed to read body: {}",
                crate::logging::Debug2Format(&err)
            );
            FailedToExtractEntireBodyError::IoError
        })
    }
}

impl<'r, State> FromRequest<'r, State> for &'r [u8] {
    type Rejection = FailedToExtractEntireBodyError;

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

#[cfg(feature = "alloc")]
impl<'r, State> FromRequest<'r, State> for alloc::vec::Vec<u8> {
    type Rejection = FailedToExtractEntireBodyError;

    async fn from_request<R: Read>(
        _state: &'r State,
        _request_parts: RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        let mut buffer = alloc::vec::Vec::new();

        buffer
            .try_reserve_exact(request_body.content_length())
            .map_err(|_| FailedToExtractEntireBodyError::BufferIsTooSmall {
                content_length: request_body.content_length(),
                buffer_length: request_body.buffer_length(),
            })?;

        buffer.fill(0);

        request_body
            .reader()
            .read_exact(buffer.as_mut_slice())
            .await
            .map_err(|err| {
                log_error!(
                    "Failed to read body: {:?}",
                    crate::logging::Debug2Format(&err)
                );
                FailedToExtractEntireBodyError::IoError
            })?;

        Ok(buffer)
    }
}

#[cfg(feature = "alloc")]
impl<'r, State> FromRequest<'r, State> for alloc::borrow::Cow<'r, [u8]> {
    type Rejection = FailedToExtractEntireBodyError;

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

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Errors arising while reading the entire body as a UTF-8 String
pub enum FailedToExtractEntireBodyAsStringError {
    FailedToExtractEntireBody(FailedToExtractEntireBodyError),
    StringIsNotUtf8(#[cfg_attr(feature = "defmt", defmt(Debug2Format))] core::str::Utf8Error),
}

impl IntoResponse for FailedToExtractEntireBodyAsStringError {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            FailedToExtractEntireBodyAsStringError::FailedToExtractEntireBody(err) => {
                err.write_to(connection, response_writer).await
            }
            FailedToExtractEntireBodyAsStringError::StringIsNotUtf8(err) => {
                (
                    StatusCode::BAD_REQUEST,
                    format_args!("Body is not UTF-8: {err}"),
                )
                    .write_to(connection, response_writer)
                    .await
            }
        }
    }
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

#[cfg(feature = "alloc")]
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

#[cfg(feature = "alloc")]
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

/// Rejection used for [Query].
pub struct QueryRejection;

impl IntoResponse for QueryRejection {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (StatusCode::BAD_REQUEST, "Bad Query\n")
            .write_to(connection, response_writer)
            .await
    }
}

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

/// Rejection used for [Form].
pub enum FormRejection {
    /// Error decoding the body as UTF-8
    BodyIsNotUtf8,
    /// Error deserializing Form
    BadForm,
}

impl IntoResponse for FormRejection {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            StatusCode::BAD_REQUEST,
            match self {
                Self::BodyIsNotUtf8 => "Body is not UTF-8\n",
                Self::BadForm => "Bad Form\n",
            },
        )
            .write_to(connection, response_writer)
            .await
    }
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

/// Used to do reference to value conversions, mainly used with the [State] extractor to extract parts of the application state.
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

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// The Connection could not be upgraded because the "Upgrade" headed was missing
pub struct NoUpgradeHeaderError;

impl IntoResponse for NoUpgradeHeaderError {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            StatusCode::BAD_REQUEST,
            "Connection header did not include `upgrade`\n",
        )
            .write_to(connection, response_writer)
            .await
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// A token which allows a connection to be upgraded. Verifies that the "Upgrade" header has been set
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
