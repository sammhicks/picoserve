//! HTTP response types.
//!
//! Anything that implements [IntoResponse] can be returned from handlers, such as
//!
//! + [Response]
//! + [Json]
//! + [Redirect]
//! + `(("HeaderName", "HeaderValue"), impl Content)`
//! + `(("HeaderName0", "HeaderValue0"), ("HeaderName1", "HeaderValue1"), impl Content)`
//! + `([("HeaderName0", "HeaderValue0"), ("HeaderName1", "HeaderValue1")], impl Content)`
//! + `([StatusCode], impl Content)`
//! + `([StatusCode], ("HeaderName", "HeaderValue"), impl Content)`
//! + Tuples consisting of:
//!     1. Optionally, a status code. If not provided, a status code of [StatusCode::OK] is used
//!     2. A number of values which implement [HeadersIter], such as:
//!         + `(&str, impl Display)`
//!         + `Option<impl HeadersIter>`
//!         + `[impl HeadersIter; N]`
//!     3. A value which implements [Content]
//!
//! For a complete list, see [IntoResponse].

use core::fmt;

use crate::{
    io::{Read, Write},
    KeepAlive, ResponseSent,
};

pub mod chunked;
pub mod custom;
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

struct MeasureFormatSize<'a>(&'a mut usize);

impl<'a> fmt::Write for MeasureFormatSize<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        *self.0 += s.len();

        Ok(())
    }
}

pub(crate) struct BufferedReader<'r, R: Read> {
    pub(crate) reader: R,
    pub(crate) buffer: &'r mut [u8],
    pub(crate) read_position: usize,
    pub(crate) buffer_usage: usize,
}

impl<'r, R: Read> BufferedReader<'r, R> {
    async fn read_into(&mut self, buffer: &mut [u8]) -> Result<usize, R::Error> {
        let prefix = &self.buffer[self.read_position..self.buffer_usage];

        if prefix.is_empty() {
            self.reader.read(buffer).await
        } else {
            let read_size = prefix.len().min(buffer.len());

            buffer[..read_size].copy_from_slice(prefix);
            self.read_position += read_size;

            Ok(read_size)
        }
    }
}

/// A connection which has been upgraded, and is thus allowed to read arbitary data from the socket.
pub struct UpgradedConnection<'r, R: Read> {
    reader: BufferedReader<'r, R>,
}

impl<'r, R: Read> crate::io::ErrorType for UpgradedConnection<'r, R> {
    type Error = R::Error;
}

impl<'r, R: Read> Read for UpgradedConnection<'r, R> {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        self.reader.read_into(buffer).await
    }
}

/// A handle to the current conneection. Allows a long-lasting response to check if the client has disconnected.
pub struct Connection<'r, R: Read> {
    pub(crate) reader: BufferedReader<'r, R>,
    pub(crate) has_been_upgraded: &'r mut bool,
}

impl<'r, R: Read> Connection<'r, R> {
    /// Upgrade the connection and get access to the inner reader
    pub fn upgrade(
        self,
        _upgrade_token: crate::extract::UpgradeToken,
    ) -> UpgradedConnection<'r, R> {
        *self.has_been_upgraded = true;

        UpgradedConnection {
            reader: self.reader,
        }
    }

    /// Wait for the client to disconnect. This will discard any additional data sent by the client.
    pub async fn wait_for_disconnection(self) -> Result<(), R::Error> {
        crate::extract::UpgradeToken::discard_all_data(self).await
    }

    pub async fn run_until_disconnection<T>(
        self,
        default: T,
        action: impl core::future::Future<Output = Result<T, R::Error>>,
    ) -> Result<T, R::Error> {
        futures_util::future::select(
            core::pin::pin!(async {
                self.wait_for_disconnection().await?;
                Ok(default)
            }),
            core::pin::pin!(action),
        )
        .await
        .factor_first()
        .0
    }
}

