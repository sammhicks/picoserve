//! HTTP request types.

use core::{fmt, ops::Range};

use embedded_io_async::Read;

use super::url_encoded::UrlEncodedString;

struct Subslice<'a> {
    buffer: &'a [u8],
    range: Range<usize>,
}

impl<'a> Subslice<'a> {
    fn as_ref(&self) -> &'a [u8] {
        &self.buffer[self.range.clone()]
    }
}

struct RequestLine<S> {
    method: S,
    url: S,
    http_version: S,
}

impl<'a> RequestLine<Subslice<'a>> {
    fn range(&self) -> RequestLine<Range<usize>> {
        let RequestLine {
            method,
            url,
            http_version,
        } = self;

        RequestLine {
            method: method.range.clone(),
            url: url.range.clone(),
            http_version: http_version.range.clone(),
        }
    }

    fn as_str(&self) -> Result<RequestLine<&'a str>, core::str::Utf8Error> {
        let RequestLine {
            method,
            url,
            http_version,
        } = self;

        Ok(RequestLine {
            method: core::str::from_utf8(method.as_ref())?,
            url: core::str::from_utf8(url.as_ref())?,
            http_version: core::str::from_utf8(http_version.as_ref())?,
        })
    }
}

impl RequestLine<Range<usize>> {
    fn index_buffer<'a>(&self, buffer: &'a [u8]) -> RequestLine<Subslice<'a>> {
        let RequestLine {
            method,
            url,
            http_version,
        } = self;

        RequestLine {
            method: Subslice {
                buffer,
                range: method.clone(),
            },
            url: Subslice {
                buffer,
                range: url.clone(),
            },
            http_version: Subslice {
                buffer,
                range: http_version.clone(),
            },
        }
    }
}

fn trim(b: &[u8]) -> &[u8] {
    fn trim_start(mut b: &[u8]) -> &[u8] {
        loop {
            break match b.split_first() {
                Some((head, tail)) if head.is_ascii_whitespace() => {
                    b = tail;
                    continue;
                }
                _ => b,
            };
        }
    }

    fn trim_end(mut b: &[u8]) -> &[u8] {
        loop {
            break match b.split_last() {
                Some((last, rest)) if last.is_ascii_whitespace() => {
                    b = rest;
                    continue;
                }
                _ => b,
            };
        }
    }

    trim_end(trim_start(b))
}

fn eq_ignore_ascii_case(lhs: &[u8], rhs: &[u8]) -> bool {
    if lhs.len() != rhs.len() {
        return false;
    }

    lhs.iter()
        .zip(rhs.iter())
        .all(|(lhs, rhs)| lhs.eq_ignore_ascii_case(rhs))
}

fn escape_debug(data: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    use fmt::Write;

    data.iter().try_for_each(|&b| {
        if b.is_ascii_graphic() {
            f.write_char(b.into())
        } else {
            write!(f, "\\x{b:02x}")
        }
    })
}

#[derive(Clone, PartialEq, Eq)]
pub struct HeaderName<'a> {
    name: &'a [u8],
}

impl<'a> fmt::Debug for HeaderName<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        escape_debug(self.name, f)
    }
}

#[cfg(feature = "defmt")]
impl<'a> defmt::Format for HeaderName<'a> {
    fn format(&self, fmt: defmt::Formatter) {
        self.name.format(fmt)
    }
}

impl<'a> HeaderName<'a> {
    pub fn as_raw(&self) -> &'a [u8] {
        self.name
    }

    pub fn as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.name)
    }
}

impl<'a> PartialEq<str> for HeaderName<'a> {
    fn eq(&self, other: &str) -> bool {
        eq_ignore_ascii_case(self.name, other.as_bytes())
    }
}

impl<'a> PartialEq<&str> for HeaderName<'a> {
    fn eq(&self, other: &&str) -> bool {
        eq_ignore_ascii_case(self.name, other.as_bytes())
    }
}

