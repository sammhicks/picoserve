#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cyw43::Control;
use embassy_rp::{
    gpio::{Level, Output},
    pio::Pio,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use panic_persist as _;
use picoserve::{
    AppBuilder, AppRouter, make_static,
    response::DebugValue,
    routing::{PathRouter, get, parse_path_segment},
};
use rand::Rng;

use picoserve::extract::State;

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

type ControlMutex = Mutex<CriticalSectionRawMutex, Control<'static>>;

#[derive(Clone, Copy)]
struct SharedControl(&'static ControlMutex);

struct AppState {
    shared_control: SharedControl,
}

impl picoserve::extract::FromRef<AppState> for SharedControl {
    fn from_ref(state: &AppState) -> Self {
        state.shared_control
    }
}

struct AppProps {
    state: AppState,
}

impl AppBuilder for AppProps {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        let Self { state } = self;

        picoserve::Router::from_service(
            const {
                use picoserve::response::File;

                picoserve::response::Directory {
                    files: &[
                        ("", File::html(include_str!("index.html"))),
                        ("index.css", File::css(include_str!("index.css"))),
                        ("index.js", File::javascript(include_str!("index.js"))),
                    ],
                    sub_directories: &[],
                }
            },
        )
        .route(
            ("/set_led", parse_path_segment()),
            get(
                |led_is_on, State(SharedControl(control)): State<SharedControl>| async move {
                    log::info!("Setting led to {}", if led_is_on { "ON" } else { "OFF" });
                    control.lock().await.gpio_set(0, led_is_on).await;
                    DebugValue(led_is_on)
                },
            ),
        )
        .with_state(state)
    }
}

static CONFIG: picoserve::Config = picoserve::Config::const_default().keep_connection_alive();

const WEB_TASK_POOL_SIZE: usize = 8;

#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
async fn web_task(
    task_id: usize,
    stack: embassy_net::Stack<'static>,
    token: embassy_net::tcp::AcceptToken,
    app: &'static AppRouter<AppProps>,
) {
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    let _ = picoserve::Server::new(app, &CONFIG, &mut http_buffer)
        .accept_and_serve(task_id, stack, token, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await;
}

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

    spawner.spawn(net_task(runner).unwrap());

    control
        .start_ap_wpa2(
            example_secrets::WIFI_SSID,
            example_secrets::WIFI_PASSWORD,
            8,
        )
        .await;

    spawner.spawn(example_utils::dhcp::dhcp_task(example_utils::ADDRESS, stack).unwrap());

    let shared_control = SharedControl(
        make_static!(Mutex<CriticalSectionRawMutex, Control<'static>>, Mutex::new(control)),
    );

    let app = make_static!(
        AppRouter<AppProps>,
        AppProps {
            state: AppState { shared_control }
        }
        .build_app()
    );

    log::info!("{}", example_utils::WELCOME_MESSAGE);
    const PORT: u16 = 80;
    let mut listener = embassy_net::tcp::TcpListener::new(stack).unwrap();
    listener.listen(PORT).unwrap();
    log::info!("Listening on TCP:{}", PORT);

    for conn_id in 0.. {
        let token = match listener.accept().await {
            Ok(token) => token,
            Err(err) => {
                log::warn!("accept error: {:?}", err);
                continue;
            }
        };
        match web_task(conn_id, stack, token, app) {
            Ok(spawn_token) => spawner.spawn(spawn_token),
            Err(_) => log::warn!("conn {}: no free socket, dropping the connection attempt", conn_id),
        }
    }
}
