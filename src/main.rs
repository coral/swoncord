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
use source::applescript::AppleScriptSource;
use source::notification::NotificationSource;
use source::workspace::WorkspaceWatcher;

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

    // All three sources share the channel: notifications (live play/pause/track),
    // the AppleScript poll (startup + progress + quit backstop), and the
    // workspace watcher (instant quit). They register on the main run loop.
    NotificationSource.start(tx.clone());
    AppleScriptSource.start(tx.clone());
    WorkspaceWatcher.start(tx);

    app.run();
    Ok(())
}
