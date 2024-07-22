//! Responses with a body that doesn't match a regular HTTP response.
//!
//! For example responses which send a partial response body, wait some amount of time, and send more data later on.
//!
//! Care should be taken when sending custom responses.
//! For example, the Content-Length and Content-Type headers are not automatically sent,
//! and thus must be manually included if need be.
//!
//! The only header that is automatically send is `Connection: close`

use core::marker::PhantomData;

use super::HeadersIter;

/// The headers that are sent with every custom response.
pub struct CustomHeaders;

impl HeadersIter for CustomHeaders {
    async fn for_each_header<F: super::ForEachHeader>(
        self,
        mut f: F,
    ) -> Result<F::Output, F::Error> {
        f.call("Connection", "close").await?;
        f.finalize().await
    }
}

/// The body of a custom response.
/// The writer is automatically flushed after `write_response_body` is called, but intermediate content must be manually flushed.
pub trait CustomBody {
    async fn write_response_body<W: crate::io::Write>(self, writer: W) -> Result<(), W::Error>;
}

/// A custom response.
pub struct CustomResponse<H: HeadersIter, B: CustomBody> {
    status_code: super::StatusCode,
    headers: H,
    body: B,
}

impl<B: CustomBody> CustomResponse<CustomHeaders, B> {
    pub fn build(status_code: super::StatusCode) -> CustomResponseBuilder<CustomHeaders, B> {
        CustomResponseBuilder {
            status_code,
            headers: CustomHeaders,
            _body: PhantomData,
        }
    }
}

impl<H: HeadersIter, B: CustomBody> super::IntoResponse for CustomResponse<H, B> {
    async fn write_to<R: embedded_io_async::Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        struct Body<B: CustomBody> {
            body: B,
        }

        impl<B: CustomBody> super::Body for Body<B> {
            async fn write_response_body<
                R: crate::io::Read,
                W: crate::io::Write<Error = R::Error>,
            >(
                self,
                connection: crate::response::Connection<'_, R>,
                mut writer: W,
            ) -> Result<(), W::Error> {
                connection
                    .run_until_disconnection((), async {
                        self.body.write_response_body(&mut writer).await?;
                        writer.flush().await
                    })
                    .await
            }
        }

        let Self {
            status_code,
            headers,
            body,
        } = self;

        response_writer
            .write_response(
                connection,
                super::Response {
                    status_code,
                    headers,
                    body: Body { body },
                },
            )
            .await
    }
}

/// Build a custom response.
pub struct CustomResponseBuilder<H: HeadersIter, B: CustomBody> {
    status_code: super::StatusCode,
    headers: H,
    _body: PhantomData<B>,
}

impl<H: HeadersIter, B: CustomBody> CustomResponseBuilder<H, B> {
    /// Add a header to the response.
    pub fn with_header<Value: core::fmt::Display>(
        self,
        name: &'static str,
        value: Value,
    ) -> CustomResponseBuilder<impl HeadersIter, B> {
        self.with_headers((name, value))
    }

    /// Add a list of headers to the response.
    pub fn with_headers<HS: HeadersIter>(
        self,
        headers: HS,
    ) -> CustomResponseBuilder<impl HeadersIter, B> {
        let Self {
            status_code,
            headers: current_headers,
            _body,
        } = self;

        CustomResponseBuilder {
            status_code,
            headers: super::HeadersChain(current_headers, headers),
            _body,
        }
    }

    /// Add the body to the response and finish building.
    pub fn with_body(self, body: B) -> CustomResponse<H, B> {
        let Self {
            status_code,
            headers,
            _body,
        } = self;

        CustomResponse {
            status_code,
            headers,
            body,
        }
    }
}
