use std::cell::RefCell;

use picoserve::{
    response::with_state::WithStateUpdate,
    routing::{get, get_service},
};

struct AppState {
    value: RefCell<i32>,
}

fn api_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new().route(
        "/value",
        get({
            struct GetValue(i32);

            impl picoserve::extract::FromRef<AppState> for GetValue {
                fn from_ref(input: &AppState) -> Self {
                    Self(*input.value.borrow())
                }
            }

            async |picoserve::extract::State(GetValue(value))| picoserve::response::Json(value)
        })
        .post(|picoserve::extract::Json(new_value)| async move {
            ().with_state_update(async move |state: &AppState| {
                *state.value.borrow_mut() = new_value
            })
        }),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app_state = AppState {
        value: RefCell::new(0),
    };

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .nest("/api", api_router())
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!("index.html"))),
            )
            .with_state(app_state),
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
                                "{} requests handled from {}",
                                handled_requests_count,
                                remote_address,
                            ),
                            Err(error) => {
                                log::error!(
                                    "Error handling requests from {remote_address}: {error}"
                                )
                            }
                        }
                    });
                });
            }
        })
        .await
}
