use std::{
    convert::Infallible,
    format,
    pin::Pin,
    string::String,
    task::{Context, Poll},
    time::Duration,
    vec::Vec,
};

use alloc::borrow::ToOwned;

use futures_util::FutureExt;
use http_body_util::BodyExt;
use hyper::StatusCode;
use tokio::sync::mpsc;

use self::routing::PathRouter;

use super::*;

use super::io::{Read, Write};

struct VecRead(Vec<u8>);

impl VecRead {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn read(&mut self, buf: &mut [u8]) -> usize {
        let read_size = self.0.len().min(buf.len());

        let (data, rest) = self.0.split_at(read_size);

        buf[..read_size].copy_from_slice(data);

        self.0 = rest.into();

        read_size
    }
}

struct PipeRx {
    current: VecRead,
    channel: mpsc::UnboundedReceiver<std::vec::Vec<u8>>,
}

impl io::ErrorType for PipeRx {
    type Error = Infallible;
}

impl Read for PipeRx {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if self.current.is_empty() {
            let Some(mut next) = self.channel.recv().await else {
                return Ok(0);
            };

            while let Ok(mut other) = self.channel.try_recv() {
                next.append(&mut other);
            }

            self.current = VecRead(next);
        }

        Ok(self.current.read(buf))
    }
}

impl hyper::rt::Read for PipeRx {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();

        if this.current.is_empty() {
            this.current = match this.channel.poll_recv(cx) {
                Poll::Ready(Some(item)) => VecRead(item),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            };
        }

        let read_size = this.current.read(
            // Safety:
            // Copied from MaybeUninit::slice_assume_init_mut.
            #[allow(unsafe_code, clippy::multiple_unsafe_ops_per_block)]
            unsafe {
                // TODO - replace with MaybeUninit::slice_assume_init_mut when stable
                &mut *(buf.as_mut() as *mut [std::mem::MaybeUninit<u8>] as *mut [u8])
            },
        );

        // Safety:
        // read_size comes from reading from the buffer and thus is at most the size of the buffer.
        #[allow(unsafe_code)]
        unsafe {
            buf.advance(read_size)
        };

        Poll::Ready(Ok(()))
    }
}

struct PipeTx(mpsc::UnboundedSender<std::vec::Vec<u8>>);

impl io::ErrorType for PipeTx {
    type Error = Infallible;
}

impl Write for PipeTx {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let _ = self.0.send(buf.into());

        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl hyper::rt::Write for PipeTx {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let _ = self.0.send(buf.into());

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }
}

fn pipe() -> (PipeTx, PipeRx) {
    let (tx, rx) = mpsc::unbounded_channel();

    (
        PipeTx(tx),
        PipeRx {
            current: VecRead(std::vec::Vec::new()),
            channel: rx,
        },
    )
}

struct TestSocket<TX, RX> {
    tx: TX,
    rx: RX,
}

impl<TX: Write<Error = Infallible>, RX: Read<Error = Infallible>> io::Socket<TokioRuntime>
    for TestSocket<TX, RX>
{
    type Error = Infallible;

    type ReadHalf<'a>
        = &'a mut RX
    where
        TX: 'a,
        RX: 'a;
    type WriteHalf<'a>
        = &'a mut TX
    where
        TX: 'a,
        RX: 'a;

    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
        (&mut self.rx, &mut self.tx)
    }

    async fn shutdown<Timer: time::Timer<TokioRuntime>>(
        self,
        _timeouts: &Timeouts<Timer::Duration>,
        _timer: &mut Timer,
    ) -> Result<(), Error<Self::Error>> {
        Ok(())
    }
}

impl<TX: Unpin, RX: hyper::rt::Read + Unpin> hyper::rt::Read for TestSocket<TX, RX> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().rx).poll_read(cx, buf)
    }
}

impl<TX: hyper::rt::Write + Unpin, RX: Unpin> hyper::rt::Write for TestSocket<TX, RX> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().tx).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().tx).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().tx).poll_shutdown(cx)
    }
}

