mod config;
mod logging;

fn main() {
    let _guard = logging::init();
    let _config = config::Config::load_or_default();
}
