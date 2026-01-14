use picoserve::{
    response::IntoResponse,
    routing::{get, get_service, parse_path_segment},
    ResponseSent,
};

struct PrefixOperation {
    operator: char,
    input: f32,
    output: f32,
}

impl IntoResponse for PrefixOperation {
    async fn write_to<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        self,
        connection: picoserve::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let Self {
            operator,
            input,
            output,
        } = self;

        format_args!("{operator}({input}) = {output}\n")
            .write_to(connection, response_writer)
            .await
    }
}

struct InfixOperation {
    input_0: f32,
    operator: char,
    input_1: f32,
    output: f32,
}

impl IntoResponse for InfixOperation {
    async fn write_to<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        self,
        connection: picoserve::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let Self {
            input_0,
            operator,
            input_1,
            output,
        } = self;

        format_args!("({input_0}) {operator} ({input_1}) = {output}\n")
            .write_to(connection, response_writer)
            .await
    }
}

struct MyService;

impl<State> picoserve::routing::RequestHandlerService<State, (f32,)> for MyService {
    async fn call_request_handler_service<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        &self,
        _state: &State,
        (n,): (f32,),
        request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        picoserve::response::DebugValue(("n", n))
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                ("/foo", parse_path_segment::<f32>()),
                get_service(MyService),
            )
            .route(
                ("/neg", parse_path_segment::<f32>()),
                get(|input| async move {
                    PrefixOperation {
                        operator: '-',
                        input,
                        output: -input,
                    }
                }),
            )
            .route(
                (
                    "/add",
                    parse_path_segment::<f32>(),
                    parse_path_segment::<f32>(),
                ),
                get(|(input_0, input_1)| async move {
                    InfixOperation {
                        input_0,
                        operator: '+',
                        input_1,
                        output: input_0 + input_1,
                    }
                }),
            )
            .route(
                (
                    "/sub",
                    parse_path_segment::<f32>(),
                    parse_path_segment::<f32>(),
                ),
                get(|(input_0, input_1)| async move {
                    InfixOperation {
                        input_0,
                        operator: '-',
                        input_1,
                        output: input_0 - input_1,
                    }
                }),
            ),
    );

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

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
                        }) => {
                            println!(
                                "{handled_requests_count} requests handled from {remote_address}"
                            )
                        }
                        Err(err) => println!("{err:?}"),
                    }
                });
            }
        })
        .await
}
