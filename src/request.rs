//! HTTP request types.

use core::fmt;

use embedded_io_async::Read;

use super::url_encoded::UrlEncodedString;

pub struct HeadersIter<'a>(core::str::Lines<'a>);

impl<'a> Iterator for HeadersIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let name = self.0.next()?;
        let value = self.0.next()?;
        // We ensure that name does not have any surrounding whitespace when parsing the headers.
        Some((name, value.trim()))
    }
}

#[derive(Clone, Copy)]
/// The Request Headers.
pub struct Headers<'a>(&'a str);

impl<'a> Headers<'a> {
    /// Iterator over all headers.
    pub fn iter(&self) -> HeadersIter<'a> {
        HeadersIter(self.0.lines())
    }

    /// Get a header with a name which matches (ignoring ASCII case) the given name
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.iter()
            .find_map(|(header_key, value)| name.eq_ignore_ascii_case(header_key).then_some(value))
    }
}

impl<'a> IntoIterator for Headers<'a> {
    type Item = (&'a str, &'a str);
    type IntoIter = HeadersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, 'b> IntoIterator for &'b Headers<'a> {
    type Item = (&'a str, &'a str);
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

#[derive(Debug, Clone, Copy)]
/// The URL-encoded path of the request
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

#[derive(Clone)]
/// A path "segment", i.e. the text between two `/`s.
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
    pub fn method(&self) -> &'r str {
        self.method
    }

    /// Return the request path, without the query or fragments
    pub fn path(&self) -> Path<'r> {
        self.path
    }

    /// Return the query section of the request URL, i.e. everything after the "?"
    pub fn query(&self) -> Option<UrlEncodedString<'r>> {
        self.query
    }

    /// Return the fragments of the request URL, i.e. everything after the "#"
    pub fn fragments(&self) -> Option<UrlEncodedString<'r>> {
        self.fragments
    }

    /// Return the HTTP version as sent by the client
    pub fn http_version(&self) -> &'r str {
        self.http_version
    }

    /// Return the request headers
    pub fn headers(&self) -> Headers<'r> {
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
    pub fn content_length(&self) -> usize {
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

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// The body of the request, which may not have yet been buffered.
pub struct RequestBody<'r, R: Read> {
    content_length: usize,
    reader: &'r mut R,
    buffer: &'r mut [u8],
    read_position: &'r mut usize,
    buffer_usage: &'r mut usize,
}

impl<'r, R: Read> RequestBody<'r, R> {
    /// The total length of the body
    pub fn content_length(&self) -> usize {
        self.content_length
    }

    /// The size of the buffer used to read the body into
    pub fn buffer_length(&self) -> usize {
        self.buffer.len()
    }

