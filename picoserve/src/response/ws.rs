//! Web Sockets. See [web_sockets](https://github.com/sammhicks/picoserve/blob/main/examples/web_sockets/src/main.rs) for usage example.

use core::marker::PhantomData;

use picoserve_derive::ErrorWithStatusCode;

use crate::{
    self as picoserve,
    extract::FromRequestParts,
    futures::Either,
    io::{Read, Write, WriteExt},
};

use super::StatusCode;

/// Indicates that the websocket failed to be upgraded.
#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[status_code(BAD_REQUEST)]
pub enum WebSocketUpgradeRejection {
    /// Websocket upgrades must use the GET method.
    #[error("Websocket upgrades must use the `GET` method")]
    #[status_code(BAD_REQUEST)]
    MethodNotGet,
    /// Websocket upgrades must have a Connection header of "Upgrade".
    #[error("Websocket upgrades must have a Connection header of `Upgrade`")]
    InvalidConnectionHeader,
    /// Websocket upgrades must have an Upgrade of "websocket".
    #[error("Websocket upgrades must have an Upgrade of `websocket`")]
    InvalidUpgradeHeader,
    /// Websocket version must be 13.
    #[error("Websocket version must be 13")]
    InvalidWebSocketVersionHeader,
    /// Websocket upgrade header "sec-websocket-key" is missing.
    #[error("Websocket upgrades must have a `Sec-WebSocket-Key` header")]
    WebSocketKeyHeaderMissing,
}

/// Types which can represent either a specified web socket protocol, or an unspecified web socket protocol.
pub trait WebSocketProtocol {
    /// Return the name of the protocol, or None if unspecified.
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

type WebSocketKey = [u8; 28];

/// A HTTP upgrade request.
pub struct WebSocketUpgrade {
    key: WebSocketKey,
    protocols: Option<heapless::String<32>>,
    upgrade_token: crate::extract::UpgradeToken,
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

impl<'r, State> crate::extract::FromRequest<'r, State> for WebSocketUpgrade {
    type Rejection = WebSocketUpgradeRejection;

    async fn from_request<R: Read>(
        state: &'r State,
        request_parts: crate::request::RequestParts<'r>,
        _request_body: crate::request::RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        if !request_parts.method().eq_ignore_ascii_case("get") {
            return Err(WebSocketUpgradeRejection::MethodNotGet);
        }

        let upgrade_token = crate::extract::UpgradeToken::from_request_parts(state, &request_parts)
            .await
            .map_err(|crate::extract::NoUpgradeHeaderError| {
                WebSocketUpgradeRejection::InvalidUpgradeHeader
            })?;

        if request_parts
            .headers()
            .get("upgrade")
            .is_none_or(|upgrade| upgrade != "websocket")
        {
            return Err(WebSocketUpgradeRejection::InvalidUpgradeHeader);
        }

        if !request_parts
            .headers()
            .get("sec-websocket-version")
            .is_some_and(|version| version == "13")
        {
            return Err(WebSocketUpgradeRejection::InvalidWebSocketVersionHeader);
        }

        let key = request_parts
            .headers()
            .get("sec-websocket-key")
            .map(|key| {
                let hash = lhash::Sha1::new()
                    .const_update(key.value)
                    .const_update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
                    .const_result();

                let mut buffer = [0; 28];

                data_encoding::BASE64.encode_mut(&hash, &mut buffer);

                buffer
            })
            .ok_or(WebSocketUpgradeRejection::WebSocketKeyHeaderMissing)?;

        let protocols = request_parts
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|protocol| {
                let mut buffer = heapless::String::new();
                buffer.push_str(protocol.as_str().ok()?).ok()?;
                Some(buffer)
            });

        Ok(Self {
            key,
            protocols,
            upgrade_token,
        })
    }
}

/// A web socket message opcode.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Opcode {
    /// "Data", e.g. text or binary.
    Data(Data),
    /// "Control" information, such as Close, Ping, and Pong.
    Control(Control),
}