impl<'a> PartialEq<HeaderName<'a>> for str {
    fn eq(&self, other: &HeaderName<'a>) -> bool {
        *other == self
    }
}

impl<'a> PartialEq<HeaderName<'a>> for &str {
    fn eq(&self, other: &HeaderName<'a>) -> bool {
        *other == *self
    }
}

#[derive(Clone)]
pub struct HeaderValue<'a> {
    pub(crate) value: &'a [u8],
}

impl<'a> fmt::Debug for HeaderValue<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        escape_debug(self.value, f)
    }
}

#[cfg(feature = "defmt")]
impl<'a> defmt::Format for HeaderValue<'a> {
    fn format(&self, fmt: defmt::Formatter) {
        self.value.format(fmt)
    }
}

impl<'a> HeaderValue<'a> {
    pub fn as_raw(&self) -> &'a [u8] {
        self.value
    }

    pub fn as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.value)
    }

    pub fn split(&self, b: u8) -> impl Iterator<Item = HeaderValue<'a>> {
        self.value
            .split(move |&bb| b == bb)
            .map(trim)
            .map(|value| HeaderValue { value })
    }
}

impl<'a> PartialEq<str> for HeaderValue<'a> {
    fn eq(&self, other: &str) -> bool {
        eq_ignore_ascii_case(self.value, other.as_bytes())
    }
}

impl<'a> PartialEq<&str> for HeaderValue<'a> {
    fn eq(&self, other: &&str) -> bool {
        eq_ignore_ascii_case(self.value, other.as_bytes())
    }
}

impl<'a> PartialEq<HeaderValue<'a>> for str {
    fn eq(&self, other: &HeaderValue<'a>) -> bool {
        *other == self
    }
}

impl<'a> PartialEq<HeaderValue<'a>> for &str {
    fn eq(&self, other: &HeaderValue<'a>) -> bool {
        *other == *self
    }
}

pub struct HeadersIter<'a>(&'a [u8]);

impl<'a> Iterator for HeadersIter<'a> {
    type Item = (HeaderName<'a>, HeaderValue<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.0.split_inclusive(|&b| b == b'\n').next()?;

        self.0 = &self.0[line.len()..];

        let colon_position = line
            .iter()
            .copied()
            .enumerate()
            .find_map(|(i, b)| (b == b':').then_some(i))?;

        let name = trim(&line[..colon_position]);

        let value = trim(&line[(colon_position + 1)..]);

        Some((HeaderName { name }, HeaderValue { value }))
    }
}

/// The Request Headers.
#[derive(Clone, Copy)]
pub struct Headers<'a>(&'a [u8]);

impl<'a> Headers<'a> {
    /// Iterator over all headers.
    pub fn iter(&self) -> HeadersIter<'a> {
        HeadersIter(self.0)
    }

    /// Get a header with a name which matches (ignoring ASCII case) the given name
    pub fn get(&self, name: &str) -> Option<HeaderValue<'a>> {
        self.iter()
            .find_map(|(header_name, value)| (name == header_name).then_some(value))
    }
}

impl<'a> IntoIterator for Headers<'a> {
    type Item = (HeaderName<'a>, HeaderValue<'a>);
    type IntoIter = HeadersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, 'b> IntoIterator for &'b Headers<'a> {
    type Item = (HeaderName<'a>, HeaderValue<'a>);
    type IntoIter = HeadersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> fmt::Debug for Headers<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// The URL-encoded path of the request
#[derive(Debug, Clone, Copy)]
pub struct Path<'r>(pub(crate) UrlEncodedString<'r>);

impl<'r> fmt::Display for Path<'r> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.encoded().fmt(f)
    }
}

impl<'r> PartialEq<&'r str> for Path<'r> {
    fn eq(&self, other: &&'r str) -> bool {
        matches!(self.strip_prefix(other), Some(Path(UrlEncodedString(""))))
    }
}

