#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![recursion_limit = "256"]

use embassy_rp::{
    gpio::{Level, Output},
    pio::Pio,
};

use embassy_sync::watch::Watch;
use embassy_time::Duration;
use panic_persist as _;
use picoserve::{make_static, routing::get};
use rand::Rng;

embassy_rp::bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::task]
async fn logger_task(usb: embassy_rp::Peri<'static, embassy_rp::peripherals::USB>) {
    use example_utils::log::{CommandHandler, ReceiverHandler};

    let driver = embassy_rp::usb::Driver::new(usb, Irqs);
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver, CommandHandler);
}

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        cyw43::SpiBus<
            Output<'static>,
            cyw43_pio::PioSpi<'static, embassy_rp::peripherals::PIO0, 0>,
        >,
        cyw43::Cyw43439
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut stack: embassy_net::Runner<'static>) -> ! {
    stack.run().await
}

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

// Larger timeouts to demonstrate rapid graceful shutdown
static CONFIG: picoserve::Config = picoserve::Config::new(picoserve::Timeouts {
    start_read_request: Duration::from_secs(10),
    persistent_start_read_request: Duration::from_secs(10),
    read_request: Duration::from_secs(1),
    write: Duration::from_secs(1),
})
.keep_connection_alive();

type SharedListener<'s> = embassy_sync::mutex::Mutex<embassy_sync::blocking_mutex::raw::NoopRawMutex, embassy_net::tcp::TcpListener<'s>>;

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = embassy_rp::init(Default::default());

    spawner.spawn(logger_task(p.USB).unwrap());

    if let Some(panic_message) = panic_persist::get_panic_message_utf8() {
        loop {
            log::error!("{panic_message}");
            embassy_time::Timer::after_secs(5).await;
        }
    }

    let fw = cyw43::aligned_bytes!("../../cyw43-firmware/43439A0.bin");
    let clm = cyw43::aligned_bytes!("../../cyw43-firmware/43439A0_clm.bin");
    let nvram = cyw43::aligned_bytes!("../../cyw43-firmware/nvram_rp2040.bin");

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
        embassy_rp::dma::Channel::new(p.DMA_CH0, Irqs),
        embassy_rp::dma::Channel::new(p.DMA_CH1, Irqs),
    );

    let state = make_static!(cyw43::State, cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    spawner.spawn(wifi_task(runner).unwrap());

    control.init(clm).await;

    let (stack, runner) = embassy_net::Stack::new(
        make_static!(
            embassy_net::StackStorage,
            embassy_net::StackStorage::new()
        ),
        embassy_rp::clocks::RoscRng.random(),
    );

    let iface = stack.add_iface(make_static!(cyw43::NetDriver<'static>, net_device)).unwrap();
    iface.add_ip_addr(embassy_net::wire::IpCidr::new(example_utils::ADDRESS.into(), 24)).unwrap();
    iface.set_dhcpv4_server(Some(example_utils::dhcp_server_config()));

    spawner.spawn(net_task(runner).unwrap());

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
                match suspend_server() {
                    Ok(spawn_token) => {
                        spawner.spawn(spawn_token);
                        "Server suspended\n"
                    }
                    Err(_) => "Failed to suspend server\n",
                }
            }),
        );

    let mut server_state = SERVER_STATE.receiver().unwrap();

    log::info!("{}", example_utils::WELCOME_MESSAGE);

    const PORT: u16 = 80;
    let listener = {
        let mut listener = embassy_net::tcp::TcpListener::new(stack).unwrap();
        listener.listen(PORT).unwrap();
        log::info!("Listening on TCP:{}", PORT);
        &*make_static!(SharedListener, SharedListener::new(listener))
    };

    loop {
        log::info!("Waiting for startup");

        server_state.get_and(ServerState::is_running).await;

        embassy_futures::join::join_array::<_, WEB_TASK_POOL_SIZE>(core::array::from_fn(
            |task_id| {
                let mut server_state = SERVER_STATE.receiver().unwrap();

                async move {
                    let mut tcp_rx_buffer = [0; 1024];
                    let mut tcp_tx_buffer = [0; 1024];
                    let mut http_buffer = [0; 2048];
                    let shutdown_timeout = embassy_time::Duration::from_secs(3);

                    let mut listener = listener.lock().await;
                    let token = loop {
                        match listener.accept().await {
                            Ok(token) => {
                                break token;
                            },
                            Err(err) => {
                                log::warn!("accept error: {:?}", err);
                                continue;
                            }
                        }
                    };
                    drop(listener);

                    let _ = picoserve::Server::new(app, &CONFIG, &mut http_buffer)
                        .with_graceful_shutdown(
                            server_state.get_and(ServerState::is_shutdown),
                            shutdown_timeout,
                        )
                        .accept_and_serve(
                            task_id,
                            stack,
                            token,
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