/// A web socket message data opcode.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Data {
    /// This frame continues from the previous frame.
    Continue,
    /// This frame starts a UTF-8 text string.
    Text,
    /// This frame starts a binary blob.
    Binary,
    /// This frame uses a reserved opcode.
    Reserved(u8),
}

/// A web socket message control opcode.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Control {
    /// The connection should be closed.
    Close,
    /// A ping message, which should be replied with a "pong" message containing the same data.
    Ping,
    /// The response to a "ping" message
    Pong,
    /// This frame uses a reserved opcode.
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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReadFrameError {
    /// EOF received which reading the frame.
    UnexpectedEof,
    /// The message length is too large to be represented as a usize.
    MessageIsTooLong(u64),
    /// The message is larger than the given buffer.
    OutOfSpace,
}

/// Errors arising when reading a message.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReadMessageError {
    /// Error while reading a frame.
    ReadFrameError(ReadFrameError),
    /// The opcode is a reserved value.
    ReservedOpcode(u8),
    /// The first frame received was a continuation frame.
    MessageStartsWithContinuation,
    /// An opcode that wasn't "Continuation" was received before a final frame was received.
    UnexpectedMessageStart,
    /// The message was a text message, but the data was not UTF-8.
    TextIsNotUtf8,
}

impl ReadMessageError {
    pub fn code(&self) -> u16 {
        match self {
            Self::ReadFrameError(_)
            | Self::MessageStartsWithContinuation
            | Self::UnexpectedMessageStart => 1002,
            Self::ReservedOpcode(_) => 1003,
            Self::TextIsNotUtf8 => 1007,
        }
    }
}

enum InternalError<IoError, Error> {
    Io(IoError),
    Other(Error),
}

impl<IoError, Error> From<Error> for InternalError<IoError, Error> {
    fn from(error: Error) -> Self {
        Self::Other(error)
    }
}

impl<IoError> From<crate::io::ReadExactError<IoError>> for InternalError<IoError, ReadFrameError> {
    fn from(value: crate::io::ReadExactError<IoError>) -> Self {
        match value {
            crate::io::ReadExactError::UnexpectedEof => Self::Other(ReadFrameError::UnexpectedEof),
            crate::io::ReadExactError::Other(error) => Self::Io(error),
        }
    }
}

impl<IOError> From<InternalError<IOError, ReadFrameError>>
    for InternalError<IOError, ReadMessageError>
{
    fn from(error: InternalError<IOError, ReadFrameError>) -> Self {
        match error {
            InternalError::Io(error) => InternalError::Io(error),
            InternalError::Other(error) => {
                InternalError::Other(ReadMessageError::ReadFrameError(error))
            }
        }
    }
}

impl<IoError> From<core::str::Utf8Error> for InternalError<IoError, ReadMessageError> {
    fn from(_: core::str::Utf8Error) -> Self {
        ReadMessageError::TextIsNotUtf8.into()
    }
}

trait InternalResultExt<T, S, IoError, Error> {
    fn into_nested_result(self) -> Result<Either<Result<T, Error>, S>, IoError>;
}

