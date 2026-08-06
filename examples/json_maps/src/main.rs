use std::collections::BTreeMap;

use picoserve::{
    response::json::{serialize_map_as_array_of_pairs, serialize_map_as_object},
    routing::get,
};

#[derive(serde::Serialize)]
struct Values {
    unchanged: BTreeMap<&'static str, i32>,
    #[serde(serialize_with = "serialize_map_as_object")]
    as_object: BTreeMap<&'static str, i32>,
    #[serde(serialize_with = "serialize_map_as_array_of_pairs")]
    as_array_of_pairs: BTreeMap<&'static str, i32>,
}

impl Default for Values {
    fn default() -> Self {
        let values = BTreeMap::from([("a", 1), ("b", 2)]);

        Self {
            unchanged: values.clone(),
            as_object: values.clone(),
            as_array_of_pairs: values,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get(async || picoserve::response::Json(Values::default())),
            )
            .route(
                "/array",
                get(async || {
                    picoserve::response::Json(Values::default()).serialize_map_as_array_of_pairs()
                }),
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
