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

async fn get_counter(State(state): State<SharedCounter>) -> impl IntoResponse {
    picoserve::response::Json(state.borrow().clone())
}

async fn increment_counter(State(state): State<Rc<RefCell<Counter>>>) -> impl IntoResponse {
    state.borrow_mut().counter += 1;
    Redirect::to("/")
}

async fn set_counter(value: i32, State(state): State<Rc<RefCell<Counter>>>) -> impl IntoResponse {
    state.borrow_mut().counter = value;
    Redirect::to("/")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route("/", get(get_counter))
            .route("/increment", get(increment_counter))
            .route(("/set", parse_path_segment()), get(set_counter)),
    );

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 8000)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    match picoserve::serve_with_state(
                        &app,
                        &config,
                        &mut [0; 2048],
                        stream,
                        &Rc::new(RefCell::new(Counter { counter: 0 })),
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
