use crossbeam::channel::bounded;
use log::info;
use objc2::MainThreadMarker;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
    // Default to showing our own info/warn/error so failures aren't silent;
    // RUST_LOG still overrides (e.g. `RUST_LOG=debug`).
    pretty_env_logger::formatted_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "swoncord=info".into()))
        .init();
    info!("Starting Swinsian Rich Presence");

    let mtm = MainThreadMarker::new().expect("main must run on the main thread");
    let (tx, rx) = bounded(100);

    let enabled = Arc::new(AtomicBool::new(true));
    let (wake_tx, wake_rx) = bounded::<()>(1);

    // background thread for discord
    let _discord = discord::Discord::new(rx, enabled.clone(), wake_rx)?;

    // osx menu bar on main thread
    let mut app = macos::Wrapper::new(mtm, enabled, wake_tx)?;
    app.configure();

    // shared communication
    NotificationSource.start(tx.clone());
    AppleScriptSource.start(tx.clone());
    WorkspaceWatcher.start(tx);

    app.run();
    Ok(())
}
