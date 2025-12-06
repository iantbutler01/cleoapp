use log::LevelFilter;
use oslog::OsLogger;
use std::sync::OnceLock;

static LOGGER_INIT: OnceLock<()> = OnceLock::new();

pub fn init() {
    LOGGER_INIT.get_or_init(|| {
        OsLogger::new("com.cleo.cleo")
            .level_filter(LevelFilter::Info)
            .init()
            .expect("failed to initialize unified logging");
    });
}