pub(crate) struct EmptyReader<E: crate::io::Error>(core::marker::PhantomData<E>);

impl<E: crate::io::Error> crate::io::ErrorType for EmptyReader<E> {
    type Error = E;
}

impl<E: crate::io::Error> crate::io::Read for EmptyReader<E> {
    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(0)
    }
}

impl<'r, E: crate::io::Error> Connection<'r, EmptyReader<E>> {
    pub(crate) fn empty(has_been_upgraded: &'r mut bool) -> Self {
        Self {
            reader: BufferedReader {
                reader: EmptyReader(core::marker::PhantomData),
                buffer: &mut [],
                read_position: 0,
                buffer_usage: 0,
            },
            has_been_upgraded,
        }
    }
}

#[doc(hidden)]
pub trait ForEachHeader {
    type Output;
    type Error;

    async fn call<Value: fmt::Display>(
        &mut self,
        name: &str,
        value: Value,
    ) -> Result<(), Self::Error>;

    async fn finalize(self) -> Result<Self::Output, Self::Error>;
}

struct BorrowedForEachHeader<'a, F: ForEachHeader>(&'a mut F);

impl<'a, F: ForEachHeader> ForEachHeader for BorrowedForEachHeader<'a, F> {
    type Output = ();
    type Error = F::Error;

    async fn call<Value: fmt::Display>(
        &mut self,
        name: &str,
        value: Value,
    ) -> Result<(), F::Error> {
        self.0.call(name, value).await
    }

    async fn finalize(self) -> Result<Self::Output, Self::Error> {
        Ok(())
    }
}

/// The HTTP response headers.
pub trait HeadersIter {
    /// Perform the following action for each header.
    async fn for_each_header<F: ForEachHeader>(self, f: F) -> Result<F::Output, F::Error>;
}

impl<'a, V: fmt::Display> HeadersIter for (&'a str, V) {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<F::Output, F::Error> {
        let (name, value) = self;
        f.call(name, value).await?;
        f.finalize().await
    }
}

impl<'a, 'b, V: fmt::Display> HeadersIter for &'a [(&'b str, V)] {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<F::Output, F::Error> {
        for (name, value) in self {
            f.call(name, value).await?;
        }
        f.finalize().await
    }
}

impl<H: HeadersIter, const N: usize> HeadersIter for [H; N] {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<F::Output, F::Error> {
        for headers in self {
            headers
                .for_each_header(BorrowedForEachHeader(&mut f))
                .await?;
        }
        f.finalize().await
    }
}

impl<T: HeadersIter> HeadersIter for Option<T> {
    async fn for_each_header<F: ForEachHeader>(self, f: F) -> Result<F::Output, F::Error> {
        if let Some(headers) = self {
            headers.for_each_header(f).await
        } else {
            f.finalize().await
        }
    }
}

struct HeadersChain<A: HeadersIter, B: HeadersIter>(A, B);

impl<A: HeadersIter, B: HeadersIter> HeadersIter for HeadersChain<A, B> {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<F::Output, F::Error> {
        let Self(a, b) = self;
        a.for_each_header(BorrowedForEachHeader(&mut f)).await?;
        b.for_each_header(BorrowedForEachHeader(&mut f)).await?;
        f.finalize().await
    }
}

/// The HTTP response body.
pub trait Body {
    /// Write the response body to the socket.
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        writer: W,
    ) -> Result<(), W::Error>;
}

struct NoBody;

impl Body for NoBody {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        _connection: Connection<'_, R>,
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
    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error>;
}

#[doc(hidden)]
pub struct ContentBody<C: Content>(C);

impl<C: Content> Body for ContentBody<C> {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        _connection: Connection<'_, R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        self.0.write_content(&mut writer).await?;
        writer.flush().await?;
        Ok(())
    }
}

impl<'a> Content for &'a [u8] {
    fn content_type(&self) -> &'static str {
        "application/octet-stream"
    }

    fn content_length(&self) -> usize {
        self.len()
    }

    async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        writer.write_all(self).await
    }
}

