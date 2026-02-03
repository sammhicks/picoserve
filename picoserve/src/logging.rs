#![allow(unused_macros)]

macro_rules! log_error {
    ($f:literal $(,$arg:expr)* $(,)?) => {
        {
            #[cfg(feature = "log")]
            log::error!($f $(,$arg)*);

            #[cfg(feature = "defmt")]
            defmt::error!($f $(,$arg)*);

            $(
                let _ = &$arg;
            )*
        }
    };
}

macro_rules! log_warn {
    ($f:literal $(,$arg:expr)* $(,)?) => {
        {
            #[cfg(feature = "log")]
            log::warn!($f $(,$arg)*);

            #[cfg(feature = "defmt")]
            defmt::warn!($f $(,$arg)*);

            $(
                let _ = &$arg;
            )*
        }
    };
}

macro_rules! log_info {
    ($f:literal $(,$arg:expr)* $(,)?) => {
        {
            #[cfg(feature = "log")]
            log::info!($f $(,$arg)*);

            #[cfg(feature = "defmt")]
            defmt::info!($f $(,$arg)*);

            $(
                let _ = &$arg;
            )*
        }
    };
}

#[cfg(feature = "defmt")]
pub use defmt::Debug2Format;

#[cfg(not(feature = "defmt"))]
#[allow(dead_code)]
pub struct Debug2Format<'a, T: core::fmt::Debug + ?Sized>(pub &'a T);

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Debug + ?Sized> core::fmt::Debug for Debug2Format<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.0, f)
    }
}

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Debug + ?Sized> core::fmt::Display for Debug2Format<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.0, f)
    }
}

#[cfg(feature = "defmt")]
pub use defmt::Display2Format;

#[cfg(not(feature = "defmt"))]
pub struct Display2Format<'a, T: core::fmt::Display + ?Sized>(pub &'a T);

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Display + ?Sized> core::fmt::Debug for Display2Format<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self.0, f)
    }
}

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Display + ?Sized> core::fmt::Display for Display2Format<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self.0, f)
    }
}

#[cfg(feature = "defmt")]
pub trait LogDisplay: core::fmt::Display + defmt::Format {}

#[cfg(feature = "defmt")]
impl<T: core::fmt::Display + defmt::Format> LogDisplay for T {}

#[cfg(not(feature = "defmt"))]
pub trait LogDisplay: core::fmt::Display {}

#[cfg(not(feature = "defmt"))]
impl<T: core::fmt::Display> LogDisplay for T {}
