use std::cell::RefCell;

use picoserve::{
    extract::State,
    response::{with_state::WithStateUpdate, IntoResponse, IntoResponseWithState, Redirect},
    routing::{get, parse_path_segment},
};

struct AppState {
    value: RefCell<i32>,
}

#[derive(serde::Serialize)]
struct AppStateValue {
    value: i32,
}

impl picoserve::extract::FromRef<AppState> for AppStateValue {
    fn from_ref(AppState { value, .. }: &AppState) -> Self {
        Self {
            value: *value.borrow(),
        }
    }
}

async fn get_value(State(value): State<AppStateValue>) -> impl IntoResponse {
    picoserve::response::Json(value)
}

async fn increment_value() -> impl IntoResponseWithState<AppState> {
    Redirect::to(".").with_state_update(async |state: &AppState| {
        *state.value.borrow_mut() += 1;
    })
}

async fn set_value(value: i32) -> impl IntoResponseWithState<AppState> {
    Redirect::to("..").with_state_update(async move |state: &AppState| {
        *state.value.borrow_mut() = value;
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let state = AppState { value: 0.into() };

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route("/", get(get_value))
            .route("/increment", get(increment_value))
            .route(("/set", parse_path_segment()), get(set_value))
            .with_state(state),
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
