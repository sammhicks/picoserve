#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use embassy_rp::peripherals::USB;
use embassy_usb::{
    Builder, Config, UsbDevice,
    class::cdc_acm::{CdcAcmClass, State as AcmState},
    class::cdc_ncm::embassy_net::{Device, Runner, State as NetState},
    class::cdc_ncm::{CdcNcmClass, State as NcmState},
};

use panic_persist as _;
use picoserve::{AppBuilder, AppRouter, make_static, routing::get};
use rand::Rng;

// USB IRQs are handled by embassy_rp::usb directly
embassy_rp::bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
});

type MyDriver = embassy_rp::usb::Driver<'static, USB>;

const MTU: usize = 1514;

// This example uses with_class! instead of run! since we have a composite USB device
// with one NCM and one ACM interface
#[embassy_executor::task]
async fn logger_task(class: CdcAcmClass<'static, MyDriver>) {
    use example_utils::log::{CommandHandler, ReceiverHandler};

    embassy_usb_logger::with_class!(1024, log::LevelFilter::Info, class, CommandHandler).await
}

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, MyDriver>) -> ! {
    device.run().await
}

#[embassy_executor::task]
async fn usb_ncm_task(class: Runner<'static, MyDriver>) -> ! {
    class.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static>) -> ! {
    runner.run().await
}

struct AppProps;

impl AppBuilder for AppProps {
    type PathRouter = impl picoserve::routing::PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        picoserve::Router::new().route("/", get(|| async move { "Hello World" }))
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

    // Create the driver, from the HAL.
    let driver = embassy_rp::usb::Driver::new(p.USB, Irqs);

    // Logger will be created after the composite USB device has been built

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Picoserve");
    config.product = Some("USB-Ethernet example");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Create embassy-usb DeviceBuilder using the driver and config.
    let mut builder = Builder::new(
        driver,
        config,
        picoserve::make_static!([u8; 256], [0; _]),
        picoserve::make_static!([u8; 256], [0; _]),
        &mut [], // no msos descriptors
        picoserve::make_static!([u8; 128], [0; _]),
    );

    // Our MAC addr.
    let our_mac_addr = [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];
    // Host's MAC addr. This is the MAC the host "thinks" its USB-to-ethernet adapter has.
    let host_mac_addr = [0x88, 0x88, 0x88, 0x88, 0x88, 0x88];

    // Create classes on the builder
    // CDC NCM for the Ethernet emulation and the web server
    let ncm_class = CdcNcmClass::new(
        &mut builder,
        picoserve::make_static!(NcmState, NcmState::new()),
        host_mac_addr,
        64,
    );

    // CDC ACM for the logger via emulated serial
    let logger_class = CdcAcmClass::new(
        &mut builder,
        picoserve::make_static!(AcmState, AcmState::new()),
        64,
    );

    // Build the builder.
    let usb = builder.build();

    spawner.spawn(usb_task(usb).unwrap());
    spawner.spawn(logger_task(logger_class).unwrap());

    // Create the NCM net_device from the NCM class
    let (runner, net_device) = ncm_class.into_embassy_net_device::<4, 4>(
        picoserve::make_static!(NetState<4, 4>, NetState::new()),
        our_mac_addr,
        MTU
    );
    spawner.spawn(usb_ncm_task(runner).unwrap());

    // Init the network stack with static IPv4 and using the NCM device
    let (stack, runner) = embassy_net::Stack::new(
        make_static!(
            embassy_net::StackStorage,
            embassy_net::StackStorage::new()
        ),
        embassy_rp::clocks::RoscRng.random(),
    );

    let iface = stack.add_iface(make_static!(Device<'static>, net_device)).unwrap();
    iface.add_ip_addr(embassy_net::wire::IpCidr::new(example_utils::ADDRESS.into(), 24)).unwrap();
    iface.set_dhcpv4_server(Some(example_utils::dhcp_server_config()));

    spawner.spawn(net_task(runner).unwrap());

    // Start the web server and span its tasks
    let app = make_static!(AppRouter<AppProps>, AppProps.build_app());

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