    /// Does the entire body fit into the buffer?
    pub fn entire_body_fits_into_buffer(&self) -> bool {
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
    pub fn content_length(&self) -> usize {
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
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError<E: embedded_io_async::Error> {
    /// The request line is invalid
    BadRequestLine,
    /// A Header line does not contain a ':'
    HeaderDoesNotContainColon,
    /// A Header line contains an invalid byte
    InvalidByteInHeader,
    /// EndOfFile before the end of the request line or headers
    UnexpectedEof,
    /// The request headers are too large to fit in the read buffer
    BufferFull,
    /// IO Error
    IO(E),
}

impl<E: embedded_io_async::Error> From<E> for ReadError<E> {
    fn from(err: E) -> Self {
        Self::IO(err)
    }
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
        if self.buffer_usage > 0 {
            if self.read_position < self.buffer_usage {
                self.buffer
                    .copy_within(self.read_position..self.buffer_usage, 0);
            }
            self.buffer_usage -= self.read_position;
            self.read_position = 0;
        }
    }

    pub async fn request_is_pending(&mut self) -> Result<bool, R::Error> {
        if self.has_been_upgraded {
            Ok(false)
        } else {
            if self.read_position == self.buffer_usage {
                self.read_position = 0;
                self.buffer_usage = self.reader.read(&mut self.buffer).await?;
            }
            Ok(self.buffer_usage > self.read_position)
        }
    }

    pub async fn read(&mut self) -> Result<Request<'_, R>, ReadError<R::Error>> {
        self.wind_buffer_to_start();

        let buf_start = self.buffer.as_ptr();
        let helper = ReadHelper {
            reader: &mut self.reader,
            buffer: &mut self.buffer,
            buffer_usage: self.buffer_usage,
        };

        let (request_line, helper) = helper.read_request_line().await?;
        let (headers, body) = helper.read_headers().await?;
        // safety: the body buffer is guaranteed to point into our read buffer.
        self.read_position = unsafe { body.buffer.as_ptr().offset_from(buf_start) as usize };
        self.buffer_usage = self.read_position + body.buffer_usage;

        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);

        let (url, fragments) = request_line
            .path
            .split_once('#')
            .map_or((request_line.path, None), |(url, fragments)| {
                (url, Some(UrlEncodedString(fragments)))
            });

        let (path, query) = url
            .split_once('?')
            .map_or((Path(UrlEncodedString(url)), None), |(path, query)| {
                (Path(UrlEncodedString(path)), Some(UrlEncodedString(query)))
            });

        let request = Request {
            parts: RequestParts {
                method: request_line.method,
                path,
                query,
                fragments,
                http_version: request_line.http_version,
                headers,
            },
            body_connection: RequestBodyConnection {
                content_length,
                reader: body.reader,
                read_position: 0,
                buffer: body.buffer,
                buffer_usage: body.buffer_usage,
                has_been_upgraded: &mut self.has_been_upgraded,
            },
        };

        // This will be true once the RequestBodyConnection has been finalized, which happens no matter how the request is handled
        self.read_position += content_length;
        self.buffer_usage = self.buffer_usage.max(self.read_position);

        Ok(request)
    }
}

/// A helper class for reading data into and consuming data out of the Reader buffer.
///
/// This provides APIs to consume and get a reference to data at the start of the buffer,
/// and return a new ReadHelper object for continuing to read into and process the tail end of the
/// buffer.
pub(crate) struct ReadHelper<'b, R: Read> {
    reader: &'b mut R,
    buffer: &'b mut [u8],
    buffer_usage: usize,
}

impl<'b, R: Read> ReadHelper<'b, R> {
    /// Read more data into self.buffer, and advance self.buffer_usage
    async fn read_more(&mut self) -> Result<usize, ReadError<R::Error>> {
        if self.buffer_usage == self.buffer.len() {
            return Err(ReadError::BufferFull);
        }
        let read_size = self
            .reader
            .read(&mut self.buffer[self.buffer_usage..])
            .await?;
        self.buffer_usage += read_size;
        Ok(read_size)
    }

    /// Read the request line.
    ///
    /// Returns the parsed request line, and a new ReadHelper that contains the remainder of the
    /// buffer.
    pub(crate) async fn read_request_line(
        self,
    ) -> Result<(RequestLine<'b>, ReadHelper<'b, R>), ReadError<R::Error>> {
        let mut remainder = self;
        let line_end = loop {
            let line_end = remainder.peek_until(b"\r\n", 0).await?;
            if line_end == 0 {
                // According to RFC 9112 section 2.2, servers SHOULD ignore
                // at least one empty line received prior to the request line.
                remainder = remainder.advance(2);
            } else {
                break line_end;
            }
        };
        let (request_line, remainder) = remainder.split_at(line_end);
        Ok((RequestLine::parse(request_line)?, remainder.advance(2)))
    }

    /// Read request headers.
    ///
    /// Reads the headers up to the CRLFCRLF token.  Returns the headers and a ReadHelper
    /// containing the remainder of the data.
    pub(crate) async fn read_headers(
        mut self,
    ) -> Result<(Headers<'b>, ReadHelper<'b, R>), ReadError<R::Error>> {
        let mut line_start = 0;
        loop {
            let line_end = self.peek_until(b"\r\n", line_start).await?;
            if line_end == line_start {
                // End of the headers.
                let (headers_buffer, remainder) = self.split_at(line_end);
                let headers = Headers(
                    // safety: we have verified that all header bytes are ASCII
                    unsafe { core::str::from_utf8_unchecked(headers_buffer) },
                );
                return Ok((headers, remainder.advance(2)));
            } else {
                let header_line = &mut self.buffer[line_start..line_end];
                parse_header_line(header_line)?;
                line_start = line_end + 2;
            }
        }
    }

