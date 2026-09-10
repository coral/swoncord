//! Instant quit detection via NSWorkspace.
//!
//! Swinsian posts no notification when it quits, so a stale presence would
//! linger until the next poll. This observes the system's app-termination
//! notification and, when Swinsian is the app that quit, immediately reports
//! `Stopped`. The 30s poll in [`super::applescript`] is the backstop for cases
//! this misses (e.g. force-kill).

use super::{TrackSource, TrackTx};
use crate::track::{State, TrackInfo, TrackUpdate, unix_now};
use block2::RcBlock;
use log::error;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidTerminateApplicationNotification,
};
use objc2_foundation::NSNotification;
use std::ptr::NonNull;

/// Swinsian's bundle identifier and display name, used to recognize it among
/// terminated apps.
const SWINSIAN_BUNDLE_ID: &str = "com.swinsian.Swinsian";
const SWINSIAN_APP_NAME: &str = "Swinsian";

/// Observes app terminations and reports `Stopped` when Swinsian quits.
pub struct WorkspaceWatcher;

impl TrackSource for WorkspaceWatcher {
    fn start(self, tx: TrackTx) {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();

        let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
            // Safe: the run loop hands us a live notification for the call.
            let notification = unsafe { notification.as_ref() };
            if !terminated_app_is_swinsian(notification) {
                return;
            }
            if let Err(e) = tx.send(TrackUpdate {
                state: State::Stopped,
                track: TrackInfo::default(),
                observed_at: unix_now(),
            }) {
                error!("failed to forward quit update: {e}");
            }
        });

        // Safe: nil object/queue is valid; the center copies the block and
        // retains the observer token. Reading the framework statics is the only
        // reason this needs `unsafe`.
        unsafe {
            let _observer = center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidTerminateApplicationNotification),
                None,
                None,
                &block,
            );
        }
    }
}

/// Returns whether the just-terminated app (carried in the notification's
/// `userInfo`) is Swinsian, matched by bundle identifier or display name.
fn terminated_app_is_swinsian(notification: &NSNotification) -> bool {
    let Some(user_info) = notification.userInfo() else {
        return false;
    };
    let key = unsafe { NSWorkspaceApplicationKey };
    let Some(app) = user_info.objectForKey(key) else {
        return false;
    };
    let Some(app) = app.downcast_ref::<NSRunningApplication>() else {
        return false;
    };

    let by_bundle = app
        .bundleIdentifier()
        .is_some_and(|id| id.to_string() == SWINSIAN_BUNDLE_ID);
    let by_name = app
        .localizedName()
        .is_some_and(|name| name.to_string() == SWINSIAN_APP_NAME);
    by_bundle || by_name
}
