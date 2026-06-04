use crossbeam::channel::bounded;
use log::info;
use objc2::MainThreadMarker;

mod consts;
mod discord;
mod error;
mod macos;
mod objc_util;
mod source;
mod track;

use source::TrackSource;
use source::notification::NotificationSource;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();
    info!("Starting Swinsian Rich Presence");

    let mtm = MainThreadMarker::new().expect("main must run on the main thread");
    let (tx, rx) = bounded(100);

    // Discord runs the only background thread, consuming track updates.
    let _discord = discord::Discord::new(rx)?;

    // The menu-bar app and track sources stay on the main thread.
    let mut app = macos::Wrapper::new(mtm)?;
    app.configure();

    // Push source today; the Phase-2 osakit pull source would be selected here.
    NotificationSource.start(tx);

    app.run();
    Ok(())
}
