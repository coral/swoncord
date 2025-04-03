use crossbeam::channel::bounded;

extern crate log;
extern crate pretty_env_logger;

use log::{error, info};
use swinsian::State;

mod app;
mod discord;
mod error;
mod swinsian;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();
    info!("Starting Swinsian Rich Presence");
    let (s, r) = bounded(100);

    let mut dc = discord::Discord::new()?;

    std::thread::spawn(move || {
        loop {
            let (state, track_info) = match r.recv() {
                Ok((state, track_info)) => (state, track_info),
                Err(e) => {
                    panic!("Error receiving from channel: {}", e);
                }
            };

            match state {
                State::Playing => {
                    if let Err(e) = dc.update(track_info) {
                        error!("Error updating Discord status: {:?}", e);
                    }
                }
                State::Stopped | State::Paused | State::Unknown => {
                    if let Err(e) = dc.clear() {
                        error!("Error clearing Discord status: {:?}", e);
                    }
                }
            }
        }
    });

    let mut application = app::Wrapper::new(s).unwrap();
    application.configure().unwrap();
    application.run();
    Ok(())
}
