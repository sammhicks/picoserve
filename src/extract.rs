//! Types and traits for extracting data from requests.

use crate::{request::Request, response::status, response::IntoResponse, ResponseSent};

/// Types that can be created from requests.
pub trait FromRequest<State>: Sized {
    type Rejection: IntoResponse;

    async fn from_request(state: &State, request: &Request) -> Result<Self, Self::Rejection>;
}

/// Extractor that deserializes query strings into some type.
pub struct Query<T: serde::de::DeserializeOwned>(pub T);

/// Rejection used for [Query].
pub enum QueryRejection {
    NoQuery,
    BadQuery,
}

impl IntoResponse for QueryRejection {
    async fn write_to<W: super::response::ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            status::BAD_REQUEST,
            match self {
                QueryRejection::NoQuery => "No Query\n",
                QueryRejection::BadQuery => "Bad Query\n",
            },
        )
            .write_to(response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequest<State> for Query<T> {
    type Rejection = QueryRejection;

    async fn from_request(
        _state: &State,
        request: &Request<'_>,
    ) -> Result<Query<T>, QueryRejection> {
        super::url_encoded::deserialize_url_encoded_form(
            request.query.ok_or(QueryRejection::NoQuery)?.0,
        )
        .map(Self)
        .map_err(|super::url_encoded::BadUrlEncodedForm| QueryRejection::BadQuery)
    }
}

/// URL encoded extractor.
pub struct Form<T: serde::de::DeserializeOwned>(pub T);

/// Rejection used for [Form].
pub enum FormRejection {
    BodyIsNotUtf8,
    BadForm,
}

impl IntoResponse for FormRejection {
    async fn write_to<W: super::response::ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            status::BAD_REQUEST,
            match self {
                Self::BodyIsNotUtf8 => "Body is not UTF-8\n",
                Self::BadForm => "Bad Form\n",
            },
        )
            .write_to(response_writer)
            .await
    }
}

impl<State, T: serde::de::DeserializeOwned> FromRequest<State> for Form<T> {
    type Rejection = FormRejection;

    async fn from_request(_state: &State, request: &Request<'_>) -> Result<Form<T>, FormRejection> {
        super::url_encoded::deserialize_url_encoded_form(
            core::str::from_utf8(request.body)
                .map_err(|core::str::Utf8Error { .. }| FormRejection::BodyIsNotUtf8)?,
        )
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

impl<S, T: FromRef<S>> FromRequest<S> for State<T> {
    type Rejection = core::convert::Infallible;

    async fn from_request(state: &S, _request: &Request<'_>) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}
