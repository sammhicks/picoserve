#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cyw43_pio::PioSpi;
use embassy_rp::{
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0},
    pio::Pio,
};

use embassy_sync::watch::Watch;
use embassy_time::Duration;
use panic_persist as _;
use picoserve::{make_static, routing::get};
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

// struct AppProps {
//     spawner: embassy_executor::Spawner,
// }

// impl AppBuilder for AppProps {
//     type PathRouter = impl picoserve::routing::PathRouter;

//     fn build_app(self) -> picoserve::Router<Self::PathRouter> {
//         let Self { spawner } = self;

//         picoserve::Router::new()
//             .route(
//                 "/",
//                 get(|| async {
//                     "Hello World\n\nNavigate to /suspend to temporarily shutdown the server."
//                 }),
//             )
//             .route(
//                 "/suspend",
//                 get(move || async move {
//                     match spawner.spawn(suspend_server()) {
//                         Ok(()) => "Server suspended",
//                         Err(_) => "Failed to suspend server",
//                     }
//                 }),
//             )
//     }
// }

const WEB_TASK_POOL_SIZE: usize = 8;

#[derive(Clone)]
enum ServerState {
    Running,
    Shutdown,
}

impl ServerState {
    fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    fn is_shutdown(&self) -> bool {
        matches!(self, Self::Shutdown)
    }
}

static SERVER_STATE: Watch<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    ServerState,
    { WEB_TASK_POOL_SIZE + 1 },
> = Watch::new_with(ServerState::Running);

#[embassy_executor::task]
async fn suspend_server() {
    log::info!("Shutting down server");
    SERVER_STATE.sender().send(ServerState::Shutdown);

    embassy_time::Timer::after_secs(5).await;

    log::info!("Resuming server");
    SERVER_STATE.sender().send(ServerState::Running);
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

    let app = &picoserve::Router::new()
        .route(
            "/",
            get(|| async {
                "Hello World\n\nNavigate to /suspend to temporarily shutdown the server.\n"
            }),
        )
        .route(
            "/suspend",
            get(move || async move {
                match spawner.spawn(suspend_server()) {
                    Ok(()) => "Server suspended\n",
                    Err(_) => "Failed to suspend server\n",
                }
            }),
        );

    let config = &picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let mut server_state = SERVER_STATE.receiver().unwrap();

    loop {
        log::info!("Waiting for startup");

        server_state.get_and(ServerState::is_running).await;

        embassy_futures::join::join_array::<_, WEB_TASK_POOL_SIZE>(core::array::from_fn(
            |task_id| {
                let mut server_state = SERVER_STATE.receiver().unwrap();

                async move {
                    let port = 80;
                    let mut tcp_rx_buffer = [0; 1024];
                    let mut tcp_tx_buffer = [0; 1024];
                    let mut http_buffer = [0; 2048];
                    let shutdown_timeout = embassy_time::Duration::from_secs(3);

                    picoserve::Server::new(app, config, &mut http_buffer)
                        .with_graceful_shutdown(
                            server_state.get_and(ServerState::is_shutdown),
                            shutdown_timeout,
                        )
                        .listen_and_serve(
                            task_id,
                            stack,
                            port,
                            &mut tcp_rx_buffer,
                            &mut tcp_tx_buffer,
                        )
                        .await;
                }
            },
        ))
        .await;
    }
}