impl<T, S, IoError, Error> InternalResultExt<T, S, IoError, Error>
    for Result<Either<T, S>, InternalError<IoError, Error>>
{
    fn into_nested_result(self) -> Result<Either<Result<T, Error>, S>, IoError> {
        match self {
            Ok(Either::First(value)) => Ok(Either::First(Ok(value))),
            Ok(Either::Second(signal)) => Ok(Either::Second(signal)),
            Err(InternalError::Io(error)) => Err(error),
            Err(InternalError::Other(error)) => Ok(Either::First(Err(error))),
        }
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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Message<'a> {
    /// A UTF-8 encoded string.
    Text(&'a str),
    /// A blob of (possibly structured) binary data.
    Binary(&'a [u8]),
    /// A request to close the connection.
    Close(Option<(u16, &'a str)>),
    /// A ping message, which should be replied with a "pong" message containing the same data.
    Ping(&'a [u8]),
    /// The response to a "ping" message
    Pong(&'a [u8]),
}

async fn next_byte<R: Read>(reader: &mut R) -> Result<u8, crate::io::ReadExactError<R::Error>> {
    let mut buffer = 0;

    reader
        .read_exact(core::slice::from_mut(&mut buffer))
        .await
        .map(|()| buffer)
}

/// A source of Web Socket Frames.
pub struct SocketRx<R: Read> {
    reader: R,
}

impl<R: Read> SocketRx<R> {
    async fn next_frame_internal<Signal: core::future::Future>(
        &mut self,
        buffer: &mut [u8],
        other_signal: Signal,
    ) -> Result<Either<Frame, Signal::Output>, InternalError<R::Error, ReadFrameError>> {
        let first = match crate::futures::select_either(
            core::pin::pin!(other_signal),
            next_byte(&mut self.reader),
        )
        .await
        {
            Either::First(signal) => return Ok(Either::Second(signal)),
            Either::Second(b) => b?,
        };

        let second = next_byte(&mut self.reader).await?;

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

        Ok(Either::First(Frame {
            is_final,
            opcode,
            length,
        }))
    }

    /// Read the next frame unless `signal` resolves before receiving the start of the frame. `signal` **must** be cancel-safe.
    /// If the frame is not final, then before calling [`next_message`](Self::next_message),
    /// `next_frame` must be repeatedly called until a final frame is received.
    ///
    /// `next_frame` is *not* cancel-safe.
    pub async fn next_frame<Signal: core::future::Future>(
        &mut self,
        buffer: &mut [u8],
        signal: Signal,
    ) -> Result<Either<Result<Frame, ReadFrameError>, Signal::Output>, R::Error> {
        self.next_frame_internal(buffer, signal)
            .await
            .into_nested_result()
    }

    async fn next_message_internal<'a, Signal: core::future::Future>(
        &mut self,
        buffer: &'a mut [u8],
        signal: Signal,
    ) -> Result<Either<Message<'a>, Signal::Output>, InternalError<R::Error, ReadMessageError>>
    {
        let Frame {
            is_final: is_single_frame,
            opcode,
            length: mut message_length,
        } = match self.next_frame_internal(buffer, signal).await? {
            Either::First(frame) => frame,
            Either::Second(signal) => return Ok(Either::Second(signal)),
        };

        let opcode = match opcode {
            Opcode::Data(Data::Continue) => {
                return Err(ReadMessageError::MessageStartsWithContinuation.into())
            }
            Opcode::Data(Data::Text) => MessageOpcode::Text,
            Opcode::Data(Data::Binary) => MessageOpcode::Binary,
            Opcode::Control(Control::Close) => MessageOpcode::Close,
            Opcode::Control(Control::Ping) => MessageOpcode::Ping,
            Opcode::Control(Control::Pong) => MessageOpcode::Pong,
            Opcode::Data(Data::Reserved(opcode)) | Opcode::Control(Control::Reserved(opcode)) => {
                return Err(ReadMessageError::ReservedOpcode(opcode).into())
            }
        };

        if !is_single_frame {
            loop {
                let Frame {
                    is_final,
                    opcode,
                    length,
                } = self
                    .next_frame_internal(&mut buffer[message_length..], core::future::pending())
                    .await?
                    .ignore_never_b();

                match opcode {
                    Opcode::Data(Data::Continue) => (),
                    Opcode::Data(Data::Text)
                    | Opcode::Data(Data::Binary)
                    | Opcode::Control(Control::Close)
                    | Opcode::Control(Control::Ping)
                    | Opcode::Control(Control::Pong) => {
                        return Err(ReadMessageError::UnexpectedMessageStart.into())
                    }
                    Opcode::Data(Data::Reserved(opcode))
                    | Opcode::Control(Control::Reserved(opcode)) => {
                        return Err(ReadMessageError::ReservedOpcode(opcode).into())
                    }
                }

                message_length += length;

                if is_final {
                    break;
                }
            }
        }

        let data = &buffer[..message_length];

        Ok(Either::First(match opcode {
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
        }))
    }

    /// Read the next message unless `signal` resolves before receiving the start of the message. `signal` **must** be cancel-safe. Frame data is concatenated together.
    pub async fn next_message<'a, Signal: core::future::Future>(
        &mut self,
        buffer: &'a mut [u8],
        signal: Signal,
    ) -> Result<Either<Result<Message<'a>, ReadMessageError>, Signal::Output>, R::Error> {
        self.next_message_internal(buffer, signal)
            .await
            .into_nested_result()
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

    /// Send the given value as UTF-8 text using its [`Display`](core::fmt::Display) implementation.
    /// If the message is long, the message will be sent as several frames, [`Display::fmt`](core::fmt::Display::fmt) will be repeatedly called
    /// so must produce the same output each time.
    pub async fn send_display(&mut self, data: impl core::fmt::Display) -> Result<(), W::Error> {
        let opcode = &mut 1;
        write!(FrameWriter { opcode, tx: self }, "{data}").await?;
        self.write_frame(true, *opcode, &[]).await?;
        self.flush().await
    }

    /// Send the given value as a JSON encoded text message.
    /// If the message is long, the message will be sent as several frames, and the value will be repeatedly serialized,
    /// so it must serialize to the same value each time.
    #[cfg(feature = "json")]
    pub async fn send_json(&mut self, value: impl serde::Serialize) -> Result<(), W::Error> {
        let opcode = &mut 1;
        super::json::Json(value)
            .do_write_to(&mut FrameWriter { opcode, tx: self })
            .await?;
        self.write_frame(true, *opcode, &[]).await?;
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
        self.write_frame(true, 9, data).await?;
        self.flush().await
    }

    /// Send a pong message with the given data.
    pub async fn send_pong(&mut self, data: &[u8]) -> Result<(), W::Error> {
        self.write_frame(true, 10, data).await?;
        self.flush().await
    }
}

struct FrameWriter<'w, W: Write> {
    opcode: &'w mut u8,
    tx: &'w mut SocketTx<W>,
}

impl<W: Write> crate::io::ErrorType for FrameWriter<'_, W> {
    type Error = W::Error;
}

impl<W: Write> Write for FrameWriter<'_, W> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, W::Error> {
        self.tx
            .write_frame(false, core::mem::replace(self.opcode, 0), data)
            .await
            .map(|_| data.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.tx.flush().await
    }
}

/// Implement [`WebSocketCallback`] to handle and send web socket messages.
pub trait WebSocketCallback {
    /// Run the WebSocket connection, reading and writing to the socket.
    async fn run<R: Read, W: Write<Error = R::Error>>(
        self,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
    ) -> Result<(), W::Error>;
}

impl<C: WebSocketCallback> WebSocketCallbackWithShutdownSignal for C {
    async fn run_with_shutdown_signal<
        R: Read,
        W: Write<Error = R::Error>,
        S: core::future::Future<Output = ()> + Clone + Unpin,
    >(
        self,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
        _shutdown_signal: S,
    ) -> Result<(), W::Error> {
        self.run(rx, tx).await
    }
}

/// A [`WebSocketCallback`] which is signalled when the server shuts down gracefully.
pub trait WebSocketCallbackWithShutdownSignal {
    /// Run the WebSocket connection, reading and writing to the socket.
    /// If the server has graceful shutdown configured, `shutdown_signal` resolves when the server shuts down.
    async fn run_with_shutdown_signal<
        R: Read,
        W: Write<Error = R::Error>,
        S: core::future::Future<Output = ()> + Clone + Unpin,
    >(
        self,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
        shutdown_signal: S,
    ) -> Result<(), W::Error>;
}

/// A [`WebSocketCallback`] with access to the server state.
pub trait WebSocketCallbackWithState<State> {
    /// Run the WebSocket connection, reading and writing to the socket.
    async fn run_with_state<R: Read, W: Write<Error = R::Error>>(
        self,
        state: &State,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
    ) -> Result<(), W::Error>;
}

impl<State, C: WebSocketCallback> WebSocketCallbackWithState<State> for C {
    async fn run_with_state<R: Read, W: Write<Error = R::Error>>(
        self,
        _state: &State,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
    ) -> Result<(), W::Error> {
        self.run(rx, tx).await
    }
}

/// A [`WebSocketCallback`] with access to the server state, and which is signalled when the server shuts down gracefully..
pub trait WebSocketCallbackWithStateAndShutdownSignal<State> {
    /// Run the WebSocket connection, reading and writing to the socket.
    async fn run_with_state_and_shutdown_signal<
        R: Read,
        W: Write<Error = R::Error>,
        S: core::future::Future<Output = ()> + Clone + Unpin,
    >(
        self,
        state: &State,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
        shutdown_signal: S,
    ) -> Result<(), W::Error>;
}

impl<State, C: WebSocketCallbackWithState<State>> WebSocketCallbackWithStateAndShutdownSignal<State>
    for C
{
    async fn run_with_state_and_shutdown_signal<
        R: Read,
        W: Write<Error = R::Error>,
        S: core::future::Future<Output = ()> + Clone + Unpin,
    >(
        self,
        state: &State,
        rx: SocketRx<R>,
        tx: SocketTx<W>,
        _shutdown_signal: S,
    ) -> Result<(), W::Error> {
        self.run_with_state(state, rx, tx).await
    }
}

/// The HTTP response sent to the client, notifying it that the connection can been upgraded to a web socket connection.
pub struct UpgradedWebSocket<P: WebSocketProtocol, C> {
    sec_websocket_accept: WebSocketKey,
    sec_websocket_protocol: P,
    upgrade_token: crate::extract::UpgradeToken,
    callback: C,
}

impl<C> UpgradedWebSocket<UnspecifiedProtocol, C> {
    /// Specify the web socket protocol used.
    pub fn with_protocol<P: AsRef<str>>(
        self,
        protocol: P,
    ) -> UpgradedWebSocket<SpecifiedProtocol<P>, C> {
        let UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol: UnspecifiedProtocol,
            upgrade_token,
            callback,
        } = self;

        UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol: SpecifiedProtocol(protocol),
            upgrade_token,
            callback,
        }
    }
}

