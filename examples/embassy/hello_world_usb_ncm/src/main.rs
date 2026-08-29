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
use static_cell::StaticCell;

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
async fn usb_ncm_task(class: Runner<'static, MyDriver, MTU>) -> ! {
    class.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Device<'static, MTU>>) -> ! {
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
    app: &'static AppRouter<AppProps>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::Server::new(app, &CONFIG, &mut http_buffer)
        .listen_and_serve(task_id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
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
    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let mut builder = Builder::new(
        driver,
        config,
        &mut CONFIG_DESC.init([0; 256])[..],
        &mut BOS_DESC.init([0; 256])[..],
        &mut [], // no msos descriptors
        &mut CONTROL_BUF.init([0; 128])[..],
    );

    // Our MAC addr.
    let our_mac_addr = [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];
    // Host's MAC addr. This is the MAC the host "thinks" its USB-to-ethernet adapter has.
    let host_mac_addr = [0x88, 0x88, 0x88, 0x88, 0x88, 0x88];

    // Create classes on the builder
    // CDC NCM for the Ethernet emulation and the web server
    static STATE: StaticCell<NcmState> = StaticCell::new();
    let ncm_class = CdcNcmClass::new(&mut builder, STATE.init(NcmState::new()), host_mac_addr, 64);

    // CDC ACM for the logger via emulated serial
    static LOGGER_STATE: StaticCell<AcmState> = StaticCell::new();
    let logger_class = CdcAcmClass::new(&mut builder, LOGGER_STATE.init(AcmState::new()), 64);

    // Build the builder.
    let usb = builder.build();

    spawner.must_spawn(usb_task(usb));
    spawner.must_spawn(logger_task(logger_class));

    // Create the NCM net_device from the NCM class
    static NET_STATE: StaticCell<NetState<MTU, 4, 4>> = StaticCell::new();
    let (runner, net_device) = ncm_class
        .into_embassy_net_device::<MTU, 4, 4>(NET_STATE.init(NetState::new()), our_mac_addr);
    let _ = spawner.spawn(usb_ncm_task(runner));

    // Init the network stack with static IPv4 and using the NCM device
    let (stack, runner) = embassy_net::new(
        net_device,
        embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: embassy_net::Ipv4Cidr::new(example_utils::ADDRESS, 24),
            gateway: None,
            dns_servers: Default::default(),
        }),
        make_static!(
            embassy_net::StackResources<{ WEB_TASK_POOL_SIZE + example_utils::dhcp::SOCKET_COUNT }>,
            embassy_net::StackResources::new()
        ),
        embassy_rp::clocks::RoscRng.random(),
    );

    spawner.must_spawn(net_task(runner));

    spawner.must_spawn(example_utils::dhcp::dhcp_task(
        example_utils::ADDRESS,
        stack,
    ));

    // Start the web server and span its tasks
    let app = make_static!(AppRouter<AppProps>, AppProps.build_app());

    log::info!("{}", example_utils::WELCOME_MESSAGE);

    for task_id in 0..WEB_TASK_POOL_SIZE {
        spawner.must_spawn(web_task(task_id, stack, app));
    }
}
