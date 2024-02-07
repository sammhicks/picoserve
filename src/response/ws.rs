//! Web Sockets. See [web_sockets](https://github.com/sammhicks/picoserve/blob/main/examples/web_sockets/src/main.rs) for usage example.

use crate::io::{Read, Write};

use super::{status, Connection, ResponseWriter};

/// Indicates that the websocket failed to be upgraded.
pub enum WebSocketUpgradeRejection {
    /// Websocket upgrades must use the GET method.
    MethodNotGet,
    /// Websocket upgrades must have a Connection header of "Upgrade".
    InvalidConnectionHeader,
    /// Websocket upgrades must have an Upgrade of "websocket".
    InvalidUpgradeHeader,
    /// Websocket version must be 13.
    InvalidWebSocketVersionHeader,
    /// Websocket upgrade header "sec-websocket-key" is missing.
    WebSocketKeyHeaderMissing,
}

impl super::IntoResponse for WebSocketUpgradeRejection {
    async fn write_to<R: Read, W: ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: Connection<R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, R::Error> {
        (
            status::BAD_REQUEST,
            match self {
                WebSocketUpgradeRejection::MethodNotGet => {
                    return (
                        status::METHOD_NOT_ALLOWED,
                        "Websocket upgrades must use the `GET` method\n",
                    )
                        .write_to(writer, connection, response_writer)
                        .await
                }
                WebSocketUpgradeRejection::InvalidConnectionHeader => {
                    "Websocket upgrades must have a Connection header of `Upgrade`\n"
                }
                WebSocketUpgradeRejection::InvalidUpgradeHeader => {
                    "Websocket upgrades must have an Upgrade of `websocket`\n"
                }
                WebSocketUpgradeRejection::InvalidWebSocketVersionHeader => {
                    "Websocket version must be 13\n"
                }
                WebSocketUpgradeRejection::WebSocketKeyHeaderMissing => {
                    "Websocket upgrades must have a `Sec-WebSocket-Key` header\n"
                }
            },
        )
            .write_to(writer, connection, response_writer)
            .await
    }
}

/// Types which can represent either a specified web socket protocol, or an unspecified web socket protocol.
pub trait WebSocketProtocol {
    fn name(&self) -> Option<&str>;
}

/// The Web Socket HTTP response does not have a specified protocol.
pub struct UnspecifiedProtocol;

impl WebSocketProtocol for UnspecifiedProtocol {
    fn name(&self) -> Option<&str> {
        None
    }
}

/// The Web Socket HTTP response has the following specified protocol.
pub struct SpecifiedProtocol<P: AsRef<str>>(P);

impl<P: AsRef<str>> WebSocketProtocol for SpecifiedProtocol<P> {
    fn name(&self) -> Option<&str> {
        Some(self.0.as_ref())
    }
}

/// A HTTP upgrade request.
pub struct WebSocketUpgrade {
    key: [u8; 28],
    protocols: Option<heapless::String<32>>,
}

impl WebSocketUpgrade {
    /// If protocols are specified by the client, return an iterator of them.
    /// If not, return None.
    pub fn protocols(&self) -> Option<impl Iterator<Item = &str>> {
        self.protocols
            .as_ref()
            .map(|protocols| protocols.split(',').map(str::trim))
    }
}

impl<State> crate::extract::FromRequest<State> for WebSocketUpgrade {
    type Rejection = WebSocketUpgradeRejection;

    async fn from_request<R: Read>(
        _state: &State,
        request: &crate::request::Request<'_>,
        _body_reader: R,
    ) -> Result<Self, Self::Rejection> {
        if !request.method().eq_ignore_ascii_case("get") {
            return Err(WebSocketUpgradeRejection::MethodNotGet);
        }

        if request
            .headers()
            .get("upgrade")
            .map_or(true, |upgrade| !upgrade.eq_ignore_ascii_case("websocket"))
        {
            return Err(WebSocketUpgradeRejection::InvalidUpgradeHeader);
        }

        if request.headers().get("sec-websocket-version") != Some("13") {
            return Err(WebSocketUpgradeRejection::InvalidWebSocketVersionHeader);
        }

        let key = request
            .headers()
            .get("sec-websocket-key")
            .map(|key| {
                let hash = lhash::Sha1::new()
                    .const_update(key.as_bytes())
                    .const_update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
                    .const_result();

                let mut buffer = [0; 28];

                data_encoding::BASE64.encode_mut(&hash, &mut buffer);

                buffer
            })
            .ok_or(WebSocketUpgradeRejection::WebSocketKeyHeaderMissing)?;

        let protocols = request
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|protocol| {
                let mut buffer = heapless::String::new();
                buffer.push_str(protocol).ok()?;
                Some(buffer)
            });

        Ok(Self { key, protocols })
    }
}

