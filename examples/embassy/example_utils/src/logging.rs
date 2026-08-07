macro_rules! log_warn {
    ($f:literal $(,$arg:expr)* $(,)?) => {
        {
            #[cfg(feature = "log")]
            log::warn!($f $(,$arg)*);

            #[cfg(feature = "defmt")]
            defmt::warn!($f $(,$arg)*);

            $(
                #[allow(clippy::let_underscore_untyped, reason = "Logging isn't enabled, discard the log info")]
                let _ = &$arg;
            )*
        }
    };
}