/// Indicates that the callback doesn't use the server state
pub struct CallbackNotUsingState<C: WebSocketCallbackWithShutdownSignal> {
    callback: C,
}

/// Indicates that the callback uses the server state of type `State`
pub struct CallbackUsingState<State, C: WebSocketCallbackWithStateAndShutdownSignal<State>> {
    callback: C,
    state: PhantomData<fn(&State)>,
}

impl WebSocketUpgrade {
    /// Handle the websocket upgrade. The returned [`UpgradedWebSocket`] should be returned by the request handler,
    /// and thus returned to the client.
    ///
    /// `on_upgrade` also accepts a [`WebSocketCallback`], as all [`WebSocketCallback`] also implement [`WebSocketCallbackWithShutdownSignal`].
    pub fn on_upgrade<C: WebSocketCallbackWithShutdownSignal>(
        self,
        callback: C,
    ) -> UpgradedWebSocket<UnspecifiedProtocol, CallbackNotUsingState<C>> {
        super::assert_implements_into_response(UpgradedWebSocket {
            sec_websocket_accept: self.key,
            sec_websocket_protocol: UnspecifiedProtocol,
            upgrade_token: self.upgrade_token,
            callback: CallbackNotUsingState { callback },
        })
    }

    /// Handle the websocket upgrade, which requires access to the state. The returned [`UpgradedWebSocket`] should be returned by the request handler,
    /// and thus returned to the client.
    ///
    /// `on_upgrade` also accepts a [`WebSocketCallbackWithState`], as all [`WebSocketCallbackWithState`] also implement [`WebSocketCallbackWithStateAndShutdownSignal`].
    pub fn on_upgrade_using_state<State, C: WebSocketCallbackWithStateAndShutdownSignal<State>>(
        self,
        callback: C,
    ) -> UpgradedWebSocket<UnspecifiedProtocol, CallbackUsingState<State, C>> {
        super::assert_implements_into_response_with_state::<State, _>(UpgradedWebSocket {
            sec_websocket_accept: self.key,
            sec_websocket_protocol: UnspecifiedProtocol,
            upgrade_token: self.upgrade_token,
            callback: CallbackUsingState {
                callback,
                state: PhantomData,
            },
        })
    }
}

