pub use embassy_usb_logger::ReceiverHandler;

pub struct CommandHandler {
    position: embassy_sync::mutex::Mutex<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        usize,
    >,
}

impl CommandHandler {
    fn push_bytes(position: &mut usize, data: &[u8]) {
        let reset_command = b"elf2uf2-term\r\n";

        for &b in data {
            if b == reset_command[*position] {
                *position += 1;

                if *position == reset_command.len() {
                    embassy_rp::rom_data::reset_to_usb_boot(0, 0);
                }
            } else {
                *position = 0;
            }
        }
    }
}

impl ReceiverHandler for CommandHandler {
    fn new() -> Self {
        Self { position: 0.into() }
    }

    async fn handle_data(&self, data: &[u8]) {
        Self::push_bytes(&mut *self.position.lock().await, data);
    }
}
