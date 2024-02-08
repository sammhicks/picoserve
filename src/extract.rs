//! Types and traits for extracting data from requests.
//!
//! A handler function is an async function that takes any number of "extractors" as arguments. An extractor is a type that implements [FromRequest].
//!
//! For example:
//!
//! + [`State<T>`] will extract part or all of the application state.
//! + [`Form<T: serde::DeserializeOwned>`] will extract the body of a request as Form data.
//!
//! For an example of how to implement [FromRequest], see [custom_extractor](https://github.com/sammhicks/picoserve/blob/main/examples/custom_extractor/src/main.rs)

use crate::{
    io::Read,
    request::{RequestBody, RequestParts},
    response::{status, IntoResponse},
    ResponseSent,
};

mod private {
    pub struct ViaRequest;
    pub struct ViaParts;
}

pub trait FromRequestParts<State>: Sized {
    type Rejection: IntoResponse;

    async fn from_request_parts(
        state: &State,
        request_parts: &RequestParts<'_>,
    ) -> Result<Self, Self::Rejection>;
}

/// Types that can be created from requests.
pub trait FromRequest<State, M = private::ViaRequest>: Sized {
    type Rejection: IntoResponse;

    async fn from_request<R: Read>(
        state: &State,
        request_parts: RequestParts<'_>,
        request_body: RequestBody<'_, R>,
    ) -> Result<Self, Self::Rejection>;
}

impl<State, T: FromRequestParts<State>> FromRequest<State, private::ViaParts> for T {
    type Rejection = <Self as FromRequestParts<State>>::Rejection;

    async fn from_request<R: Read>(
        state: &State,
        request_parts: RequestParts<'_>,
        _request_body: RequestBody<'_, R>,
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
        (status::BAD_REQUEST, "Bad Query\n")
            .write_to(connection, response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequestParts<State> for Query<T> {
    type Rejection = QueryRejection;

    async fn from_request_parts(
        _state: &State,
        request_parts: &RequestParts<'_>,
    ) -> Result<Self, Self::Rejection> {
        super::url_encoded::deserialize_form(request_parts.query().unwrap_or_default())
            .map(Self)
            .map_err(|super::url_encoded::BadUrlEncodedForm| QueryRejection)
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
    BodyIsNotUtf8,
    BadForm,
}

impl IntoResponse for FormRejection {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            status::BAD_REQUEST,
            match self {
                Self::BodyIsNotUtf8 => "Body is not UTF-8\n",
                Self::BadForm => "Bad Form\n",
            },
        )
            .write_to(connection, response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequest<State> for Form<T> {
    type Rejection = FormRejection;

    async fn from_request<R: Read>(
        _state: &State,
        _request_parts: RequestParts<'_>,
        request_body: RequestBody<'_, R>,
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
        .map_err(|super::url_encoded::BadUrlEncodedForm| FormRejection::BadForm)
    }
}

/// Used to do reference to value conversions, mainly used with the [State] extractor to extract parts of the application state.
pub trait FromRef<T> {
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

impl<S, T: FromRef<S>> FromRequestParts<S> for State<T> {
    type Rejection = core::convert::Infallible;

    async fn from_request_parts(
        state: &S,
        _request_parts: &RequestParts<'_>,
    ) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}

#[derive(Debug)]
pub struct NoUpgradeHeader;

impl IntoResponse for NoUpgradeHeader {
    async fn write_to<R: Read, W: crate::response::ResponseWriter<Error = R::Error>>(
        self,
        connection: crate::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            status::BAD_REQUEST,
            "Connection header did not include `upgrade`\n",
        )
            .write_to(connection, response_writer)
            .await
    }
}

/// A token which allows a connection to be upgraded. Verifies that the "Upgrade" header has been set
#[derive(Debug)]
pub struct UpgradeToken(());

impl<State> FromRequestParts<State> for UpgradeToken {
    type Rejection = NoUpgradeHeader;

    async fn from_request_parts(
        _state: &State,
        request_parts: &RequestParts<'_>,
    ) -> Result<Self, Self::Rejection> {
        request_parts
            .headers()
            .get("upgrade")
            .map(|_| Self(()))
            .ok_or(NoUpgradeHeader)
    }
}

impl UpgradeToken {
    pub(crate) async fn discard_all_data<R: Read>(
        connection: crate::response::Connection<'_, R>,
    ) -> Result<(), R::Error> {
        // Consumes and discards all data, so cannot gain access to the next requests data,
        // and the connection is consumed so cannot be upgraded after this call

        let mut reader = connection.upgrade(UpgradeToken(()));

        while reader.read(&mut [0; 8]).await? > 0 {}

        Ok(())
    }
}