fn websocket_response<'a, B: super::Body + 'a>(
    sec_websocket_accept: &'a WebSocketKey,
    sec_websocket_protocol: Option<&'a str>,
    body: B,
) -> super::Response<impl super::HeadersIter + 'a, B> {
    super::Response {
        status_code: StatusCode::SWITCHING_PROTOCOLS,
        headers: [
            ("Upgrade", "websocket"),
            ("Connection", "upgrade"),
            (
                "Sec-WebSocket-Accept",
                // Safety:
                // sec_websocket_accept was created by data_encoding::BASE64.encode_mut, which creates a UTF-8 string
                #[allow(unsafe_code)]
                unsafe {
                    core::str::from_utf8_unchecked(sec_websocket_accept)
                },
            ),
        ],
        body,
    }
    .with_headers(
        sec_websocket_protocol
            .map(|sec_websocket_protocol| ("Sec-WebSocket-Protocol", sec_websocket_protocol)),
    )
}

impl<P: WebSocketProtocol, C: WebSocketCallbackWithShutdownSignal> super::IntoResponse
    for UpgradedWebSocket<P, CallbackNotUsingState<C>>
{
    async fn write_to<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        struct Body<C: WebSocketCallbackWithShutdownSignal> {
            upgrade_token: crate::extract::UpgradeToken,
            callback: CallbackNotUsingState<C>,
        }

        impl<C: WebSocketCallbackWithShutdownSignal> super::Body for Body<C> {
            async fn write_response_body<
                R: crate::io::Read,
                W: crate::io::Write<Error = R::Error>,
            >(
                self,
                connection: super::Connection<'_, R>,
                writer: W,
            ) -> Result<(), W::Error> {
                let shutdown_signal = connection.shutdown_signal.clone();

                self.callback
                    .callback
                    .run_with_shutdown_signal(
                        SocketRx {
                            reader: connection.upgrade(self.upgrade_token),
                        },
                        SocketTx { writer },
                        shutdown_signal,
                    )
                    .await
            }
        }

        let UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol,
            upgrade_token,
            callback,
        } = self;

        response_writer
            .write_response(
                connection,
                websocket_response(
                    &sec_websocket_accept,
                    sec_websocket_protocol.name(),
                    Body {
                        upgrade_token,
                        callback,
                    },
                ),
            )
            .await
    }
}

