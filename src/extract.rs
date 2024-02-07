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

use embedded_io_async::{Read, Write};

use crate::{request::Request, response, response::IntoResponse, ResponseSent};

/// Types that can be created from requests.
pub trait FromRequest<State>: Sized {
    type Rejection: IntoResponse;

    async fn from_request<R: Read>(state: &State, request: &Request, body_reader: R) -> Result<Self, Self::Rejection>;
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
    async fn write_to<R: Read, W: response::ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: response::Connection<R>,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        (response::status::BAD_REQUEST, "Bad Query\n")
            .write_to(writer, connection, response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequest<State> for Query<T> {
    type Rejection = QueryRejection;

    async fn from_request<R: Read>(
        _state: &State,
        request: &Request<'_>,
        _body_reader: R,
    ) -> Result<Query<T>, QueryRejection> {
        todo!()
        // super::url_encoded::deserialize_form(request.query().unwrap_or_default())
        //     .map(Self)
        //     .map_err(|super::url_encoded::BadUrlEncodedForm| QueryRejection)
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
    async fn write_to<R: Read, W: response::ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: response::Connection<R>,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        (
            response::status::BAD_REQUEST,
            match self {
                Self::BodyIsNotUtf8 => "Body is not UTF-8\n",
                Self::BadForm => "Bad Form\n",
            },
        )
            .write_to(writer, connection, response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequest<State> for Form<T> {
    type Rejection = FormRejection;

    async fn from_request<R: Read>(_state: &State, request: &Request<'_>, body_reader: R) -> Result<Form<T>, FormRejection> {
        todo!()
        // super::url_encoded::deserialize_form(crate::url_encoded::UrlEncodedString(
        //     core::str::from_utf8(request.body())
        //         .map_err(|core::str::Utf8Error { .. }| FormRejection::BodyIsNotUtf8)?,
        // ))
        // .map(Self)
        // .map_err(|super::url_encoded::BadUrlEncodedForm| FormRejection::BadForm)
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

impl<S, T: FromRef<S>> FromRequest<S> for State<T> {
    type Rejection = core::convert::Infallible;

    async fn from_request<R: Read>(state: &S, _request: &Request<'_>, _body_reader: R) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}
