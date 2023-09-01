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

#[derive(Debug)]
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

#[derive(Debug)]
struct BadHeaderLine<'a>(&'a [u8]);

#[derive(Clone, Copy)]
pub struct Headers<'a>(&'a [u8]);

pub type HeadersIter<'a> = impl Iterator<Item = (&'a str, &'a str)> + 'a;

impl<'a> Headers<'a> {
    fn try_iter(&self) -> impl Iterator<Item = Result<(&'a str, &'a str), BadHeaderLine<'a>>> {
        fn split_line(line: &[u8]) -> Option<(&str, &str)> {
            let (name, value) = core::str::from_utf8(line).ok()?.split_once(':')?;
            Some((name.trim(), value.trim()))
        }

        self.0
            .split_inclusive(|&b| b == b'\n')
            .map(|line| split_line(line).ok_or(BadHeaderLine(line)))
    }

    pub fn iter(&self) -> HeadersIter<'a> {
        self.try_iter().flatten()
    }

    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.iter()
            .find_map(|(header_key, value)| key.eq_ignore_ascii_case(header_key).then_some(value))
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
        f.debug_list().entries(self.try_iter()).finish()
    }
}

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

    pub fn split_first_segment(self) -> Option<(UrlEncodedString<'r>, Path<'r>)> {
        let path = self.encoded().strip_prefix('/')?;

        let (segment, path) = path
            .char_indices()
            .find_map(|(index, c)| (c == '/').then_some(path.split_at(index)))
            .unwrap_or((path, ""));

        Some((UrlEncodedString(segment), Path(UrlEncodedString(path))))
    }

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
pub struct PathSegments<'r>(Path<'r>);

impl<'r> PathSegments<'r> {
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
pub struct Request<'r> {
    pub method: &'r str,
    pub path: Path<'r>,
    pub query: Option<UrlEncodedString<'r>>,
    pub fragments: Option<UrlEncodedString<'r>>,
    pub http_version: &'r str,
    pub headers: Headers<'r>,
    pub body: &'r [u8],
}

impl<'r> Request<'r> {
    pub fn method(&self) -> &'r str {
        self.method
    }

    pub fn path(&self) -> Path<'r> {
        self.path
    }

    pub fn query(&self) -> Option<UrlEncodedString<'r>> {
        self.query
    }

    pub fn fragments(&self) -> Option<UrlEncodedString<'r>> {
        self.fragments
    }

    pub fn http_version(&self) -> &'r str {
        self.http_version
    }

    pub fn headers(&self) -> Headers<'r> {
        self.headers
    }

    pub fn body(&self) -> &'r [u8] {
        self.body
    }
}

#[derive(Debug)]
pub enum ReadError<E> {
    BadRequestLine,
    UnexpectedEof,
    Other(E),
}

pub struct Reader<'b, R: Read> {
    reader: R,
    read_position: usize,
    buffer: &'b mut [u8],
    buffer_usage: usize,
}

impl<'b, R: Read> Reader<'b, R> {
    pub async fn new(
        mut reader: R,
        buffer: &'b mut [u8],
    ) -> Result<Option<Reader<'b, R>>, R::Error> {
        let buffer_usage = reader.read(buffer).await?;

        Ok((buffer_usage > 0).then_some(Self {
            reader,
            read_position: 0,
            buffer,
            buffer_usage,
        }))
    }

    fn used_buffer(&self) -> &[u8] {
        &self.buffer[..self.buffer_usage]
    }

    async fn read_more(&mut self, required_space: usize) -> Result<(), ReadError<R::Error>> {
        while (self.read_position + required_space) > self.buffer_usage {
            let read_size = self
                .reader
                .read(&mut self.buffer[self.buffer_usage..])
                .await
                .map_err(ReadError::Other)?;

            if read_size == 0 {
                return Err(ReadError::UnexpectedEof);
            }

            self.buffer_usage += read_size;
        }

        Ok(())
    }

    async fn next_byte(&mut self) -> Result<u8, ReadError<R::Error>> {
        self.read_more(1).await?;
        let b = self.used_buffer()[self.read_position];
        self.read_position += 1;

        Ok(b)
    }

    async fn next_slice(&mut self, len: usize) -> Result<&[u8], ReadError<R::Error>> {
        self.read_more(len).await?;
        let start = self.read_position;
        let end = self.read_position + len;
        Ok(&self.used_buffer()[start..end])
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

        loop {
            let line = self.read_line().await?;

            if line.as_ref().iter().all(u8::is_ascii_whitespace) {
                let end_index = line.range.start;
                return Ok(Subslice {
                    buffer: self.used_buffer(),
                    range: start_index..end_index,
                });
            }
        }
    }

    pub async fn read(
        &mut self,
    ) -> Result<(Request<'_>, super::response::Connection<&mut R>), ReadError<R::Error>> {
        let request_line = self.read_request_line().await?;

        let request_line = request_line.range();

        let headers = self.read_headers().await?;

        let body_length = Headers(headers.as_ref())
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);

        let headers = headers.range;

        let body_start = self.read_position;

        self.next_slice(body_length).await?;

        let body_end = body_start + body_length;

        let used_buffer = &self.buffer[..self.buffer_usage];

        let RequestLine {
            method,
            url,
            http_version,
        } = request_line
            .index_buffer(used_buffer)
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

        Ok((
            Request {
                method,
                path,
                query,
                fragments,
                http_version,
                headers: Headers(&used_buffer[headers]),
                body: &used_buffer[body_start..body_end],
            },
            super::response::Connection(&mut self.reader),
        ))
    }
}
