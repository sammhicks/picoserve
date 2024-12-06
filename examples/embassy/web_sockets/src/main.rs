#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cyw43_pio::PioSpi;
use embassy_rp::{
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0},
    pio::Pio,
};
use embassy_time::Duration;
use panic_persist as _;
use picoserve::{
    make_static,
    response::ws,
    routing::{get, get_service},
    AppBuilder, AppRouter,
};
use rand::Rng;

embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::task]
async fn logger_task(usb: embassy_rp::peripherals::USB) {
    let driver = embassy_rp::usb::Driver::new(usb, Irqs);
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut stack: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

struct AppProps;

impl AppBuilder for AppProps {
    type PathRouter = impl picoserve::routing::PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        picoserve::Router::new()
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!("index.html"))),
            )
            .route(
                "/index.css",
                get_service(picoserve::response::File::css(include_str!("index.css"))),
            )
            .route(
                "/index.js",
                get_service(picoserve::response::File::javascript(include_str!(
                    "index.js"
                ))),
            )
            .route(
                "/ws",
                get(|upgrade: picoserve::response::WebSocketUpgrade| {
                    upgrade.on_upgrade(WebsocketEcho).with_protocol("echo")
                }),
            )
    }
}

const WEB_TASK_POOL_SIZE: usize = 8;

#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
async fn web_task(
    id: usize,
    stack: embassy_net::Stack<'static>,
    app: &'static AppRouter<AppProps>,
    config: &'static picoserve::Config<Duration>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::listen_and_serve(
        id,
        app,
        config,
        stack,
        port,
        &mut tcp_rx_buffer,
        &mut tcp_tx_buffer,
        &mut http_buffer,
    )
    .await
}

struct WebsocketEcho;

impl ws::WebSocketCallback for WebsocketEcho {
    async fn run<R: embedded_io_async::Read, W: embedded_io_async::Write<Error = R::Error>>(
        self,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        let mut buffer = [0; 1024];

        let close_reason = loop {
            match rx.next_message(&mut buffer).await {
                Ok(ws::Message::Text(data)) => tx.send_text(data).await,
                Ok(ws::Message::Binary(data)) => tx.send_binary(data).await,
                Ok(ws::Message::Close(reason)) => {
                    log::info!("Websocket close reason: {reason:?}");
                    break None;
                }
                Ok(ws::Message::Ping(data)) => tx.send_pong(data).await,
                Ok(ws::Message::Pong(_)) => continue,
                Err(err) => {
                    log::error!("Websocket Error: {err:?}");

                    let code = match err {
                        ws::ReadMessageError::Io(err) => return Err(err),
                        ws::ReadMessageError::ReadFrameError(_)
                        | ws::ReadMessageError::MessageStartsWithContinuation
                        | ws::ReadMessageError::UnexpectedMessageStart => 1002,
                        ws::ReadMessageError::ReservedOpcode(_) => 1003,
                        ws::ReadMessageError::TextIsNotUtf8 => 1007,
                    };

                    break Some((code, "Websocket Error"));
                }
            }?;
        };

        tx.close(close_reason).await
    }
}

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = embassy_rp::init(Default::default());

    spawner.must_spawn(logger_task(p.USB));

    if let Some(panic_message) = panic_persist::get_panic_message_utf8() {
        loop {
            log::error!("{panic_message}");
            embassy_time::Timer::after_secs(5).await;
        }
    }

    let fw = include_bytes!("../../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../../cyw43-firmware/43439A0_clm.bin");

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = cyw43_pio::PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    let state = make_static!(cyw43::State, cyw43::State::new());

    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    spawner.must_spawn(wifi_task(runner));

    control.init(clm).await;

    let (stack, runner) = embassy_net::new(
        net_device,
        embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(192, 168, 0, 1), 24),
            gateway: None,
            dns_servers: Default::default(),
        }),
        make_static!(
            embassy_net::StackResources::<WEB_TASK_POOL_SIZE>,
            embassy_net::StackResources::new()
        ),
        embassy_rp::clocks::RoscRng.gen(),
    );

    spawner.must_spawn(net_task(runner));

    control
        .start_ap_wpa2(
            example_secrets::WIFI_SSID,
            example_secrets::WIFI_PASSWORD,
            8,
        )
        .await;

    let app = make_static!(AppRouter<AppProps>, AppProps.build_app());

    let config = make_static!(
        picoserve::Config::<Duration>,
        picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            read_request: Some(Duration::from_secs(1)),
            write: Some(Duration::from_secs(1)),
        })
        .keep_connection_alive()
    );

    for id in 0..WEB_TASK_POOL_SIZE {
        spawner.must_spawn(web_task(id, stack, app, config));
    }
}
