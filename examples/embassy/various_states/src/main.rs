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
    extract::FromRequestParts, make_static, response::IntoResponse, routing::get, AppRouter,
    AppWithStateBuilder,
};
use portable_atomic::AtomicUsize;
use rand::Rng;

embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::task]
async fn logger_task(usb: embassy_rp::Peri<'static, embassy_rp::peripherals::USB>) {
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

/// The state used by different parts of the web app.
struct AppState {
    state_request_count: AtomicUsize,
}

/// An extractor for the next state request, which increments `state_request_count` when extracted.
struct NextStateRequestCount(usize);

impl<'r> picoserve::extract::FromRequestParts<'r, AppState> for NextStateRequestCount {
    type Rejection = core::convert::Infallible;

    async fn from_request_parts(
        state: &'r AppState,
        _request_parts: &picoserve::request::RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            state
                .state_request_count
                .fetch_add(1, core::sync::atomic::Ordering::SeqCst),
        ))
    }
}

/// A service which is called if none of the routes match, which has its own internal state as well as configuration.
struct FallbackService {
    service_name: &'static str,
    service_request_count: AtomicUsize,
}

impl picoserve::routing::PathRouterService<AppState> for FallbackService {
    async fn call_request_handler_service<
        R: embedded_io_async::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        &self,
        state: &AppState,
        (): (),
        path: picoserve::request::Path<'_>,
        request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let service_name = self.service_name;

        let service_request_count = self
            .service_request_count
            .fetch_add(1, core::sync::atomic::Ordering::SeqCst);

        let Ok(NextStateRequestCount(state_request_count)) =
            NextStateRequestCount::from_request_parts(state, &request.parts).await;

        format_args!("Fallback service. Name: {service_name}, Service Request Count: {service_request_count}, State Reqeust Count: {state_request_count}, Path: {path:?}\n")
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

/// Config-Time properties of the web app
struct AppProps {
    service_name: &'static str,
}

impl AppWithStateBuilder for AppProps {
    type State = AppState;
    type PathRouter = impl picoserve::routing::PathRouter<AppState>;

    fn build_app(self) -> picoserve::Router<Self::PathRouter, AppState> {
        // destructure the props to access the configuration
        let Self { service_name } = self;

        // setup the initial state of the FallbackService and configure the routes
        picoserve::Router::from_service(FallbackService {
            service_name,
            service_request_count: AtomicUsize::new(0),
        })
        .route(
            "/",
            get(|NextStateRequestCount(state_request_count)| async move {
                picoserve::response::DebugValue(("Handler", state_request_count))
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
    state: &'static AppState,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::listen_and_serve_with_state(
        id,
        app,
        config,
        stack,
        port,
        &mut tcp_rx_buffer,
        &mut tcp_tx_buffer,
        &mut http_buffer,
        state,
    )
    .await
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
        cyw43_pio::DEFAULT_CLOCK_DIVIDER,
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
            address: embassy_net::Ipv4Cidr::new(core::net::Ipv4Addr::new(192, 168, 0, 1), 24),
            gateway: None,
            dns_servers: Default::default(),
        }),
        make_static!(
            embassy_net::StackResources<WEB_TASK_POOL_SIZE>,
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

    // Possibly loaded from flash or some other config.
    let service_name = "Fallback Service";

    let app = make_static!(AppRouter<AppProps>, AppProps { service_name }.build_app());

    let config = make_static!(
        picoserve::Config<Duration>,
        picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            persistent_start_read_request: Some(Duration::from_secs(1)),
            read_request: Some(Duration::from_secs(1)),
            write: Some(Duration::from_secs(1)),
        })
        .keep_connection_alive()
    );

    let app_state = make_static!(
        AppState,
        AppState {
            state_request_count: AtomicUsize::new(0)
        }
    );

    for id in 0..WEB_TASK_POOL_SIZE {
        spawner.must_spawn(web_task(id, stack, app, config, app_state));
    }
}
