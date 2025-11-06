use std::{cell::RefCell, time::Duration};

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

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    tokio::task::spawn_local(async move {
                        match picoserve::Server::new(&app, &config, &mut [0; 2048])
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
                });
            }
        })
        .await
}
