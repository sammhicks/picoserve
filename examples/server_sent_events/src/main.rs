use picoserve::{
    response::{self, StatusCode},
    routing::{get, get_service, post},
};

struct NewMessage(String);

impl<'r, State> picoserve::extract::FromRequest<'r, State> for NewMessage {
    type Rejection = picoserve::extract::FailedToExtractEntireBodyAsStringError;

    async fn from_request<R: picoserve::io::Read>(
        state: &'r State,
        request_parts: picoserve::request::RequestParts<'r>,
        request_body: picoserve::request::RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        String::from_request(state, request_parts, request_body)
            .await
            .map(Self)
    }
}

struct Events(tokio::sync::watch::Receiver<String>);

impl response::sse::EventSource for Events {
    async fn write_events<W: picoserve::io::Write>(
        mut self,
        mut writer: response::sse::EventWriter<'_, W>,
    ) -> Result<(), W::Error> {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(15), self.0.changed()).await {
                Ok(Ok(())) => {
                    writer
                        .write_event("message_changed", self.0.borrow_and_update().as_str())
                        .await?
                }
                Ok(Err(_)) => return Ok(()),
                Err(_) => writer.write_keepalive().await?,
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let (messages_tx, messages_rx) = tokio::sync::watch::channel(String::new());

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get_service(response::File::html(include_str!("index.html"))),
            )
            .route(
                "/index.css",
                get_service(response::File::css(include_str!("index.css"))),
            )
            .route(
                "/index.js",
                get_service(response::File::javascript(include_str!("index.js"))),
            )
            .route(
                "/set_message",
                post(async move |NewMessage(message)| {
                    messages_tx
                        .send(message)
                        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to send message"))
                }),
            )
            .route(
                "/events",
                get(async move || response::EventStream(Events(messages_rx.clone()))),
            ),
    );

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    log::info!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                log::info!("Connection from {remote_address}");

                let app = app.clone();

                tokio::task::spawn_local(async move {
                    static CONFIG: picoserve::Config =
                        picoserve::Config::const_default().keep_connection_alive();

                    match picoserve::Server::new_tokio(&app, &CONFIG, &mut [0; 2048])
                        .serve(stream)
                        .await
                    {
                        Ok(picoserve::DisconnectionInfo {
                            handled_requests_count,
                            ..
                        }) => log::info!(
                            "{handled_requests_count} requests handled from {remote_address}",
                        ),
                        Err(error) => {
                            log::error!("Error handling requests from {remote_address}: {error}")
                        }
                    }
                });
            }
        })
        .await
}