/// A web socket message opcode.
#[derive(Debug)]
pub enum Opcode {
    Data(Data),
    Control(Control),
}

/// A web socket message data opcode.
#[derive(Debug)]
pub enum Data {
    Continue,
    Text,
    Binary,
    Reserved(u8),
}

/// A web socket message control opcode.
#[derive(Debug)]
pub enum Control {
    Close,
    Ping,
    Pong,
    Reserved(u8),
}

impl From<u8> for Opcode {
    fn from(value: u8) -> Self {
        match value {
            0 => Opcode::Data(Data::Continue),
            1 => Opcode::Data(Data::Text),
            2 => Opcode::Data(Data::Binary),
            3..=7 => Opcode::Data(Data::Reserved(value)),
            8 => Opcode::Control(Control::Close),
            9 => Opcode::Control(Control::Ping),
            10 => Opcode::Control(Control::Pong),
            11..=255 => Opcode::Control(Control::Reserved(value)),
        }
    }
}

/// A single Web Socket frame.
#[derive(Debug)]
pub struct Frame {
    /// If true, this frame is the final frame of the message.
    pub is_final: bool,
    /// The opcode of this frame.
    pub opcode: Opcode,
    /// The length in bytes of the data of this frame.
    pub length: usize,
}

/// Errors arising when reading a frame.
#[derive(Debug)]
pub enum ReadFrameError<E> {
    /// IO Error while reading.
    Io(E),
    /// EOF received which reading the frame.
    UnexpectedEof,
    /// The message length is too large to be represented as a usize.
    MessageIsTooLong(u64),
    /// The message is larger than the given buffer.
    OutOfSpace,
}

impl<E> From<embedded_io_async::ReadExactError<E>> for ReadFrameError<E> {
    fn from(value: embedded_io_async::ReadExactError<E>) -> Self {
        match value {
            embedded_io_async::ReadExactError::UnexpectedEof => Self::UnexpectedEof,
            embedded_io_async::ReadExactError::Other(err) => Self::Io(err),
        }
    }
}

/// Errors arising when reading a message.
#[derive(Debug)]
pub enum ReadMessageError<E> {
    /// IO Error while reading.
    Io(E),
    /// IO Error while reading a frame.
    ReadFrameError(ReadFrameError<E>),
    /// The opcode is a reserved value.
    ReservedOpcode(u8),
    /// The first frame received was a continuation frame.
    MessageStartsWithContinuation,
    /// An opcode that wasn't "Continuation" was received before a final frame was received.
    UnexpectedMessageStart,
    /// The message was a text message, but the data was not UTF-8.
    TextIsNotUtf8,
}

impl<E> From<core::str::Utf8Error> for ReadMessageError<E> {
    fn from(_: core::str::Utf8Error) -> Self {
        Self::TextIsNotUtf8
    }
}

