use std::{cell::RefCell, rc::Rc, time::Duration};

use picoserve::{
    extract::State,
    response::{IntoResponse, Redirect},
    routing::{get, parse_path_segment},
};

#[derive(Clone, serde::Serialize)]
struct Counter {
    counter: i32,
}

type SharedCounter = Rc<RefCell<Counter>>;

#[derive(Clone)]
struct AppState {
    connection_id: usize,
    counter: SharedCounter,
}

async fn get_counter(State(state): State<AppState>) -> impl IntoResponse {
    picoserve::response::Json(state.counter.borrow().clone())
        .into_response()
        .with_header("X-Connection-ID", state.connection_id)
}

async fn increment_counter(State(state): State<AppState>) -> impl IntoResponse {
    state.counter.borrow_mut().counter += 1;
    Redirect::to(".")
}

async fn set_counter(value: i32, State(state): State<AppState>) -> impl IntoResponse {
    state.counter.borrow_mut().counter = value;
    Redirect::to(".")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let counter = Rc::new(RefCell::new(Counter { counter: 0 }));

    let app = std::rc::Rc::new(
        picoserve::Router::<_, AppState>::new()
            .route("/", get(get_counter))
            .route("/increment", get(increment_counter))
            .route(("/set", parse_path_segment()), get(set_counter)),
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
            for connection_id in 0.. {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let counter = counter.clone();
                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    match picoserve::Server::new(
                        &app.shared().with_state(AppState {
                            connection_id,
                            counter,
                        }),
                        &config,
                        &mut [0; 2048],
                    )
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

            Ok(())
        })
        .await
}
