//! Static files and directories

use core::fmt;

use crate::{
    io::{Read, Write},
    request::{self, Path},
    routing::{PathRouter, RequestHandler},
    ResponseSent,
};

use super::{status, IntoResponse};

#[derive(Clone, PartialEq, Eq)]
struct ETag([u8; 20]);

impl fmt::Debug for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ETag({self})")
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }

        Ok(())
    }
}

impl<'a> PartialEq<&'a str> for ETag {
    fn eq(&self, other: &&str) -> bool {
        fn decode_hex_nibble(c: Option<char>) -> Option<u8> {
            u8::from_str_radix(c?.encode_utf8(&mut [0; 4]), 16).ok()
        }

        let mut chars = other.chars();

        for b in self.0 {
            let Some(c0) = decode_hex_nibble(chars.next()) else {
                return false;
            };
            let Some(c1) = decode_hex_nibble(chars.next()) else {
                return false;
            };

            let c = 0x10 * c0 + c1;

            if b != c {
                return false;
            }
        }

        true
    }
}

impl super::HeadersIter for ETag {
    async fn for_each_header<F: super::ForEachHeader>(
        self,
        mut f: F,
    ) -> Result<F::Output, F::Error> {
        f.call("ETag", self).await?;
        f.finalize().await
    }
}

/// [RequestHandler] that serves a single file.
#[derive(Debug, Clone)]
pub struct File {
    content_type: &'static str,
    body: &'static [u8],
    etag: ETag,
}

impl File {
    pub const fn with_content_type(content_type: &'static str, body: &'static [u8]) -> Self {
        Self {
            content_type,
            body,
            etag: ETag(const_sha1::sha1(body).as_bytes()),
        }
    }

    pub const fn html(body: &'static str) -> Self {
        Self::with_content_type("text/html; charset=utf-8", body.as_bytes())
    }

    pub const fn css(body: &'static str) -> Self {
        Self::with_content_type("text/css", body.as_bytes())
    }

    pub const fn javascript(body: &'static str) -> Self {
        Self::with_content_type("application/javascript; charset=utf-8", body.as_bytes())
    }

    /// Convert into a [super::Response] with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        let etag = self.etag.clone();
        super::Response::ok(self).with_headers(etag)
    }
}

impl<State, PathParameters> crate::routing::RequestHandler<State, PathParameters> for File {
    async fn call_request_handler<
        R: Read,
        WW: Write<Error = R::Error>,
        W: super::ResponseWriter,
    >(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: request::Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        if let Some(if_none_match) = request.headers().get("If-None-Match") {
            if if_none_match
                .split(',')
                .map(str::trim)
                .any(|etag| self.etag == etag)
            {
                return response_writer
                    .write_response(
                        writer,
                        super::Connection(body_reader),
                        super::Response {
                            status_code: status::NOT_MODIFIED,
                            headers: self.etag.clone(),
                            body: super::NoBody,
                        },
                    )
                    .await;
            }
        }

        self.clone()
            .write_to(writer, super::Connection(body_reader), response_writer)
            .await
    }
}

impl super::Content for File {
    fn content_type(&self) -> &'static str {
        self.content_type
    }

    fn content_length(&self) -> usize {
        self.body.len()
    }

    async fn write_content<R: Read, W: Write>(
        self,
        _connection: super::Connection<R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.write_all(self.body).await
    }
}

impl super::IntoResponse for File {
    async fn write_to<R: Read, W: super::ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: super::Connection<R>,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        response_writer
            .write_response(writer, connection, self.into_response())
            .await
    }
}

impl core::future::IntoFuture for File {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}

/// [PathRouter] that serves a single file.
#[derive(Debug, Default)]
pub struct Directory {
    pub files: &'static [(&'static str, File)],
    pub sub_directories: &'static [(&'static str, Directory)],
}

impl Directory {
    fn matching_file(&self, path: crate::request::Path) -> Option<&File> {
        for (name, file) in self.files.iter() {
            if let Some(crate::request::Path(crate::url_encoded::UrlEncodedString(""))) =
                path.strip_slash_and_prefix(name)
            {
                return Some(file);
            } else {
                continue;
            }
        }

        for (name, sub_directory) in self.sub_directories.iter() {
            if let Some(path) = path.strip_slash_and_prefix(name) {
                return sub_directory.matching_file(path);
            } else {
                continue;
            }
        }

        None
    }
}

impl<State, CurrentPathParameters> PathRouter<State, CurrentPathParameters> for Directory {
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: super::ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: request::Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        if !request.method().eq_ignore_ascii_case("get") {
            return crate::routing::MethodNotAllowed
                .call_request_handler(
                    state,
                    current_path_parameters,
                    request,
                    body_reader,
                    writer,
                    response_writer,
                )
                .await;
        }

        if let Some(file) = self.matching_file(request.path()) {
            file.call_request_handler(
                state,
                current_path_parameters,
                request,
                body_reader,
                writer,
                response_writer,
            )
            .await
        } else {
            crate::routing::NotFound
                .call_path_router(
                    state,
                    current_path_parameters,
                    path,
                    request,
                    body_reader,
                    writer,
                    response_writer,
                )
                .await
        }
    }
}
