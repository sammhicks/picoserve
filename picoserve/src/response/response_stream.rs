use crate::io::{Read, Write};

fn display_contains(token: &[u8], value: impl core::fmt::Display) -> bool {
    use core::fmt::Write;

    struct Contains<'a> {
        target_token: &'a [u8],
        token_read_position: usize,
        token_is_too_long: bool,
        found: bool,
    }

    impl<'a> Contains<'a> {
        fn new(target_token: &'a [u8]) -> Self {
            Self {
                target_token,
                token_read_position: 0,
                token_is_too_long: false,
                found: false,
            }
        }

        fn reset_token(&mut self) {
            self.token_read_position = 0;
            self.token_is_too_long = false;
        }

        fn push_byte(&mut self, b: u8) {
            if self.found {
                // Already found, no point in searching further
                return;
            }

            if self.token_is_too_long {
                // Already failed, no point in searching further
                return;
            }

            if let Some(&target_byte) = self.target_token.get(self.token_read_position) {
                if b.to_ascii_lowercase() == target_byte {
                    self.token_read_position += 1;
                } else {
                    // Mismatch => token can't match anymore
                    self.token_is_too_long = true;
                }
            } else {
                // Already past last character => too long
                self.token_is_too_long = true;
            }
        }

        fn finish_token(&mut self) {
            if !self.token_is_too_long && self.token_read_position == self.target_token.len() {
                self.found = true;
            }

            self.reset_token();
        }

        fn finalize(mut self) -> bool {
            // Finish last token if any data pending
            self.finish_token();
            self.found
        }
    }

    impl Write for Contains<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            // If already found, no need to keep scanning; just accept more data.
            if self.found {
                return Ok(());
            }

            for b in s.bytes() {
                if b == b',' || b.is_ascii_whitespace() {
                    self.finish_token();
                } else {
                    self.push_byte(b);
                }
            }

            Ok(())
        }
    }

    // Edge case: empty needle always matches
    if token.is_empty() {
        return true;
    }

    let mut contains = Contains::new(token);

    let Ok(()) = write!(contains, "{value}") else {
        return false;
    };

    contains.finalize()
}

pub(crate) struct ResponseSentCore(());

/// A marker showing that the response has been sent.
pub struct ResponseSent(pub(crate) ResponseSentCore);

pub(crate) struct ResponseStream<W: Write> {
    writer: W,
    connection_header: super::KeepAlive,
}

impl<W: Write> ResponseStream<W> {
    pub(crate) fn new(writer: W, connection_header: super::KeepAlive) -> Self {
        Self {
            writer,
            connection_header,
        }
    }
}

impl<W: Write> super::ResponseWriter for ResponseStream<W> {
    type Error = W::Error;

    async fn write_response<R: Read<Error = Self::Error>, H: super::HeadersIter, B: super::Body>(
        mut self,
        connection: super::Connection<'_, R>,
        super::Response {
            status_code,
            headers,
            body,
        }: super::Response<H, B>,
    ) -> Result<ResponseSent, Self::Error> {
        #[derive(Debug)]
        enum ConnectionHeader {
            DefaultTo(super::KeepAlive),
            ForceClose,
        }

        struct HeadersWriter<WW: Write> {
            writer: WW,
            connection_header: Option<ConnectionHeader>,
        }

        impl<WW: Write> HeadersWriter<WW> {
            async fn write_header(
                &mut self,
                name: &str,
                value: impl core::fmt::Display,
            ) -> Result<(), WW::Error> {
                write!(self.writer, "{name}: {value}\r\n").await
            }
        }

        impl<WW: Write> super::ForEachHeader for HeadersWriter<WW> {
            type Output = ();
            type Error = WW::Error;

            async fn call<Value: core::fmt::Display>(
                &mut self,
                name: &str,
                value: Value,
            ) -> Result<(), Self::Error> {
                if name.eq_ignore_ascii_case("connection") {
                    if matches!(self.connection_header, Some(ConnectionHeader::ForceClose))
                        && !display_contains(b"upgrade", &value)
                    {
                        return Ok(());
                    }

                    self.connection_header = None;
                }

                self.write_header(name, value).await
            }

            async fn finalize(mut self) -> Result<(), Self::Error> {
                if let Some(connection_header) =
                    self.connection_header
                        .as_ref()
                        .map(|connection_header| match connection_header {
                            &ConnectionHeader::DefaultTo(connection_header) => connection_header,
                            ConnectionHeader::ForceClose => super::KeepAlive::Close,
                        })
                {
                    self.write_header("connection", connection_header).await?;
                }

                Ok(())
            }
        }

        use crate::io::WriteExt;
        write!(self.writer, "HTTP/1.1 {status_code} \r\n").await?;

        headers
            .for_each_header(HeadersWriter {
                writer: &mut self.writer,
                connection_header: Some(
                    if connection
                        .must_close_connection_notification
                        .has_been_triggered()
                    {
                        ConnectionHeader::ForceClose
                    } else {
                        ConnectionHeader::DefaultTo(self.connection_header)
                    },
                ),
            })
            .await?;

        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await?;

        body.write_response_body(connection, &mut self.writer)
            .await
            .map(|()| super::ResponseSent(ResponseSentCore(())))
    }
}

#[cfg(test)]
mod tests {
    use crate::response::response_stream::display_contains;

    struct SplitDisplay<const N: usize> {
        sections: [&'static str; N],
    }

    impl<const N: usize> core::fmt::Display for SplitDisplay<N> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            self.sections
                .iter()
                .try_for_each(|section| f.write_str(section))
        }
    }

    #[test]
    fn display_contains_works() {
        #[allow(dead_code)]
        #[derive(Debug)]
        enum Failure {
            Sometimes {
                target_token: &'static str,
                sections: [&'static str; 3],
                expected_outcome: bool,
            },
            Always {
                target_token: &'static str,
                search: &'static str,
                expected_outcome: bool,
            },
        }

        let mut failures = std::vec::Vec::new();

        let target_token = "upgrade";

        for (search, expected_outcome) in [
            ("upgrade", true),
            ("upgrade, upgraded", true),
            ("upgraded, upgrade", true),
            ("upgraded, upgrade, upgraded", true),
            ("upgrad", false),
            ("uuupgrade", false),
            ("upgraded", false),
            ("upgradeX", false),
        ] {
            let mut sometimes_succeeds = false;
            let mut new_failures = std::vec::Vec::new();

            for a in 0..search.len() {
                for b in 0..a {
                    let Some((before_a, after_a)) = search.split_at_checked(a) else {
                        continue;
                    };

                    let Some((between_a_and_b, after_b)) = after_a.split_at_checked(b) else {
                        continue;
                    };

                    let sections = [before_a, between_a_and_b, after_b];

                    if sections.iter().copied().any(str::is_empty) {
                        continue;
                    }

                    if expected_outcome
                        == display_contains(target_token.as_bytes(), SplitDisplay { sections })
                    {
                        sometimes_succeeds = true;
                    } else {
                        new_failures.push(Failure::Sometimes {
                            target_token,
                            sections,
                            expected_outcome,
                        })
                    }
                }
            }

            if sometimes_succeeds {
                failures.append(&mut new_failures);
            } else {
                failures.push(Failure::Always {
                    target_token,
                    search,
                    expected_outcome,
                });
            }
        }

        if !failures.is_empty() {
            panic!("Test failed: {failures:#?}");
        }
    }
}
