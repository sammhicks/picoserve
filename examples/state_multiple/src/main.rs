use std::{cell::RefCell, rc::Rc};

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

async fn get_counters(
    State((a_counter, b_counter)): State<(SharedCounter, SharedCounter)>,
) -> impl IntoResponse {
    #[derive(serde::Serialize)]
    struct Response {
        a: Counter,
        b: Counter,
    }

    picoserve::response::Json(Response {
        a: a_counter.borrow().clone(),
        b: b_counter.borrow().clone(),
    })
}

async fn get_counter(State(state): State<SharedCounter>) -> impl IntoResponse {
    picoserve::response::Json(state.borrow().clone())
}

async fn increment_counter(State(state): State<SharedCounter>) -> impl IntoResponse {
    state.borrow_mut().counter += 1;
    Redirect::to(".")
}

async fn set_counter(value: i32, State(state): State<SharedCounter>) -> impl IntoResponse {
    state.borrow_mut().counter = value;
    Redirect::to(".")
}

fn make_app<State>(
    counter: SharedCounter,
) -> picoserve::Router<impl picoserve::routing::PathRouter<State>, State> {
    picoserve::Router::new()
        .route("/", get(get_counter))
        .route("/increment", get(increment_counter))
        .route(("/set", parse_path_segment()), get(set_counter))
        .with_state(counter)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let a_counter = Rc::new(RefCell::new(Counter { counter: 0 }));
    let b_counter = Rc::new(RefCell::new(Counter { counter: 0 }));

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route("/", get(get_counters))
            .route("/a", get(|| async { Redirect::to("/a/") }))
            .route("/b", get(|| async { Redirect::to("/b/") }))
            .nest("/a", make_app(a_counter.clone()))
            .nest("/b", make_app(b_counter.clone()))
            .with_state((a_counter, b_counter)),
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
