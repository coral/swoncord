//! Push source: macOS distributed notifications posted by Swinsian.

use super::{TrackSource, TrackTx};
use crate::consts;
use crate::objc_util::track_info_from_user_info;
use crate::track::State;
use block2::RcBlock;
use log::error;
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
use std::ptr::NonNull;

/// Observes Swinsian's distributed notifications and forwards each as a track
/// update. Registered on the main run loop; the OS invokes the block there.
pub struct NotificationSource;

impl TrackSource for NotificationSource {
    fn start(self, tx: TrackTx) {
        let center = NSDistributedNotificationCenter::defaultCenter();

        let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
            // Safe: the run loop hands us a live notification for the call.
            let notification = unsafe { notification.as_ref() };
            let state = State::from(notification.name().to_string().as_str());

            if let Some(user_info) = notification.userInfo() {
                let update = (state, track_info_from_user_info(&user_info));
                if let Err(e) = tx.send(update) {
                    // Receiver gone (consumer thread died); log rather than
                    // panic inside an Objective-C callback.
                    error!("failed to forward track update: {e}");
                }
            }
        });

        for name in consts::SWINSIAN_NOTIFICATIONS {
            let ns_name = NSString::from_str(name);
            // Safe: nil object/queue is valid, and the block matches the
            // expected `Fn(NonNull<NSNotification>)` signature. The center
            // copies the block and retains the observer token internally.
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
