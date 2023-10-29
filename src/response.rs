//! HTTP response types.

use core::fmt;

use crate::{
    io::{Read, Write},
    ResponseSent,
};

pub mod fs;
pub mod json;
pub mod sse;
pub mod status;
pub mod ws;

pub use fs::{Directory, File};
pub use json::Json;
pub use sse::EventStream;
pub use status::StatusCode;
pub use ws::WebSocketUpgrade;

struct MeasureFormatSize(pub usize);

impl fmt::Write for MeasureFormatSize {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();

        Ok(())
    }
}

/// A handle to the current conneection. Allows a long-lasting response to check if the client has disconnected.
pub struct Connection<R: Read>(pub(crate) R);

impl<R: Read> Connection<R> {
    /// Wait for the client to disconnect. This will discard any additional data sent by the client.
    pub async fn wait_for_disconnection(mut self) -> Result<(), R::Error> {
        while self.0.read(&mut [0; 8]).await? > 0 {}

        Ok(())
    }
}

#[doc(hidden)]
pub trait ForEachHeader {
    type Error;

    async fn call<Value: fmt::Display>(
        &mut self,
        name: &str,
        value: Value,
    ) -> Result<(), Self::Error>;
}

struct BorrowedForEachHeader<'a, F: ForEachHeader>(&'a mut F);

impl<'a, F: ForEachHeader> ForEachHeader for BorrowedForEachHeader<'a, F> {
    type Error = F::Error;

    async fn call<Value: fmt::Display>(
        &mut self,
        name: &str,
        value: Value,
    ) -> Result<(), F::Error> {
        self.0.call(name, value).await
    }
}

/// The HTTP response headers.
pub trait HeadersIter {
    async fn for_each_header<F: ForEachHeader>(self, f: F) -> Result<(), F::Error>;
}

impl<'a, V: fmt::Display> HeadersIter for (&'a str, V) {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<(), F::Error> {
        let (name, value) = self;
        f.call(name, value).await
    }
}

impl<'a, V: fmt::Display, const N: usize> HeadersIter for [(&'a str, V); N] {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<(), F::Error> {
        for (name, value) in self {
            f.call(name, value).await?;
        }

        Ok(())
    }
}

impl<T: HeadersIter> HeadersIter for Option<T> {
    async fn for_each_header<F: ForEachHeader>(self, f: F) -> Result<(), F::Error> {
        if let Some(value) = self {
            value.for_each_header(f).await
        } else {
            Ok(())
        }
    }
}

struct NoHeaders;

impl HeadersIter for NoHeaders {
    async fn for_each_header<F: ForEachHeader>(self, _f: F) -> Result<(), F::Error> {
        Ok(())
    }
}

struct HeadersChain<A: HeadersIter, B: HeadersIter>(A, B);

impl<A: HeadersIter, B: HeadersIter> HeadersIter for HeadersChain<A, B> {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<(), F::Error> {
        let Self(a, b) = self;
        a.for_each_header(BorrowedForEachHeader(&mut f)).await?;
        b.for_each_header(BorrowedForEachHeader(&mut f)).await?;
        Ok(())
    }
}

/// The HTTP response body.
pub trait Body {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: Connection<R>,
        writer: W,
    ) -> Result<(), W::Error>;
}

struct NoBody;

impl Body for NoBody {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        _connection: Connection<R>,
        _writer: W,
    ) -> Result<(), W::Error> {
        Ok(())
    }
}

/// A [Response] body containing data with a known type and length.
pub trait Content {
    /// The value of the "Content-Type" header.
    fn content_type(&self) -> &'static str;

    /// The value of the "Content-Length" header.
    fn content_length(&self) -> usize;

    /// Write the content data.
    async fn write_content<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: Connection<R>,
        writer: W,
    ) -> Result<(), W::Error>;
}

#[doc(hidden)]
pub struct ContentBody<C: Content>(C);

impl<C: Content> Body for ContentBody<C> {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: Connection<R>,
        writer: W,
    ) -> Result<(), W::Error> {
        self.0.write_content(connection, writer).await
    }
}

impl<'a> Content for &'a [u8] {
    fn content_type(&self) -> &'static str {
        "application/octet-stream"
    }

    fn content_length(&self) -> usize {
        self.len()
    }

    async fn write_content<R: Read, W: Write>(
        self,
        _connection: Connection<R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.write_all(self).await
    }
}

