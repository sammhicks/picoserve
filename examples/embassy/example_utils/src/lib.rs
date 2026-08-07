#![no_std]
#![feature(impl_trait_in_assoc_type)]

#[macro_use]
mod logging;

pub mod dhcp;

#[cfg(feature = "log")]
pub mod log;

pub const ADDRESS: core::net::Ipv4Addr = core::net::Ipv4Addr::new(10, 0, 0, 1);

pub const WELCOME_MESSAGE: &str = include_str!("welcome.txt");