#[cfg(feature = "alloc")]
impl Content for alloc::vec::Vec<u8> {
    fn content_type(&self) -> &'static str {
        self.as_slice().content_type()
    }

    fn content_length(&self) -> usize {
        self.as_slice().content_length()
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.as_slice().write_content(writer).await
    }
}

impl<'a> Content for &'a str {
    fn content_type(&self) -> &'static str {
        "text/plain; charset=utf-8"
    }

    fn content_length(&self) -> usize {
        self.len()
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.as_bytes().write_content(writer).await
    }
}

#[cfg(feature = "alloc")]
impl Content for alloc::string::String {
    fn content_type(&self) -> &'static str {
        self.as_str().content_type()
    }

    fn content_length(&self) -> usize {
        self.as_str().content_length()
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.as_str().write_content(writer).await
    }
}

impl<'a> Content for fmt::Arguments<'a> {
    fn content_type(&self) -> &'static str {
        "".content_type()
    }

    fn content_length(&self) -> usize {
        use fmt::Write;
        let mut size = 0;
        write!(MeasureFormatSize(&mut size), "{self}").map_or(0, |()| size)
    }

    async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        use crate::io::WriteExt;
        write!(writer, "{}", self).await
    }
}

#[doc(hidden)]
pub struct ContentHeaders {
    content_type: &'static str,
    content_length: usize,
}

impl ContentHeaders {
    fn new(body: &impl Content) -> Self {
        Self {
            content_type: body.content_type(),
            content_length: body.content_length(),
        }
    }
}

impl HeadersIter for ContentHeaders {
    async fn for_each_header<F: ForEachHeader>(self, mut f: F) -> Result<F::Output, F::Error> {
        f.call("Content-Type", self.content_type).await?;
        f.call("Content-Length", self.content_length).await?;
        f.finalize().await
    }
}

/// Represents a HTTP response.
pub struct Response<H: HeadersIter, B: Body> {
    pub(crate) status_code: StatusCode,
    pub(crate) headers: H,
    pub(crate) body: B,
}

impl<B: Content> Response<ContentHeaders, ContentBody<B>> {
    /// Creates a response from a HTTP status code and body with content. The Content-Type and Content-Length headers are generated from the values returned by the Body.
    pub fn new(status_code: StatusCode, body: B) -> Self {
        Self {
            status_code,
            headers: ContentHeaders::new(&body),
            body: ContentBody(body),
        }
    }

    /// A response with a status of 200 "OK".
    pub fn ok(body: B) -> Self {
        Self::new(StatusCode::OK, body)
    }
}

impl<H: HeadersIter, B: Body> Response<H, B> {
    /// Get the status code of the response.
    pub fn status_code(&self) -> StatusCode {
        self.status_code
    }

    /// Return a new response with the given status code.
    pub fn with_status_code(self, status_code: StatusCode) -> Self {
        let Self {
            status_code: _,
            headers,
            body,
        } = self;

        Self {
            status_code,
            headers,
            body,
        }
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
    /// Errors arising while writing the response.
    type Error;

    /// Write the given response to the socket, which may include the upgraded data, which thus may read from the provided connenction.
    async fn write_response<R: Read<Error = Self::Error>, H: HeadersIter, B: Body>(
        self,
        connection: Connection<'_, R>,
        response: Response<H, B>,
    ) -> Result<ResponseSent, Self::Error>;
}

pub(crate) struct ResponseStream<W: Write> {
    writer: W,
    connection_header: super::KeepAlive,
}

impl<W: Write> ResponseStream<W> {
    pub fn new(writer: W, connection_header: super::KeepAlive) -> Self {
        Self {
            writer,
            connection_header,
        }
    }
}

impl<W: Write> ResponseWriter for ResponseStream<W> {
    type Error = W::Error;