    /// Read until the specified pattern is seen, and return the index to the pattern.
    async fn peek_until(
        &mut self,
        pattern: &[u8],
        offset: usize,
    ) -> Result<usize, ReadError<R::Error>> {
        let mut index = offset;
        loop {
            if let Some(relative_pos) = self.buffer[index..self.buffer_usage]
                .iter()
                .position(|&b| b == pattern[0])
            {
                let pos = relative_pos + index;
                let remaining_len = self.buffer_usage - pos;
                if remaining_len >= pattern.len() {
                    if &self.buffer[pos..(pos + pattern.len())] == pattern {
                        return Ok(pos);
                    } else {
                        // Not a match for the full pattern.
                        // Advance past this location and continue searching from here.
                        index = pos + 1;
                        continue;
                    }
                }
            }
            // Didn't find the pattern in the data we have.  Read more data.
            if self.read_more().await? == 0 {
                return Err(ReadError::UnexpectedEof);
            }
        }
    }

    /// Split the buffer, returning the initial portion of the buffer and a new ReadHelper with the
    /// remainder of the buffer.
    fn split_at(self, offset: usize) -> (&'b mut [u8], ReadHelper<'b, R>) {
        let (first, rest) = self.buffer.split_at_mut(offset);
        (
            first,
            ReadHelper {
                reader: self.reader,
                buffer: rest,
                buffer_usage: self.buffer_usage - offset,
            },
        )
    }

    fn advance(self, num_bytes: usize) -> ReadHelper<'b, R> {
        ReadHelper {
            reader: self.reader,
            buffer: &mut self.buffer[num_bytes..],
            buffer_usage: self.buffer_usage - num_bytes,
        }
    }
}

pub(crate) struct RequestLine<'a> {
    method: &'a str,
    path: &'a str,
    http_version: &'a str,
}

impl<'a> RequestLine<'a> {
    fn parse<E: embedded_io_async::Error>(line: &'a [u8]) -> Result<Self, ReadError<E>> {
        let line = core::str::from_utf8(line).map_err(|_| ReadError::BadRequestLine)?;
        let mut words = line.split(|c: char| c == ' ');
        let method = words.next().ok_or(ReadError::BadRequestLine)?;
        let path = words.next().ok_or(ReadError::BadRequestLine)?;
        let http_version = words.next().ok_or(ReadError::BadRequestLine)?;
        if words.next().is_some() {
            return Err(ReadError::BadRequestLine);
        }
        if http_version.len() < 5 || &http_version[0..5] != "HTTP/" {
            return Err(ReadError::BadRequestLine);
        }
        Ok(Self {
            method,
            path,
            http_version,
        })
    }
}

/// Parse a single header line.
///
/// This checks that the header is valid, and replaces the colon separating the header name from
/// the value with a newline, so that HeadersIter can process it using core::str::Lines.
fn parse_header_line<E: embedded_io_async::Error>(buffer: &mut [u8]) -> Result<(), ReadError<E>> {
    let mut index = 0;

    if buffer.len() == 0 {
        // This shouldn't really happen.  An empty line indicates the end of the headers,
        // and our caller should have already checked for this.
        return Err(ReadError::HeaderDoesNotContainColon);
    }

    // Process the header name
    //
    // Note: a leading space or tab at the start of the line indicates a header continuation line.
    // Continuation lines are now obsolete, and RFC 9112 allows servers to reject them.
    // We currently just reject this as part of the is_tchar() check.
    // That said, it would be slightly nicer to return a custom error code indicating that the
    // error is due to obsolete line folding.  RFC 9112 indicates that we should ideally indicate
    // this in the error message we return to the client.
    loop {
        if !is_tchar(buffer[index]) {
            return Err(ReadError::InvalidByteInHeader);
        }
        index += 1;
        if index >= buffer.len() {
            return Err(ReadError::HeaderDoesNotContainColon);
        }
        if buffer[index] == b':' {
            buffer[index] = b'\n';
            index += 1;
            break;
        }
    }

    // Process the header value
    while index < buffer.len() {
        if !is_field_content_char(buffer[index]) {
            return Err(ReadError::InvalidByteInHeader);
        }
        index += 1;
    }

    Ok(())
}