impl<'r> Path<'r> {
    /// Return the encoded string
    pub fn encoded(self) -> &'r str {
        self.0 .0
    }

    pub(crate) fn strip_slash_and_prefix(self, prefix: &str) -> Option<Self> {
        Self(self.0.strip_prefix("/")?).strip_prefix(prefix)
    }

    pub(crate) fn strip_prefix(self, prefix: &str) -> Option<Self> {
        self.0
            .strip_prefix(prefix)
            .filter(|path| path.is_empty() || path.0.starts_with('/'))
            .map(Self)
    }

    /// Split the path into the first segment (everything before the first `/`) and the rest of the path.
    /// If the path is empty, return None.
    pub fn split_first_segment(self) -> Option<(UrlEncodedString<'r>, Path<'r>)> {
        let path = self.encoded().strip_prefix('/')?;

        let (segment, path) = path
            .char_indices()
            .find_map(|(index, c)| (c == '/').then_some(path.split_at(index)))
            .unwrap_or((path, ""));

        Some((UrlEncodedString(segment), Path(UrlEncodedString(path))))
    }

    /// Iterate over the segments of the path, more or less split by `/`
    pub fn segments(self) -> PathSegments<'r> {
        PathSegments(self)
    }
}

impl<'r> IntoIterator for Path<'r> {
    type Item = UrlEncodedString<'r>;
    type IntoIter = PathSegments<'r>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments()
    }
}

/// A path "segment", i.e. the text between two `/`s.
#[derive(Clone)]
pub struct PathSegments<'r>(Path<'r>);

impl<'r> PathSegments<'r> {
    /// Represent a path segment as a path
    pub fn as_path(&self) -> Path<'r> {
        self.0
    }
}

impl<'r> Iterator for PathSegments<'r> {
    type Item = UrlEncodedString<'r>;

    fn next(&mut self) -> Option<Self::Item> {
        let (segment, path) = self.0.split_first_segment()?;
        self.0 = path;
        Some(segment)
    }
}

impl<'r> core::iter::FusedIterator for PathSegments<'r> {}

/// Represents an HTTP request.
#[derive(Debug, Clone, Copy)]
pub struct RequestParts<'r> {
    method: &'r str,
    path: Path<'r>,
    query: Option<UrlEncodedString<'r>>,
    fragments: Option<UrlEncodedString<'r>>,
    http_version: &'r str,
    headers: Headers<'r>,
}

impl<'r> RequestParts<'r> {
    /// Return the method as sent by the client
    pub const fn method(&self) -> &'r str {
        self.method
    }

    /// Return the request path, without the query or fragments
    pub const fn path(&self) -> Path<'r> {
        self.path
    }

    /// Return the query section of the request URL, i.e. everything after the "?"
    pub const fn query(&self) -> Option<UrlEncodedString<'r>> {
        self.query
    }

    /// Return the fragments of the request URL, i.e. everything after the "#"
    pub const fn fragments(&self) -> Option<UrlEncodedString<'r>> {
        self.fragments
    }

    /// Return the HTTP version as sent by the client
    pub const fn http_version(&self) -> &'r str {
        self.http_version
    }

    /// Return the request headers
    pub const fn headers(&self) -> Headers<'r> {
        self.headers
    }
}

/// Reads the body asynchronously. Implements [Read].
pub struct RequestBodyReader<'r, R: Read> {
    content_length: usize,
    reader: &'r mut R,
    current_data: &'r [u8],
    read_position: &'r mut usize,
}

impl<'r, R: Read> crate::io::ErrorType for RequestBodyReader<'r, R> {
    type Error = R::Error;
}

impl<'r, R: Read> RequestBodyReader<'r, R> {
    /// Returns the total length of the body
    pub const fn content_length(&self) -> usize {
        self.content_length
    }
}

impl<'r, R: Read> Read for RequestBodyReader<'r, R> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let read_size = if self.current_data.is_empty() {
            let max_read_size = buf.len().min(self.content_length - *self.read_position);