enum MessageOpcode {
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

/// Message Types.
#[derive(Debug)]
pub enum Message<'a> {
    Text(&'a str),
    Binary(&'a [u8]),
    Close(Option<(u16, &'a str)>),
    Ping(&'a [u8]),
    Pong(&'a [u8]),
}

/// A source of Web Socket Frames.
pub struct SocketRx<R: Read> {
    reader: R,
}

impl<R: Read> SocketRx<R> {
    /// Read the next frame. If the frame is not final, then before calling next_message,
    /// next_frame must be repeatedly called until a final frame is received.
    pub async fn next_frame(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<Frame, ReadFrameError<R::Error>> {
        let [first, second] = {
            let mut header = [0; 2];
            self.reader.read_exact(&mut header).await?;
            header
        };

        let is_final = first & 0x80 != 0;

        // let rsv1 = first & 0x40 != 0;
        // let rsv2 = first & 0x20 != 0;
        // let rsv3 = first & 0x10 != 0;

        let opcode = Opcode::from(first & 0x0F);

        let is_masked = second & 0x80 != 0;

        let length_byte = second & 0x7F;

        let length = match length_byte {
            126 => {
                let mut length_bytes = [0; 2];
                self.reader.read_exact(&mut length_bytes).await?;
                u16::from_be_bytes(length_bytes).into()
            }
            127 => {
                let mut length_bytes = [0; 8];
                self.reader.read_exact(&mut length_bytes).await?;
                let length = u64::from_be_bytes(length_bytes);
                length
                    .try_into()
                    .map_err(|_| ReadFrameError::MessageIsTooLong(length))?
            }
            length => length.into(),
        };

        let mut mask = [0; 4];

        if is_masked {
            self.reader.read_exact(&mut mask).await?;
        }

        let data = buffer.get_mut(..length).ok_or(ReadFrameError::OutOfSpace)?;

        self.reader.read_exact(data).await?;

        if is_masked {
            for (data, mask) in data.iter_mut().zip(mask.iter().cycle()) {
                *data ^= mask;
            }
        }

        Ok(Frame {
            is_final,
            opcode,
            length,
        })
    }

    /// Read the next message. Frame data is concatenated together.
    pub async fn next_message<'a>(
        &mut self,
        buffer: &'a mut [u8],
    ) -> Result<Message<'a>, ReadMessageError<R::Error>> {
        let Frame {
            is_final: is_single_frame,
            opcode,
            length: mut message_length,
        } = self.next_frame(buffer).await.map_err(|err| {
            if let ReadFrameError::Io(io_err) = err {
                ReadMessageError::Io(io_err)
            } else {
                ReadMessageError::ReadFrameError(err)
            }
        })?;

        let opcode = match opcode {
            Opcode::Data(Data::Continue) => {
                return Err(ReadMessageError::MessageStartsWithContinuation)
            }
            Opcode::Data(Data::Text) => MessageOpcode::Text,
            Opcode::Data(Data::Binary) => MessageOpcode::Binary,
            Opcode::Control(Control::Close) => MessageOpcode::Close,
            Opcode::Control(Control::Ping) => MessageOpcode::Ping,
            Opcode::Control(Control::Pong) => MessageOpcode::Pong,
            Opcode::Data(Data::Reserved(opcode)) | Opcode::Control(Control::Reserved(opcode)) => {
                return Err(ReadMessageError::ReservedOpcode(opcode))
            }
        };

        if !is_single_frame {
            loop {
                let Frame {
                    is_final,
                    opcode,
                    length,
                } = self
                    .next_frame(&mut buffer[message_length..])
                    .await
                    .map_err(ReadMessageError::ReadFrameError)?;

                match opcode {
                    Opcode::Data(Data::Continue) => (),
                    Opcode::Data(Data::Text)
                    | Opcode::Data(Data::Binary)
                    | Opcode::Control(Control::Close)
                    | Opcode::Control(Control::Ping)
                    | Opcode::Control(Control::Pong) => {
                        return Err(ReadMessageError::UnexpectedMessageStart)
                    }
                    Opcode::Data(Data::Reserved(opcode))
                    | Opcode::Control(Control::Reserved(opcode)) => {
                        return Err(ReadMessageError::ReservedOpcode(opcode))
                    }
                }

                message_length += length;

                if is_final {
                    break;
                }
            }
        }

        let data = &buffer[..message_length];

        Ok(match opcode {
            MessageOpcode::Text => Message::Text(core::str::from_utf8(data)?),
            MessageOpcode::Binary => Message::Binary(data),
            MessageOpcode::Close => Message::Close(match data {
                [] => None,
                &[code] => Some((code.into(), "")),
                [c1, c0, text @ ..] => {
                    Some((u16::from_be_bytes([*c1, *c0]), core::str::from_utf8(text)?))
                }
            }),
            MessageOpcode::Ping => Message::Ping(data),
            MessageOpcode::Pong => Message::Pong(data),
        })
    }
}

/// A sink of Web Socket Frames.
pub struct SocketTx<W: Write> {
    writer: W,
}

impl<W: Write> SocketTx<W> {
    async fn flush(&mut self) -> Result<(), W::Error> {
        self.writer.flush().await
    }

    async fn write_length(&mut self, length: usize) -> Result<(), W::Error> {
        if let Some(length_byte) = u8::try_from(length).ok().filter(|length| *length <= 125) {
            self.writer.write_all(&[length_byte]).await
        } else if let Ok(length) = u16::try_from(length) {
            self.writer.write_all(&[126]).await?;
            self.writer.write_all(&length.to_be_bytes()).await
        } else {
            self.writer.write_all(&[127]).await?;
            self.writer.write_all(&(length as u64).to_be_bytes()).await
        }
    }

    async fn write_frame(
        &mut self,
        is_final: bool,
        opcode: u8,
        data: &[u8],
    ) -> Result<(), W::Error> {
        self.writer
            .write_all(&[if is_final { 0b10000000 } else { 0 } | opcode])
            .await?;

        self.write_length(data.len()).await?;

        self.writer.write_all(data).await
    }

    /// Send a text message.
    pub async fn send_text(&mut self, data: &str) -> Result<(), W::Error> {
        self.write_frame(true, 1, data.as_bytes()).await?;
        self.flush().await
    }

    /// Send a binary message.
    pub async fn send_binary(&mut self, data: &[u8]) -> Result<(), W::Error> {
        self.write_frame(true, 2, data).await?;
        self.flush().await
    }

    /// Send the given value as a JSON encoded text message.
    /// If the message is long, the message will be sent as several frames, and the value will be repeatedly serialized,
    /// so it must serialize to the same value each time.
    pub async fn send_json(&mut self, value: impl serde::Serialize) -> Result<(), W::Error> {
        super::json::Json(value)
            .do_write_to(&mut JsonWriter {
                is_first: true,
                tx: self,
            })
            .await?;
        self.write_frame(true, 0, &[]).await?;
        self.flush().await
    }

    /// Close the connection with the given reason.
    pub async fn close(mut self, reason: impl Into<Option<(u16, &str)>>) -> Result<(), W::Error> {
        self.writer.write_all(&[0b10000000 | 8]).await?; // Final Close frame

        match reason.into() {
            Some((code, message)) => {
                let code_bytes = code.to_be_bytes();
                self.write_length(code_bytes.len() + message.len()).await?;
                self.writer.write_all(&code_bytes).await?;
                self.writer.write_all(message.as_bytes()).await
            }
            None => self.write_length(0).await,
        }?;

        self.flush().await
    }

    /// Send a ping message with the given data.
    pub async fn send_ping(&mut self, data: &[u8]) -> Result<(), W::Error> {
        self.write_frame(true, 9, data).await
    }

    /// Send a pong message with the given data.
    pub async fn send_pong(&mut self, data: &[u8]) -> Result<(), W::Error> {
        self.write_frame(true, 10, data).await
    }
}

struct JsonWriter<'w, W: Write> {
    is_first: bool,
    tx: &'w mut SocketTx<W>,
}

impl<'w, W: Write> embedded_io_async::ErrorType for JsonWriter<'w, W> {
    type Error = W::Error;
}

impl<'w, W: Write> Write for JsonWriter<'w, W> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, W::Error> {
        self.tx
            .write_frame(
                false,
                if self.is_first {
                    self.is_first = false;
                    1
                } else {
                    0
                },
                data,
            )
            .await
            .map(|_| data.len())
    }
}

/// Implement [WebSocketCallback] to handle and sent web socket messages.
pub trait WebSocketCallback {
    async fn run<R: Read, W: Write<Error = R::Error>>(
        self,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
    ) -> Result<(), W::Error>;
}

/// The HTTP response sent to the client, notifying it that the connection can been upgraded to a web socket connection.
pub struct UpgradedWebSocket<P: WebSocketProtocol, C: WebSocketCallback> {
    sec_websocket_accept: [u8; 28],
    sec_websocket_protocol: P,
    callback: C,
}

impl<C: WebSocketCallback> UpgradedWebSocket<UnspecifiedProtocol, C> {
    /// Specify the web socket protocol used.
    pub fn with_protocol<P: AsRef<str>>(
        self,
        protocol: P,
    ) -> UpgradedWebSocket<SpecifiedProtocol<P>, C> {
        let UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol: UnspecifiedProtocol,
            callback,
        } = self;

        UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol: SpecifiedProtocol(protocol),
            callback,
        }
    }
}

impl WebSocketUpgrade {
    /// Handle the websocket upgrade. The returned [UpgradedWebSocket] should be returned by the request handler,
    /// and thus returned to the client.
    pub fn on_upgrade<C: WebSocketCallback>(
        self,
        callback: C,
    ) -> UpgradedWebSocket<UnspecifiedProtocol, C> {
        UpgradedWebSocket {
            sec_websocket_accept: self.key,
            sec_websocket_protocol: UnspecifiedProtocol,
            callback,
        }
    }
}

struct UpgradedWebSocketBody<C: WebSocketCallback>(C);

impl<C: WebSocketCallback> super::Body for UpgradedWebSocketBody<C> {
    async fn write_response_body<
        R: embedded_io_async::Read,
        W: embedded_io_async::Write<Error = R::Error>,
    >(
        self,
        super::Connection(reader): super::Connection<R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.flush().await?;
        self.0.run(SocketRx { reader }, SocketTx { writer }).await
    }
}

impl<P: WebSocketProtocol, C: WebSocketCallback> super::IntoResponse for UpgradedWebSocket<P, C> {
    async fn write_to<R: Read, W: ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: Connection<R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, R::Error> {
        let UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol,
            callback,
        } = self;

        response_writer
            .write_response(
                writer,
                connection,
                super::Response {
                    status_code: status::SWITCHING_PROTOCOLS,
                    headers: [
                        ("Upgrade", "websocket"),
                        ("Connection", "upgrade"),
                        ("Sec-WebSocket-Accept", unsafe {
                            // SAFETY: sec_websocket_accept was created by data_encoding::BASE64.encode_mut, which creates a UTF-8 string
                            core::str::from_utf8_unchecked(&sec_websocket_accept)
                        }),
                    ],
                    body: UpgradedWebSocketBody(callback),
                }
                .with_headers(sec_websocket_protocol.name().map(
                    |sec_websocket_protocol| ("Sec-WebSocket-Protocol", sec_websocket_protocol),
                )),
            )
            .await
    }
}

impl<P: WebSocketProtocol, C: WebSocketCallback> core::future::IntoFuture
    for UpgradedWebSocket<P, C>
{
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}