/// Checks if a character is valid token character
fn is_tchar(b: u8) -> bool {
    // From RFC 9110:
    // field-name = token
    // token = 1*tchar
    // tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" / "-" / "." /
    //     "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
    if (b >= b'A' && b <= b'Z') || (b >= b'a' && b <= b'z') || (b >= b'0' && b <= b'9') {
        true
    } else {
        match b {
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_'
            | b'`' | b'|' | b'~' => true,
            _ => false,
        }
    }
}

fn is_field_content_char(b: u8) -> bool {
    // From RFC 7230:
    // - field-value    = *( field-content / obs-fold )
    // - field-content  = field-vchar [ 1*( SP / HTAB ) field-vchar ]
    // - field-vchar    = VCHAR / obs-text
    // - obs-text       = %x80-FF ; non-ASCII characters
    // - VCHAR: any visible US ASCII character

    if b >= b' ' && b < 127 {
        // Visible ASCII characters, plus space
        true
    } else if b == b'\t' {
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, PartialEq, Eq)]
    pub struct TestIoError;

    impl embedded_io_async::Error for TestIoError {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            embedded_io_async::ErrorKind::Other
        }
    }

    struct TestDataInner {
        buf: Vec<u8>,
        offset: usize,
        /// max_read_at_once controls how many bytes read() will return at a time.
        /// This allows testing behavior when headers and other data spans multiple read() calls.
        max_read_at_once: usize,
    }

    struct TestData {
        inner: RefCell<TestDataInner>,
    }

    impl TestData {
        fn new(buf: &[u8]) -> Self {
            Self {
                inner: RefCell::new(TestDataInner {
                    buf: buf.to_vec(),
                    offset: 0,
                    max_read_at_once: usize::MAX,
                }),
            }
        }

        fn from_str(s: &str) -> Self {
            Self::new(s.as_bytes())
        }

        fn set_max_read_at_once(&self, max_read_at_once: usize) {
            self.inner.borrow_mut().max_read_at_once = max_read_at_once
        }

        fn append(&self, data: &str) {
            self.inner
                .borrow_mut()
                .buf
                .extend_from_slice(data.as_bytes());
        }
    }

    struct TestDataReader<'a>(&'a RefCell<TestDataInner>);

    impl<'a> embedded_io_async::ErrorType for TestDataReader<'a> {
        type Error = TestIoError;
    }

    impl<'a> embedded_io_async::Read for TestDataReader<'a> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let mut inner = self.0.borrow_mut();
            let bytes_left = inner.buf.len() - inner.offset;
            let read_len = buf.len().min(bytes_left).min(inner.max_read_at_once);
            buf[0..read_len].copy_from_slice(&inner.buf[inner.offset..(inner.offset + read_len)]);
            inner.offset += read_len;
            Ok(read_len)
        }
    }

    async fn test_basic(max_read_size: usize) -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(concat!(
            "GET /some_path HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Accept-Encoding: gzip, deflate, br\r\n",
            "\r\n"
        ));
        input.set_max_read_at_once(max_read_size);
        let mut buffer = [0; 1024];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let request = reader.read().await?;
        assert_eq!(request.parts.method(), "GET");
        assert_eq!(request.parts.path(), "/some_path");
        assert_eq!(request.parts.headers.get("Host"), Some("example.com"));
        assert_eq!(
            request.parts.headers.get("Accept-Encoding"),
            Some("gzip, deflate, br")
        );
        assert_eq!(request.parts.headers.get("Content-Length"), None);
        Ok(())
    }

    #[tokio::test]
    async fn basic() -> Result<(), ReadError<TestIoError>> {
        test_basic(1024).await
    }

    #[tokio::test]
    async fn byte_at_a_time() -> Result<(), ReadError<TestIoError>> {
        // Same as the basic() test, but each read() call only returns a single byte.
        test_basic(1).await
    }

    #[tokio::test]
    async fn buffer_overflow() -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(concat!(
            "GET /some_path HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Accept-Encoding: gzip, deflate, br\r\n",
            "\r\n"
        ));
        let mut buffer = [0; 64];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let result = reader.read().await;
        assert_eq!(result.map(|_| ()).unwrap_err(), ReadError::BufferFull);
        Ok(())
    }

    async fn check_bad_request(
        data: &str,
        expected_err: ReadError<TestIoError>,
    ) -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(data);
        let mut buffer = [0; 1024];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let result = reader.read().await;
        assert_eq!(result.map(|_| ()).unwrap_err(), expected_err);
        Ok(())
    }

    async fn check_bad_request_line(data: &str) -> Result<(), ReadError<TestIoError>> {
        check_bad_request(data, ReadError::BadRequestLine).await
    }

    #[tokio::test]
    async fn bad_request_line() -> Result<(), ReadError<TestIoError>> {
        check_bad_request_line("GET /some_path SNTP/1.1\r\nHost: example.com\r\n\r\n").await?;
        check_bad_request_line("GET /some_path\r\nHost: example.com\r\n\r\n").await?;
        check_bad_request_line("GET /some_path HTTP/1.1 foobar\r\nHost: example.com\r\n\r\n")
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn bad_headers() -> Result<(), ReadError<TestIoError>> {
        check_bad_request(
            "GET /some_path HTTP/1.1\r\nHost : example.com\r\n\r\n",
            ReadError::InvalidByteInHeader,
        )
        .await?;
        check_bad_request(
            "GET /some_path HTTP/1.1\r\nHost: exa\x02mple.com\r\n\r\n",
            ReadError::InvalidByteInHeader,
        )
        .await?;
        check_bad_request(
            "GET /some_path HTTP/1.1\r\nHost\r\n\r\n",
            ReadError::HeaderDoesNotContainColon,
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn unexpected_eof() -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(concat!(
            "POST /upload HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Content-L",
        ));
        let mut buffer = [0; 1024];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let result = reader.read().await;
        assert_eq!(result.map(|_| ()).unwrap_err(), ReadError::UnexpectedEof);
        Ok(())
    }

    #[tokio::test]
    async fn pipelined_requests() -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(concat!(
            "POST /upload HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Content-Length: 8\r\n",
            "\r\n",
            "12345678",
            "GET /some_path HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Accept-Encoding: gzip, deflate, br\r\n",
            "\r\n"
        ));
        let mut buffer = [0; 1024];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let request = reader.read().await?;
        assert_eq!(request.parts.method(), "POST");
        assert_eq!(request.parts.path(), "/upload");
        assert_eq!(request.parts.headers.get("Host"), Some("example.com"));
        request.body_connection.finalize().await?;

        let request = reader.read().await?;
        assert_eq!(request.parts.method(), "GET");
        assert_eq!(request.parts.path(), "/some_path");
        assert_eq!(request.parts.headers.get("Host"), Some("example.com"));
        assert_eq!(
            request.parts.headers.get("Accept-Encoding"),
            Some("gzip, deflate, br")
        );
        Ok(())
    }

    #[tokio::test]
    async fn partial_pipeline() -> Result<(), ReadError<TestIoError>> {
        let input = TestData::from_str(concat!(
            "POST /upload HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Content-Length: 8\r\n",
            "\r\n",
            "12345678",
            "GET /some_pa",
        ));
        let mut buffer = [0; 1024];
        let mut reader = Reader::new(TestDataReader(&input.inner), &mut buffer);
        let request = reader.read().await?;
        assert_eq!(request.parts.method(), "POST");
        assert_eq!(request.parts.path(), "/upload");
        assert_eq!(request.parts.headers.get("Host"), Some("example.com"));
        request.body_connection.finalize().await?;

        input.append(concat!(
            "th HTTP/1.1\r\n",
            "Host: example.com\r\n",
            "Accept-Encoding: gzip, deflate, br\r\n",
            "\r\n"
        ));
        let request = reader.read().await?;
        assert_eq!(request.parts.method(), "GET");
        assert_eq!(request.parts.path(), "/some_path");
        assert_eq!(request.parts.headers.get("Host"), Some("example.com"));
        assert_eq!(
            request.parts.headers.get("Accept-Encoding"),
            Some("gzip, deflate, br")
        );
        Ok(())
    }
}
