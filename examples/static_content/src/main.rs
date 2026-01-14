use picoserve::{
    response::{Directory, File},
    routing::get_service,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route("/", get_service(File::html(include_str!("index.html"))))
            .nest_service(
                "/static",
                const {
                    Directory {
                        files: &[("index.css", File::css(include_str!("index.css")))],
                        ..Directory::DEFAULT
                    }
                },
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