            if max_read_size == 0 {
                0
            } else {
                self.reader.read(&mut buf[..max_read_size]).await?
            }
        } else {
            let read_size = self.current_data.len().min(buf.len());

            let (current_data, remaining_data) = self.current_data.split_at(read_size);

            buf[..read_size].copy_from_slice(current_data);
            self.current_data = remaining_data;

            read_size
        };

        *self.read_position += read_size;

        Ok(read_size)
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Errors arising when reading the entire body
pub enum ReadAllBodyError<E> {
    /// The body does not fit into the remaining request buffer.
    BufferIsTooSmall,
    /// EndOfFile reached while reading the body before the entire body has been read.
    UnexpectedEof,
    /// The socket failed to read.
    IO(E),
}

/// The body of the request, which may not have yet been buffered.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RequestBody<'r, R: Read> {
    content_length: usize,
    reader: &'r mut R,
    buffer: &'r mut [u8],
    read_position: &'r mut usize,
    buffer_usage: &'r mut usize,
}

impl<'r, R: Read> RequestBody<'r, R> {
    /// The total length of the body
    pub const fn content_length(&self) -> usize {
        self.content_length
    }

    /// The size of the buffer used to read the body into
    pub const fn buffer_length(&self) -> usize {
        self.buffer.len()
    }

    /// Does the entire body fit into the buffer?
    pub const fn entire_body_fits_into_buffer(&self) -> bool {
        self.content_length() <= self.buffer_length()
    }

    /// Read the entire body into the HTTP buffer.
    pub async fn read_all(self) -> Result<&'r mut [u8], ReadAllBodyError<R::Error>> {
        let buffer = self
            .buffer
            .get_mut(..self.content_length)
            .ok_or(ReadAllBodyError::BufferIsTooSmall)?;

        if let Some(remaining_body_to_read) = buffer.get_mut(*self.buffer_usage..) {
            self.reader
                .read_exact(remaining_body_to_read)
                .await
                .map_err(|err| match err {
                    embedded_io_async::ReadExactError::UnexpectedEof => {
                        ReadAllBodyError::UnexpectedEof
                    }
                    embedded_io_async::ReadExactError::Other(err) => ReadAllBodyError::IO(err),
                })?;

            *self.buffer_usage = self.content_length;
        }

        *self.read_position = self.content_length;

        Ok(buffer)
    }

    /// Return a reader which can be used to asynchronously read the body, such as decoding it on the fly or streaming into an external buffer.
    pub fn reader(self) -> RequestBodyReader<'r, R> {
        RequestBodyReader {
            content_length: self.content_length,
            reader: self.reader,
            current_data: &self.buffer[..(self.content_length.min(*self.buffer_usage))],
            read_position: self.read_position,
        }
    }
}

/// The connection reading the request body. Can be used to read the request body and then extract the underlying connection for reading further data,
/// such as if the connenction has been upgraded.
pub struct RequestBodyConnection<'r, R: Read> {
    content_length: usize,
    reader: &'r mut R,
    read_position: usize,
    buffer: &'r mut [u8],
    buffer_usage: usize,
    has_been_upgraded: &'r mut bool,
}

impl<'r, R: Read> RequestBodyConnection<'r, R> {
    /// Return the total length of the body
    pub const fn content_length(&self) -> usize {
        self.content_length
    }

