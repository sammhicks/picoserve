//! Static files and directories.

use core::fmt;

use crate::{
    io::{Read, Write},
    request::Path,
    routing::{PathRouter, PathRouterService, RequestHandler, RequestHandlerService},
    ResponseSent,
};

use super::{IntoResponse, StatusCode};

#[derive(Clone, PartialEq, Eq)]
struct ETag([u8; 20]);

impl fmt::Debug for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ETag({self})")
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, "\"")?;

        Ok(())
    }
}

impl PartialEq<[u8]> for ETag {
    fn eq(&self, other: &[u8]) -> bool {
        struct Eq;

        fn eq(self_bytes: &[u8], other_str_bytes: &[u8]) -> Option<Eq> {
            fn decode_hex_nibble(c: u8) -> Option<u8> {
                Some(match c {
                c @ b'0'..=b'9' => c - b'0',
                c @ b'a'..=b'f' => 10 + c - b'a',
                c @ b'A'..=b'F' => 10 + c - b'A',
                _ => return None,
            })
        }

            let mut other_str_bytes = other_str_bytes
                .strip_prefix(b"\"")?
                .strip_suffix(b"\"")?
                .iter()
                .copied();

            for &self_byte in self_bytes {
                let other_byte0 = decode_hex_nibble(other_str_bytes.next()?)?;
                let other_byte1 = decode_hex_nibble(other_str_bytes.next()?)?;

                let other_byte = 0x10 * other_byte0 + other_byte1;

                if self_byte != other_byte {
                    return None;
            }
        }

            other_str_bytes.next().is_none().then_some(Eq)
        }

        matches!(eq(&self.0, other), Some(Eq))
    }
}

impl PartialEq<&[u8]> for ETag {
    fn eq(&self, other: &&[u8]) -> bool {
        *self == **other
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
    headers: &'static [(&'static str, &'static str)],
}

impl File {
    /// Create a file with the given content type but no additional headers.
    pub const fn with_content_type(content_type: &'static str, body: &'static [u8]) -> Self {
        Self {
            content_type,
            body,
            etag: ETag(const_sha1::sha1(body).as_bytes()),
            headers: &[],
        }
    }

    /// Create a file with the given content type and some additional headers.
    pub const fn with_content_type_and_headers(
        content_type: &'static str,
        body: &'static [u8],
        headers: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            content_type,
            body,
            etag: ETag(const_sha1::sha1(body).as_bytes()),
            headers,
        }
    }

    /// A HyperText Markup Language file with a MIME type of "text/html; charset=utf-8"
    pub const fn html(body: &'static str) -> Self {
        Self::with_content_type("text/html; charset=utf-8", body.as_bytes())
    }

    /// Cascading StyleSheets file with a MIME type of "text/css"
    pub const fn css(body: &'static str) -> Self {
        Self::with_content_type("text/css", body.as_bytes())
    }

    /// A Javascript file with a MIME type of "application/javascript; charset=utf-8"
    pub const fn javascript(body: &'static str) -> Self {
        Self::with_content_type("application/javascript; charset=utf-8", body.as_bytes())
    }

    /// Convert into a [super::Response] with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        let etag = self.etag.clone();
        let headers = self.headers;
        super::Response::ok(self)
            .with_headers(headers)
            .with_headers(etag)
    }
}

impl<State, PathParameters> crate::routing::RequestHandlerService<State, PathParameters> for File {
    async fn call_request_handler_service<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        _path_parameters: PathParameters,
        request: crate::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        if let Some(if_none_match) = request.parts.headers().get("If-None-Match") {
            if if_none_match
                .split(b',')
                .any(|etag| self.etag == etag.as_raw())
            {
                return response_writer
                    .write_response(
                        request.body_connection.finalize().await?,
                        super::Response {
                            status_code: StatusCode::NOT_MODIFIED,
                            headers: self.etag.clone(),
                            body: super::NoBody,
                        },
                    )
                    .await;
            }
        }

        self.clone()
            .write_to(request.body_connection.finalize().await?, response_writer)
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

    async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        writer.write_all(self.body).await
    }
}

impl super::IntoResponse for File {
    async fn write_to<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        response_writer
            .write_response(connection, self.into_response())
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
    /// The files in the directory.
    pub files: &'static [(&'static str, File)],

    /// Subdirectories inside this directory.
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

impl<State, CurrentPathParameters> PathRouterService<State, CurrentPathParameters> for Directory {
    async fn call_request_handler_service<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: crate::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        if !request.parts.method().eq_ignore_ascii_case("get") {
            return crate::routing::MethodNotAllowed
                .call_request_handler(state, current_path_parameters, request, response_writer)
                .await;
        }

        if let Some(file) = self.matching_file(path) {
            file.call_request_handler_service(
                state,
                current_path_parameters,
                request,
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
                    response_writer,
                )
                .await
        }
    }
}