impl<State, P: WebSocketProtocol, C: WebSocketCallbackWithStateAndShutdownSignal<State>>
    super::IntoResponseWithState<State> for UpgradedWebSocket<P, CallbackUsingState<State, C>>
{
    async fn write_to_with_state<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        struct Body<'s, State, C: WebSocketCallbackWithStateAndShutdownSignal<State>> {
            state: &'s State,
            upgrade_token: crate::extract::UpgradeToken,
            callback: CallbackUsingState<State, C>,
        }

        impl<State, C: WebSocketCallbackWithStateAndShutdownSignal<State>> super::Body
            for Body<'_, State, C>
        {
            async fn write_response_body<
                R: crate::io::Read,
                W: crate::io::Write<Error = R::Error>,
            >(
                self,
                connection: super::Connection<'_, R>,
                writer: W,
            ) -> Result<(), W::Error> {
                let shutdown_signal = connection.shutdown_signal.clone();

                self.callback
                    .callback
                    .run_with_state_and_shutdown_signal(
                        self.state,
                        SocketRx {
                            reader: connection.upgrade(self.upgrade_token),
                        },
                        SocketTx { writer },
                        shutdown_signal,
                    )
                    .await
            }
        }

        let UpgradedWebSocket {
            sec_websocket_accept,
            sec_websocket_protocol,
            upgrade_token,
            callback,
        } = self;

        response_writer
            .write_response(
                connection,
                websocket_response(
                    &sec_websocket_accept,
                    sec_websocket_protocol.name(),
                    Body {
                        state,
                        upgrade_token,
                        callback,
                    },
                ),
            )
            .await
    }
}