async fn run_single_request_test(
    app: &Router<impl PathRouter>,
    request: hyper::Request<http_body_util::Full<hyper::body::Bytes>>,
) -> (hyper::http::response::Parts, hyper::body::Bytes) {
    let (request_tx, request_rx) = pipe();
    let (response_tx, response_rx) = pipe();

    let config = Config::new(Timeouts {
        start_read_request: None,
        persistent_start_read_request: None,
        read_request: None,
        write: None,
    });

    let mut http_buffer = [0; 2048];

    let server = std::pin::pin!(
        Server::new(app, &config, &mut http_buffer).serve(TestSocket {
            rx: request_rx,
            tx: response_tx,
        })
    );

    let (mut request_sender, connection) = hyper::client::conn::http1::handshake(TestSocket {
        tx: request_tx,
        rx: response_rx,
    })
    .now_or_never()
    .expect("handshake stalled")
    .unwrap();

    tokio::spawn(connection);

    let request = std::pin::pin!(request_sender.send_request(request));

    let (response, server_output) = tokio::time::timeout(
        Duration::from_secs(1),
        futures_util::future::join(request, server),
    )
    .await
    .unwrap();

    assert_eq!(server_output.unwrap().handled_requests_count, 1);

    let (parts, body) = response.unwrap().into_parts();

    (parts, body.collect().await.unwrap().to_bytes())
}

#[tokio::test]
/// Test that routing works
async fn simple_routing() {
    async fn run_test(path: &'static str, body: &'static str) {
        let (response_parts, response_body) = run_single_request_test(
            &Router::new().route(path, routing::get(|| async move { body })),
            hyper::Request::get(path).body(Default::default()).unwrap(),
        )
        .await;

        assert_eq!(response_parts.status, StatusCode::OK);

        assert_eq!(response_body, body.as_bytes());
    }

    for path in ["/", "/foo", "/bar"] {
        for body in ["a", "b", "c"] {
            run_test(path, body).await;
        }
    }
}

