use std::{cell::RefCell, rc::Rc, time::Duration};

use picoserve::{
    extract::State,
    routing::{get, get_service},
};

#[derive(Clone)]
struct AppState {
    value: Rc<RefCell<i32>>,
}

fn api_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new().route(
        "/value",
        get(|State(AppState { value })| picoserve::response::Json(*value.borrow())).post(
            |State(AppState { value }), picoserve::extract::Json::<_, 0>(new_value)| async move {
                *value.borrow_mut() = new_value
            },
        ),
    )
}

fn app_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new().nest("/api", api_router()).route(
        "/",
        get_service(picoserve::response::File::html(include_str!("index.html"))),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(app_router());

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
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
                let app_state = AppState {
                    value: Rc::new(RefCell::new(0)),
                };

                tokio::task::spawn_local(async move {
                    match picoserve::serve_with_state(
                        &app,
                        &config,
                        &mut [0; 2048],
                        stream,
                        &app_state,
                    )
                    .await
                    {
                        Ok(handled_requests_count) => {
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