impl<'a> Content for &'a str {
    fn content_type(&self) -> &'static str {
        "text/plain; charset=utf-8"
    }

    fn content_length(&self) -> usize {
        self.len()
    }

    async fn write_content<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: Connection<R>,
        writer: W,
    ) -> Result<(), W::Error> {
        self.as_bytes().write_content(connection, writer).await
    }
}

impl<'a> Content for fmt::Arguments<'a> {
    fn content_type(&self) -> &'static str {
        "".content_type()
    }

    fn content_length(&self) -> usize {
        use fmt::Write;
        let mut size = MeasureFormatSize(0);
        write!(&mut size, "{self}").map_or(0, |()| size.0)
    }

    async fn write_content<R: Read, W: Write>(
        self,
        _connection: Connection<R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        use crate::io::WriteExt;
        write!(writer, "{}", self).await
    }
}

struct BodyHeaders {
    content_type: &'static str,
    content_length: usize,
}

impl BodyHeaders {
    fn new(body: &impl Content) -> Self {
        Self {
            content_type: body.content_type(),
            content_length: body.content_length(),
        }
    }
}

impl HeadersIter for BodyHeaders {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<(), F::Error> {
        f.call("Content-Type", self.content_type).await?;
        f.call("Content-Length", self.content_length).await?;
        Ok(())
    }
}

/// Represents a HTTP response.
pub struct Response<H: HeadersIter, B: Body> {
    pub(crate) status_code: StatusCode,
    pub(crate) headers: H,
    pub(crate) body: B,
}

impl<B: Content> Response<BodyHeaders, ContentBody<B>> {
    /// Creates a response from a HTTP status code and body with content. The Content-Type and Content-Length headers are generated from the values returned by the Body.
    pub fn new(status_code: StatusCode, body: B) -> Self {
        Self {
            status_code,
            headers: BodyHeaders::new(&body),
            body: ContentBody(body),
        }
    }

    /// A response with a status of 200 "OK".
    pub fn ok(body: B) -> Self {
        Self::new(status::OK, body)
    }
}

impl<H: HeadersIter, B: Body> Response<H, B> {
    pub fn status_code(&self) -> StatusCode {
        self.status_code
    }

    /// Add additional headers to a response.
    pub fn with_headers<HH: HeadersIter>(self, headers: HH) -> Response<impl HeadersIter, B> {
        let Response {
            status_code,
            headers: current_headers,
            body,
        } = self;

        Response {
            status_code,
            headers: HeadersChain(current_headers, headers),
            body,
        }
    }

    /// Add an additional header to a response.
    pub fn with_header<V: fmt::Display>(
        self,
        name: &'static str,
        value: V,
    ) -> Response<impl HeadersIter, B> {
        self.with_headers([(name, value)])
    }
}

/// Types which a HTTP response can be written to.
pub trait ResponseWriter: Sized {
    type Error;

    async fn write_response<H: HeadersIter, B: Body>(
        self,
        response: Response<H, B>,
    ) -> Result<ResponseSent, Self::Error>;
}

pub(crate) struct ResponseStream<R: Read, W: Write> {
    connection: Connection<R>,
    writer: W,
}

impl<R: Read, W: Write<Error = R::Error>> ResponseStream<R, W> {
    pub fn new(connection: Connection<R>, writer: W) -> Self {
        Self { connection, writer }
    }
}

impl<R: Read, W: Write<Error = R::Error>> ResponseWriter for ResponseStream<R, W> {
    type Error = W::Error;

    async fn write_response<H: HeadersIter, B: Body>(
        mut self,
        Response {
            status_code,
            headers,
            body,
        }: Response<H, B>,
    ) -> Result<ResponseSent, W::Error> {
        struct HeadersWriter<WW: Write>(WW);

        impl<WW: Write> ForEachHeader for HeadersWriter<WW> {
            type Error = WW::Error;

            async fn call<Value: fmt::Display>(
                &mut self,
                name: &str,
                value: Value,
            ) -> Result<(), Self::Error> {
                write!(self.0, "{name}: {value}\r\n").await
            }
        }

        use crate::io::WriteExt;
        write!(self.writer, "HTTP/1.1 {status_code}\r\n").await?;

        headers
            .for_each_header(HeadersWriter(&mut self.writer))
            .await?;

        self.writer.write_all(b"\r\n").await?;

        body.write_response_body(self.connection, &mut self.writer)
            .await?;

        self.writer.flush().await.map(ResponseSent)
    }
}