#[tokio::test]
/// Test that requesting a nonexistant route returns NOT_FOUND
async fn not_found() {
    let (response_parts, _response_body) = run_single_request_test(
        &Router::new().route("/", routing::get(|| async move {})),
        hyper::Request::get("/not_found")
            .body(Default::default())
            .unwrap(),
    )
    .await;

    assert_eq!(response_parts.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
/// Test that nesting works
async fn nesting() {
    use routing::get;

    const A: &str = "A";
    const B: &str = "B";

    const AA: &str = "AA";
    const AB: &str = "AB";
    const BA: &str = "BA";
    const BB: &str = "BB";

    const A_PATH: &str = "/a";
    const B_PATH: &str = "/b";

    const AA_PATH: &str = "/a/a";
    const AB_PATH: &str = "/a/b";
    const BA_PATH: &str = "/b/a";
    const BB_PATH: &str = "/b/b";

    async fn run_tests(app: Router<impl PathRouter>) {
        async fn run_test(app: &Router<impl PathRouter>, path: &str, expected_body: &str) {
            let (response_parts, response_body) = run_single_request_test(
                app,
                hyper::Request::get(path).body(Default::default()).unwrap(),
            )
            .await;

            assert_eq!(response_parts.status, StatusCode::OK);

            assert_eq!(response_body, expected_body.as_bytes());
        }

        run_test(&app, A_PATH, A).await;
        run_test(&app, AA_PATH, AA).await;
        run_test(&app, AB_PATH, AB).await;
        run_test(&app, B_PATH, B).await;
        run_test(&app, BA_PATH, BA).await;
        run_test(&app, BB_PATH, BB).await;
    }

    fn add_direct_routes(router: Router<impl PathRouter>) -> Router<impl PathRouter> {
        router
            .route(A_PATH, get(|| async { A }))
            .route(B_PATH, get(|| async { B }))
    }

    fn add_nested_routes(router: Router<impl PathRouter>) -> Router<impl PathRouter> {
        router
            .nest(
                A_PATH,
                Router::new()
                    .route(A_PATH, get(|| async { AA }))
                    .route(B_PATH, get(|| async { AB })),
            )
            .nest(
                B_PATH,
                Router::new()
                    .route(A_PATH, get(|| async { BA }))
                    .route(B_PATH, get(|| async { BB })),
            )
    }

    run_tests(add_direct_routes(add_nested_routes(Router::new()))).await;
    run_tests(add_nested_routes(add_direct_routes(Router::new()))).await;
}

#[tokio::test]
/// Test file and directory routing
async fn file_routing() {
    use response::fs::{Directory, File};

    const HTML: &str = "<h1>Hello World</h1>";
    const CSS: &str = "h1 { font-weight: bold; }";

    const STATIC_DIR: &str = "/static";
    const HTML_PATH: &str = "index.html";
    const STYLES_DIRECTORY: &str = "styles";
    const CSS_PATH: &str = "index.css";

    const FILES: Directory = Directory {
        files: &[(HTML_PATH, File::html(HTML))],
        sub_directories: &[(
            STYLES_DIRECTORY,
            Directory {
                files: &[(CSS_PATH, File::css(CSS))],
                ..Directory::DEFAULT
            },
        )],
    };

    let app = Router::new().nest_service(STATIC_DIR, FILES);

    {
        let (parts, body) = run_single_request_test(
            &app,
            hyper::Request::get(format!("{STATIC_DIR}/{HTML_PATH}"))
                .body(Default::default())
                .unwrap(),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(body, HTML.as_bytes());
    }

    {
        let (parts, body) = run_single_request_test(
            &app,
            hyper::Request::get(format!("{STATIC_DIR}/{STYLES_DIRECTORY}/{CSS_PATH}"))
                .body(Default::default())
                .unwrap(),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(body, CSS.as_bytes());
    }

    for path in [
        format!("/{HTML_PATH}"),
        format!("/{STATIC_DIR}/{CSS_PATH}"),
        format!("/{STATIC_DIR}/{STYLES_DIRECTORY}/{HTML_PATH}"),
    ] {
        let (parts, _body) = run_single_request_test(
            &app,
            hyper::Request::get(&path).body(Default::default()).unwrap(),
        )
        .await;

        assert_eq!(
            parts.status,
            StatusCode::NOT_FOUND,
            "{path} should not have been found"
        );
    }
}

#[tokio::test]
/// Test file and directory routing
async fn file_etag_based_cache() {
    const HTML: &str = "<h1>Hello World</h1>";

    let app = Router::new().route("/", routing::get_service(response::File::html(HTML)));

    let etag;

    {
        let (parts, body) = run_single_request_test(
            &app,
            hyper::Request::get("/").body(Default::default()).unwrap(),
        )
        .await;

        assert_eq!(parts.status, StatusCode::OK);
        assert_eq!(body, HTML.as_bytes());

        etag = parts
            .headers
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();

        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
        assert_eq!(etag.len(), 42);
    }

    {
        let (parts, body) = run_single_request_test(
            &app,
            hyper::Request::get("/")
                .header("If-None-Match", etag)
                .body(Default::default())
                .unwrap(),
        )
        .await;

        assert_eq!(parts.status, StatusCode::NOT_MODIFIED);
        assert_eq!(&body[..], b"");
    }
}

#[tokio::test]
/// Test that only a single request is handled if configured to close the connection
async fn only_one_request() {
    let (request_tx, request_rx) = pipe();
    let (response_tx, response_rx) = pipe();

    let app = Router::new().route("/", routing::get(|| async move { "Hello World" }));

    let config = Config::new(Timeouts {
        start_read_request: None,
        persistent_start_read_request: None,
        read_request: None,
        write: None,
    });

    let mut http_buffer = [0; 2048];

    let server = Server::new(&app, &config, &mut http_buffer).serve(TestSocket {
        rx: request_rx,
        tx: response_tx,
    });

    request_tx
        .0
        .send(
            "GET / HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\n\r\n"
                .as_bytes()
                .into(),
        )
        .unwrap();

    drop(request_tx);

    assert_eq!(
        server
            .now_or_never()
            .expect("Server has stalled")
            .unwrap()
            .handled_requests_count,
        1
    );

    drop(response_rx);
}

#[tokio::test]
/// Test that multiple requests are handled if the connection is kept alive
async fn keep_alive() {
    let app = Router::new().route("/", routing::get(|| async move { "Hello World" }));

    let config = Config::new(Timeouts {
        start_read_request: None,
        persistent_start_read_request: None,
        read_request: None,
        write: None,
    })
    .keep_connection_alive();

    let mut http_buffer = [0; 2048];

    let server = Server::new(&app, &config, &mut http_buffer).serve(TestSocket {
        rx: "GET / HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\n\r\n".as_bytes(),
        tx: std::vec::Vec::new(),
    });

    assert_eq!(
        server
            .now_or_never()
            .expect("Server has stalled")
            .unwrap()
            .handled_requests_count,
        2
    );
}

#[tokio::test]
/// Test correctly processing reading a request with each of
///  - A two different forced breaks in reading from the "client"
///  - Each of
///    - Not reading the body, and thus discarding it
///    - Reading part of the body into an external buffer
///    - Reading all of the body into an external buffer
///    - Attempting to read more than the entire body, testing that the body reader stops reading at the end of the body
///    - Reading the entire body into the internal buffer
async fn upgrade_with_request_body() {
    const EXPECTED_BODY: &[u8] = b"BODY";
    const EXPECTED_UPGRADE: &[u8] = b"UPGRADE";
    const REQUEST_PAYLOAD: &[u8] =
        b"POST / HTTP/1.1\r\nUpgrade: test\r\nContent-Length: 4\r\n\r\nBODYUPGRADE";

    struct VecSequence {
        current: VecRead,
        rest_reversed: Vec<Vec<u8>>,
    }

    impl io::ErrorType for VecSequence {
        type Error = Infallible;
    }

    impl Read for VecSequence {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.current.is_empty() {
                self.current = match self.rest_reversed.pop() {
                    Some(value) => VecRead(value),
                    None => return Ok(0),
                };
            }

            Ok(self.current.read(buf))
        }
    }

    struct UpgradeCheck {
        upgrade_token: extract::UpgradeToken,
    }

    impl response::Body for UpgradeCheck {
        async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
            self,
            connection: response::Connection<'_, R>,
            _writer: W,
        ) -> Result<(), W::Error> {
            let mut actual = [0; EXPECTED_UPGRADE.len()];

            connection
                .upgrade(self.upgrade_token)
                .read_exact(&mut actual)
                .await
                .unwrap();

            assert_eq!(EXPECTED_UPGRADE, actual);

            Ok(())
        }
    }

    #[derive(Debug)]
    enum BodyReadType {
        DoNotRead,
        ReadAll,
        ReadExternally { buffer_size: usize },
    }

    struct BodyCheck {
        read_body: BodyReadType,
    }

    impl routing::RequestHandlerService<()> for BodyCheck {
        async fn call_request_handler_service<
            R: Read,
            W: response::ResponseWriter<Error = R::Error>,
        >(
            &self,
            state: &(),
            (): (),
            mut request: request::Request<'_, R>,
            response_writer: W,
        ) -> Result<ResponseSent, W::Error> {
            use extract::FromRequestParts;
            use response::IntoResponse;

            let upgrade_token = extract::UpgradeToken::from_request_parts(state, &request.parts)
                .await
                .unwrap();

            match self.read_body {
                BodyReadType::DoNotRead => (),
                BodyReadType::ReadAll => {
                    let actual_body = request.body_connection.body().read_all().await.unwrap();

                    assert_eq!(actual_body, EXPECTED_BODY);
                }
                BodyReadType::ReadExternally { buffer_size } => {
                    let mut buffer = std::vec![0; buffer_size];

                    let mut reader = request.body_connection.body().reader();

                    let mut read_position = 0;

                    loop {
                        let read_buffer = &mut buffer[read_position..];

                        if read_buffer.is_empty() {
                            break;
                        }

                        let read_size = reader.read(read_buffer).await.unwrap();

                        if read_size == 0 {
                            break;
                        }

                        read_position += read_size;
                    }

                    let expected_body = EXPECTED_BODY;
                    let expected_body = &expected_body[..(buffer_size.min(expected_body.len()))];

                    assert_eq!(expected_body, &buffer[..read_position]);
                }
            }

            let connection = request.body_connection.finalize().await?;

            response::Response {
                status_code: response::StatusCode::OK,
                headers: [("Content-Type", "text/plain"), ("Content-Length", "0")],
                body: UpgradeCheck { upgrade_token },
            }
            .write_to(connection, response_writer)
            .await
        }
    }

    let config = Config::new(Timeouts {
        start_read_request: None,
        persistent_start_read_request: None,
        read_request: None,
        write: None,
    });

    let mut http_buffer = [0; 2048];

    for a in 0..REQUEST_PAYLOAD.len() {
        for b in a..REQUEST_PAYLOAD.len() {
            for read_body in [BodyReadType::DoNotRead, BodyReadType::ReadAll]
                .into_iter()
                .chain((1..=6).map(|buffer_size| BodyReadType::ReadExternally { buffer_size }))
            {
                let app = Router::new().route("/", routing::post_service(BodyCheck { read_body }));

                let server = Server::new(&app, &config, &mut http_buffer).serve(TestSocket {
                    rx: VecSequence {
                        current: VecRead(Vec::new()),
                        rest_reversed: [
                            &REQUEST_PAYLOAD[b..],
                            &REQUEST_PAYLOAD[a..b],
                            &REQUEST_PAYLOAD[..a],
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .map(Vec::from)
                        .collect(),
                    },
                    tx: Vec::new(),
                });

                assert_eq!(
                    server
                        .now_or_never()
                        .expect("Server has stalled")
                        .unwrap()
                        .handled_requests_count,
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn huge_request() {
    let request_body = ('a'..='z').cycle().take(10000).collect::<String>();

    struct ReadBody {
        expected_body: Option<String>,
    }

    impl routing::RequestHandlerService<()> for ReadBody {
        async fn call_request_handler_service<
            R: Read,
            W: response::ResponseWriter<Error = R::Error>,
        >(
            &self,
            (): &(),
            (): (),
            mut request: request::Request<'_, R>,
            response_writer: W,
        ) -> Result<ResponseSent, W::Error> {
            if let Some(expected_body) = &self.expected_body {
                let mut buffer = std::vec![0; expected_body.len()];

                request
                    .body_connection
                    .body()
                    .reader()
                    .read_exact(&mut buffer)
                    .await
                    .unwrap();

                assert_eq!(expected_body.as_bytes(), buffer.as_slice());
            }

            response_writer
                .write_response(
                    request.body_connection.finalize().await?,
                    response::Response::ok("Hello"),
                )
                .await
        }
    }

    for read_length in [None, Some(26), Some(request_body.len())] {
        let expected_body = read_length.map(|length| request_body[..length].into());

        let app = Router::new().route("/", routing::post_service(ReadBody { expected_body }));

        let response = run_single_request_test(
            &app,
            hyper::Request::post("/")
                .body(request_body.clone().into())
                .unwrap(),
        )
        .await;

        assert_eq!(response.0.status, hyper::http::StatusCode::OK);
    }
}

#[tokio::test]
async fn from_request_macros() {
    use response::IntoResponse;

    const TEST_HEADER_NAME: &str = "test";
    const TEST_HEADER_VALUE: &str = "Test Header";

    const BODY_VALUE: &str = "Test Body";

    struct TestHeader<'r>(&'r str);

    impl<'r, State> crate::extract::FromRequestParts<'r, State> for TestHeader<'r> {
        type Rejection = core::convert::Infallible;

        async fn from_request_parts(
            _state: &'r State,
            request_parts: &request::RequestParts<'r>,
        ) -> Result<Self, Self::Rejection> {
            // `expect` and `unwrap` are allowed as it's a test

            Ok(Self(
                request_parts
                    .headers()
                    .get(TEST_HEADER_NAME)
                    .expect("Header Missing")
                    .as_str()
                    .unwrap(),
            ))
        }
    }

    struct BorrowingService;

    impl crate::routing::RequestHandlerService<()> for BorrowingService {
        async fn call_request_handler_service<
            R: Read,
            W: response::ResponseWriter<Error = R::Error>,
        >(
            &self,
            state: &(),
            (): (),
            mut request: request::Request<'_, R>,
            response_writer: W,
        ) -> Result<ResponseSent, W::Error> {
            let TestHeader(header) =
                crate::from_request_parts!(state, request, response_writer, TestHeader);
            let body = crate::from_request!(state, request, response_writer, &str);

            assert_eq!(header, TEST_HEADER_VALUE);
            assert_eq!(body, BODY_VALUE);

            ().write_to(request.body_connection.finalize().await?, response_writer)
                .await
        }
    }

    let app = Router::new().route("/", crate::routing::get_service(BorrowingService));

    let (parts, _) = run_single_request_test(
        &app,
        hyper::Request::get("/")
            .header(TEST_HEADER_NAME, TEST_HEADER_VALUE)
            .body(BODY_VALUE.into())
            .unwrap(),
    )
    .await;

    assert_eq!(parts.status, StatusCode::OK);
}
