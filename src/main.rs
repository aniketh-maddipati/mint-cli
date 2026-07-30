mod agents;
mod app;
mod config;
mod logging;
mod ui;

fn main() {
    let _guard = logging::init();
    let _config = config::Config::load_or_default();
}