    /// Return the Request Body
    pub fn body(&mut self) -> RequestBody<'_, R> {
        RequestBody {
            content_length: self.content_length,
            reader: self.reader,
            read_position: &mut self.read_position,
            buffer: self.buffer,
            buffer_usage: &mut self.buffer_usage,
        }
    }

    /// "Finalize" the connection, reading and discarding the rest of the body if need be, and returning the underlying connection
    pub async fn finalize(
        self,
    ) -> Result<crate::response::Connection<'r, impl Read<Error = R::Error> + 'r>, R::Error> {
        // If the entire body is already in the buffer
        if self.content_length <= self.buffer_usage {
            return Ok(crate::response::Connection {
                reader: crate::response::BufferedReader {
                    reader: self.reader,
                    buffer: self.buffer,
                    read_position: self.content_length,
                    buffer_usage: self.buffer_usage,
                },
                has_been_upgraded: self.has_been_upgraded,
            });
        }

        // Data after the body has not yet been read, the entire buffer can be used to read the rest of the body

        // Skip the section that has already been read into the buffer
        let mut read_position = self.read_position.max(self.buffer_usage);

        while let Some(data_remaining) = self
            .content_length
            .checked_sub(read_position)
            .and_then(core::num::NonZeroUsize::new)
        {
            let read_buffer_size = data_remaining.get().min(self.buffer.len());

            let read_size = self
                .reader
                .read(&mut self.buffer[..read_buffer_size])
                .await?;

            if read_size == 0 {
                break;
            }

            read_position += read_size;
        }

        Ok(crate::response::Connection {
            reader: crate::response::BufferedReader {
                reader: self.reader,
                buffer: self.buffer,
                read_position: 0,
                buffer_usage: 0,
            },
            has_been_upgraded: self.has_been_upgraded,
        })
    }
}

/// A HTTP Request
pub struct Request<'r, R: Read> {
    /// The method, path, query, fragments, and headers.
    pub parts: RequestParts<'r>,
    /// The request body and underlying connection
    pub body_connection: RequestBodyConnection<'r, R>,
}

/// Errors arising while reading a HTTP Request
pub(crate) enum ReadError<E> {
    /// The request line is invalid
    BadRequestLine,
    /// A Header line does not contain a ':'
    HeaderDoesNotContainColon,
    /// EndOfFile before the end of the request line or headers
    UnexpectedEof,
    /// IO Error
    IO(E),
}

pub(crate) struct Reader<'b, R: Read> {
    reader: R,
    read_position: usize,
    buffer: &'b mut [u8],
    buffer_usage: usize,
    has_been_upgraded: bool,
}

impl<'b, R: Read> Reader<'b, R> {
    pub fn new(reader: R, buffer: &'b mut [u8]) -> Self {
        Self {
            reader,
            read_position: 0,
            buffer,
            buffer_usage: 0,
            has_been_upgraded: false,
        }
    }

    fn wind_buffer_to_start(&mut self) {
        if let Some(used_buffer) = self.buffer.get_mut(..self.buffer_usage) {
            used_buffer.rotate_left(self.read_position);

            self.buffer_usage -= self.read_position;
        } else {
            self.buffer_usage = 0;
        }

        self.read_position = 0;
    }

    pub async fn request_is_pending(&mut self) -> Result<bool, R::Error> {
        if self.has_been_upgraded {
            Ok(false)
        } else {
            self.wind_buffer_to_start();

            if self.buffer_usage > 0 {
                Ok(true)
            } else {
                self.buffer_usage = self.reader.read(self.buffer).await?;
                Ok(self.buffer_usage > 0)
            }
        }
    }

    fn used_buffer(&self) -> &[u8] {
        &self.buffer[..self.buffer_usage]
    }

    async fn next_byte(&mut self) -> Result<u8, ReadError<R::Error>> {
        if self.read_position == self.buffer_usage {
            let read_size = self
                .reader
                .read(&mut self.buffer[self.buffer_usage..])
                .await
                .map_err(ReadError::IO)?;

            if read_size == 0 {
                return Err(ReadError::UnexpectedEof);
            }

            self.buffer_usage += read_size;
        }

        let b = self.used_buffer()[self.read_position];
        self.read_position += 1;

        Ok(b)
    }

    async fn read_line(&mut self) -> Result<Subslice, ReadError<R::Error>> {
        let start_index = self.read_position;

        loop {
            let end_index = self.read_position;
            break if self.next_byte().await? == b'\n' {
                let slice = Subslice {
                    buffer: self.used_buffer(),
                    range: start_index..end_index,
                };

                // log::info!("{}: Line: {:?}", self.id, slice.as_ref());

                Ok(slice)
            } else {
                continue;
            };
        }
    }

