use crate::device::uart::{write_line, write_str};


static LOGGER: UartLogger = UartLogger;
struct UartLogger;

impl log::Log for UartLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // Only log smoltcp TCP-related messages at trace level
        metadata.level() <= log::Level::Debug && metadata.target().contains("smoltcp")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            write_str("[SMOLTCP] ");
            write_str(&alloc::format!("{}", record.args()));
            write_line("");
        }
    }

    fn flush(&self) {}
}



/// Initialize the logger (call once at boot)
pub fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    // Disable smoltcp debug logging in production
    log::set_max_level(log::LevelFilter::Off);
}
