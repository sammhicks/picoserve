#![no_std]
#![feature(impl_trait_in_assoc_type)]

#[macro_use]
mod logging;

use core::net::Ipv4Addr;
use embassy_net::iface::dhcpv4_server::DhcpServerConfig;

pub fn dhcp_server_config() -> DhcpServerConfig {
    let o = ADDRESS.octets();
    let mut config = DhcpServerConfig::new(
        Ipv4Addr::new(o[0], o[1], o[2], 50),
        Ipv4Addr::new(o[0], o[1], o[2], 200),
    );
    config.lease_duration = embassy_time::Duration::from_secs(7200);
    config
}

#[cfg(feature = "log")]
pub mod log;

pub const ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

pub const WELCOME_MESSAGE: &str = include_str!("welcome.txt");
