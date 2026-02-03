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

/// [`RequestHandlerService`] that serves a single file.
#[derive(Debug, Clone)]
pub struct File {
    content_type: &'static str,
    body: &'static [u8],
    etag: ETag,
    headers: &'static [(&'static str, &'static str)],
}

impl File {
    pub const MIME_HTML: &'static str = "text/html; charset=utf-8";
    pub const MIME_CSS: &'static str = "text/css";
    pub const MIME_JS: &'static str = "application/javascript; charset=utf-8";

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
        Self::with_content_type(Self::MIME_HTML, body.as_bytes())
    }

    /// Cascading StyleSheets file with a MIME type of "text/css"
    pub const fn css(body: &'static str) -> Self {
        Self::with_content_type(Self::MIME_CSS, body.as_bytes())
    }

    /// A Javascript file with a MIME type of "application/javascript; charset=utf-8"
    pub const fn javascript(body: &'static str) -> Self {
        Self::with_content_type(Self::MIME_JS, body.as_bytes())
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
        struct FileContent<'a>(&'a File);

        impl super::Content for FileContent<'_> {
            fn content_type(&self) -> &'static str {
                self.0.content_type
            }

            fn content_length(&self) -> usize {
                self.0.body.len()
            }

            async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
                writer.write_all(self.0.body).await
            }
        }

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

        super::Response::ok(FileContent(self))
            .with_headers(self.headers)
            .with_headers(self.etag.clone())
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

/// [`PathRouter`] that serves a single file based on the request path.
#[derive(Debug, Default)]
pub struct Directory {
    /// The files in the directory.
    pub files: &'static [(&'static str, File)],

    /// Subdirectories inside this directory.
    pub sub_directories: &'static [(&'static str, Directory)],
}

impl Directory {
    pub const DEFAULT: Self = Self {
        files: &[],
        sub_directories: &[],
    };

    fn matching_file(&self, path: crate::request::Path) -> Option<&File> {
        let found_file = self.files.iter().find_map(|(name, file)| {
            if let Some(crate::request::Path(crate::url_encoded::UrlEncodedString(""))) =
                path.strip_slash_and_prefix(name)
            {
                Some(file)
            } else {
                None
            }
        });

        found_file.or_else(|| {
            self.sub_directories
                .iter()
                .find_map(|(name, sub_directory)| {
                    sub_directory.matching_file(path.strip_slash_and_prefix(name)?)
                })
        })
    }
}

impl<State, CurrentPathParameters> PathRouterService<State, CurrentPathParameters> for Directory {
    async fn call_path_router_service<R: Read, W: super::ResponseWriter<Error = R::Error>>(
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
