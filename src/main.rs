use crossbeam::channel::bounded;

extern crate log;
extern crate pretty_env_logger;

use log::info;

mod app;
mod discord;
mod error;
mod swinsian;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();
    info!("Starting Swinsian Rich Presence");
    let (s, r) = bounded(100);

    let _dc = discord::Discord::new(r)?;

    let mut application = app::Wrapper::new(s).unwrap();
    application.configure().unwrap();
    application.run();
    Ok(())
}