    async fn write_response<R: Read<Error = Self::Error>, H: HeadersIter, B: Body>(
        mut self,
        connection: Connection<'_, R>,
        Response {
            status_code,
            headers,
            body,
        }: Response<H, B>,
    ) -> Result<ResponseSent, Self::Error> {
        struct HeadersWriter<WW: Write> {
            writer: WW,
            connection_header: Option<KeepAlive>,
        }

        impl<WW: Write> ForEachHeader for HeadersWriter<WW> {
            type Output = ();
            type Error = WW::Error;

            async fn call<Value: fmt::Display>(
                &mut self,
                name: &str,
                value: Value,
            ) -> Result<(), Self::Error> {
                if name.eq_ignore_ascii_case("connection") {
                    self.connection_header = None;
                }
                write!(self.writer, "{name}: {value}\r\n").await
            }

            async fn finalize(mut self) -> Result<(), Self::Error> {
                if let Some(connection_header) = self.connection_header {
                    self.call("Connection", connection_header).await?;
                }

                Ok(())
            }
        }

        use crate::io::WriteExt;
        write!(self.writer, "HTTP/1.1 {status_code}\r\n").await?;

        headers
            .for_each_header(HeadersWriter {
                writer: &mut self.writer,
                connection_header: Some(self.connection_header),
            })
            .await?;

        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await?;

        body.write_response_body(connection, &mut self.writer)
            .await
            .map(super::ResponseSent)
    }
}

/// Trait for generating responses.
///
/// Types that implement IntoResponse can be returned from handlers.
pub trait IntoResponse: Sized {
    /// Write the generated response into the given [ResponseWriter].
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

impl<H: HeadersIter, B: Body> IntoResponse for Response<H, B> {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer.write_response(connection, self).await
    }
}

impl IntoResponse for core::convert::Infallible {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        _connection: Connection<'_, R>,
        _response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {}
    }
}

impl IntoResponse for () {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        "OK\n".write_to(connection, response_writer).await
    }
}

impl<'a> IntoResponse for &'a str {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer
            .write_response(connection, Response::ok(self))
            .await
    }
}

impl<'a> IntoResponse for fmt::Arguments<'a> {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer
            .write_response(connection, Response::ok(self))
            .await
    }
}

impl<const N: usize> IntoResponse for heapless::String<N> {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.as_str().write_to(connection, response_writer).await
    }
}

#[cfg(feature = "std")]
impl IntoResponse for std::string::String {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.as_str().write_to(connection, response_writer).await
    }
}

impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            Ok(value) => value.write_to(connection, response_writer).await,
            Err(err) => err.write_to(connection, response_writer).await,
        }
    }
}

macro_rules! declare_tuple_into_response {
    ($($($name:ident)*;)*) => {
        $(
            impl<$($name: HeadersIter,)* C: Content> IntoResponse for (StatusCode, $($name,)* C,) {
                #[allow(non_snake_case)]
                async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(self, connection: Connection<'_, R>, response_writer: W) -> Result<ResponseSent, W::Error> {
                    let (status_code, $($name,)* body) = self;

                    response_writer.write_response(
                        connection,
                        Response::new(status_code, body)
                        $(.with_headers($name,))*
                    ).await
                }
            }

            impl<$($name: HeadersIter,)* C: Content> IntoResponse for ($($name,)* C,) {
                #[allow(non_snake_case)]
                async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(self, connection: Connection<'_, R>, response_writer: W) -> Result<ResponseSent, W::Error> {
                    let ($($name,)* body,) = self;

                    response_writer.write_response(
                        connection,
                        Response::new(StatusCode::OK, body)
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
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer
            .write_response(connection, Response::ok(format_args!("{:?}\r\n", self.0)))
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
            status_code: StatusCode::SEE_OTHER,
            location,
        }
    }
}

impl IntoResponse for Redirect {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            self.status_code,
            ("Location", self.location),
            format_args!("{}\n", self.location),
        )
            .write_to(connection, response_writer)
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
