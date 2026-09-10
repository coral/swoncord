//! Push source: macOS distributed notifications posted by Swinsian.

use super::{TrackSource, TrackTx};
use crate::objc_util::{describe_dictionary, track_info_from_user_info};
use crate::track::{
    SWINSIAN_TRACK_PAUSED, SWINSIAN_TRACK_PLAYING, SWINSIAN_TRACK_STOPPED, State, TrackUpdate,
    unix_now,
};
use block2::RcBlock;
use log::error;
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
use std::ptr::NonNull;

/// The Swinsian notifications this source registers observers for.
const SWINSIAN_NOTIFICATIONS: [&str; 3] = [
    SWINSIAN_TRACK_PLAYING,
    SWINSIAN_TRACK_STOPPED,
    SWINSIAN_TRACK_PAUSED,
];

/// Observes Swinsian's distributed notifications and forwards each as a track
/// update. Registered on the main run loop; the OS invokes the block there.
pub struct NotificationSource;

impl TrackSource for NotificationSource {
    fn start(self, tx: TrackTx) {
        let center = NSDistributedNotificationCenter::defaultCenter();
        let dump = std::env::var_os("SWONCORD_DEBUG_NOTIFICATIONS").is_some();
        if dump {
            eprintln!("dumping nsnotification data");
        }

        let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
            let observed_at = unix_now();
            // Safe: the run loop hands us a live notification for the call.
            let notification = unsafe { notification.as_ref() };
            let name = notification.name().to_string();
            let user_info = notification.userInfo();

            if dump {
                match &user_info {
                    Some(ui) => eprintln!(
                        "[swoncord] notification {name}:\n{}",
                        describe_dictionary(ui)
                    ),
                    None => eprintln!("[swoncord] notification {name}: <no userInfo>"),
                }
            }

            if let Some(user_info) = user_info {
                let update = TrackUpdate {
                    state: State::from(name.as_str()),
                    track: track_info_from_user_info(&user_info),
                    observed_at,
                };
                if let Err(e) = tx.send(update) {
                    // Receiver gone (consumer thread died); log rather than
                    // panic inside an Objective-C callback.
                    error!("failed to forward track update: {e}");
                }
            }
        });

        for name in SWINSIAN_NOTIFICATIONS {
            let ns_name = NSString::from_str(name);
            unsafe {
                let _observer = center.addObserverForName_object_queue_usingBlock(
                    Some(&ns_name),
                    None,
                    None,
                    &block,
                );
            }
        }
    }
}