    async fn read_request_line(&mut self) -> Result<RequestLine<Subslice>, ReadError<R::Error>> {
        fn slice_from_str<'a>(slice: &Subslice<'a>, s: &str) -> Subslice<'a> {
            let Range { start, end } = s.as_bytes().as_ptr_range();

            let start_index = start as usize - slice.buffer.as_ptr() as usize;
            let end_index = end as usize - slice.buffer.as_ptr() as usize;

            Subslice {
                buffer: slice.buffer,
                range: start_index..end_index,
            }
        }

        let line = self.read_line().await?;

        let mut words = core::str::from_utf8(line.as_ref())
            .map_err(|_| ReadError::BadRequestLine)?
            .split_whitespace()
            .map(str::trim);

        let method = words.next().ok_or(ReadError::BadRequestLine)?;
        let path = words.next().ok_or(ReadError::BadRequestLine)?;
        let http_version = words.next().ok_or(ReadError::BadRequestLine)?;

        if words.next().is_some() {
            return Err(ReadError::BadRequestLine);
        }

        Ok(RequestLine {
            method: slice_from_str(&line, method),
            url: slice_from_str(&line, path),
            http_version: slice_from_str(&line, http_version),
        })
    }

    async fn read_headers(&mut self) -> Result<Subslice, ReadError<R::Error>> {
        let start_index = self.read_position;

        let mut end_index = loop {
            // First read the line
            let line = self.read_line().await?;

            // Then check that the line is not empty
            if line.as_ref().iter().all(u8::is_ascii_whitespace) {
                break line.range.start;
            }

            // Then verify that the header is valid
            // TODO - more thorough verification
            if !line.as_ref().contains(&b':') {
                return Err(ReadError::HeaderDoesNotContainColon);
            }
        };

        let headers = &mut self.buffer[start_index..end_index];

        for index in 0..headers.len() {
            if headers[index] == 0 {
                if headers[index..].iter().all(|&b| b == 0) {
                    break;
                }

                headers[index..].rotate_left(1);

                end_index -= 1;
            }
        }

        Ok(Subslice {
            buffer: self.buffer,
            range: start_index..end_index,
        })
    }

    pub async fn read(&mut self) -> Result<Request<'_, R>, ReadError<R::Error>> {
        self.wind_buffer_to_start();

        let request_line = self.read_request_line().await?;

        let request_line = request_line.range();

        let headers = self.read_headers().await?;

        let content_length = Headers(headers.as_ref())
            .get("content-length")
            .and_then(|value| value.as_str().ok()?.parse::<usize>().ok())
            .unwrap_or(0);

        let headers = headers.range;

        let parts_length = self.read_position;

        let (parts_buffer, body_buffer) = self.buffer.split_at_mut(parts_length);

        let RequestLine {
            method,
            url,
            http_version,
        } = request_line
            .index_buffer(parts_buffer)
            .as_str()
            .map_err(|_| ReadError::BadRequestLine)?;

        let (url, fragments) = url.split_once('#').map_or((url, None), |(url, fragments)| {
            (url, Some(UrlEncodedString(fragments)))
        });

        let (path, query) = url
            .split_once('?')
            .map_or((Path(UrlEncodedString(url)), None), |(path, query)| {
                (Path(UrlEncodedString(path)), Some(UrlEncodedString(query)))
            });

        let headers = Headers(&parts_buffer[headers]);

        let request = Request {
            parts: RequestParts {
                method,
                path,
                query,
                fragments,
                http_version,
                headers,
            },
            body_connection: RequestBodyConnection {
                content_length,
                reader: &mut self.reader,
                read_position: 0,
                buffer: body_buffer,
                buffer_usage: self.buffer_usage - parts_length,
                has_been_upgraded: &mut self.has_been_upgraded,
            },
        };

        // This will be true once the RequestBodyConnection has been finalized, which happens no matter how the request is handled
        self.read_position += content_length;
        self.buffer_usage = self.buffer_usage.max(self.read_position);

        Ok(request)
    }
}