/// Trait for generating responses.
///
/// Types that implement IntoResponse can be returned from handlers.
pub trait IntoResponse: Sized {
    /// Write the generated response into the given [ResponseWriter].
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

impl<H: HeadersIter, B: Body> IntoResponse for Response<H, B> {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer.write_response(self).await
    }
}

impl IntoResponse for core::convert::Infallible {
    async fn write_to<W: ResponseWriter>(
        self,
        _response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {}
    }
}

impl IntoResponse for () {
    #[allow(clippy::let_unit_value)]
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        "OK\n".write_to(response_writer).await
    }
}

impl<'a> IntoResponse for &'a str {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer.write_response(Response::ok(self)).await
    }
}

impl<'a> IntoResponse for fmt::Arguments<'a> {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer.write_response(Response::ok(self)).await
    }
}

impl<const N: usize> IntoResponse for heapless::String<N> {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.as_str().write_to(response_writer).await
    }
}

#[cfg(feature = "std")]
impl IntoResponse for std::string::String {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.as_str().write_to(response_writer).await
    }
}

impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            Ok(value) => value.write_to(response_writer).await,
            Err(err) => err.write_to(response_writer).await,
        }
    }
}

macro_rules! declare_tuple_into_response {
    ($($($name:ident)*;)*) => {
        $(
            impl<$($name: HeadersIter,)* C: Content> IntoResponse for (StatusCode, $($name,)* C,) {
                #[allow(non_snake_case)]
                async fn write_to<W: ResponseWriter>(self, response_writer: W) -> Result<ResponseSent, W::Error> {
                    let (status_code, $($name,)* body) = self;

                    response_writer.write_response(
                        Response::new(status_code, body)
                        $(.with_headers($name,))*
                    ).await
                }
            }

            impl<$($name: HeadersIter,)* C: Content> IntoResponse for ($($name,)* C,) {
                #[allow(non_snake_case)]
                async fn write_to<W: ResponseWriter>(self, response_writer: W) -> Result<ResponseSent, W::Error> {
                    let ($($name,)* body,) = self;

                    response_writer.write_response(
                        Response::new(status::OK, body)
                        $(.with_headers($name,))*
                    ).await
                }
            }
        )*
    };
}

declare_tuple_into_response!(
    ;
    H1;
    H1 H2;
    H1 H2 H3;
    H1 H2 H3 H4;
    H1 H2 H3 H4 H5;
    H1 H2 H3 H4 H5 H6;
    H1 H2 H3 H4 H5 H6 H7;
    H1 H2 H3 H4 H5 H6 H7 H8;
    H1 H2 H3 H4 H5 H6 H7 H8 H9;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12 H13;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12 H13 H14;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12 H13 H14 H15;
    H1 H2 H3 H4 H5 H6 H7 H8 H9 H10 H11 H12 H13 H14 H15 H16;
);

/// Returns a value in [core::fmt::Debug] form as text.
pub struct DebugValue<D>(pub D);

impl<D: fmt::Debug> IntoResponse for DebugValue<D> {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer
            .write_response(Response::ok(format_args!("{:?}\r\n", self.0)))
            .await
    }
}

impl<D: fmt::Debug> core::future::IntoFuture for DebugValue<D> {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}

/// Response that redirects the request to another location.
pub struct Redirect {
    status_code: StatusCode,
    location: &'static str,
}

impl Redirect {
    /// Create a new [Redirect] that uses a 303 "See Other" status code.
    pub fn to(location: &'static str) -> Self {
        Self {
            status_code: status::SEE_OTHER,
            location,
        }
    }
}

impl IntoResponse for Redirect {
    async fn write_to<W: ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            self.status_code,
            ("Location", self.location),
            format_args!("{}\n", self.location),
        )
            .write_to(response_writer)
            .await
    }
}

impl core::future::IntoFuture for Redirect {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}
