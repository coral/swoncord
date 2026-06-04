//! Helpers for pulling Rust values out of Objective-C objects.

use crate::track::TrackInfo;
use objc2_foundation::{NSDictionary, NSNumber, NSString};

/// Looks up `key` in a notification's `userInfo` dictionary and returns it as a
/// `String`, or `None` if the key is missing or isn't a string.
fn get_string(user_info: &NSDictionary, key: &str) -> Option<String> {
    let value = user_info.objectForKey(&NSString::from_str(key))?;
    Some(value.downcast_ref::<NSString>()?.to_string())
}

/// Looks up `key` and returns it as an `f64`, accepting either an `NSNumber` or
/// a numeric `NSString`.
fn get_f64(user_info: &NSDictionary, key: &str) -> Option<f64> {
    let value = user_info.objectForKey(&NSString::from_str(key))?;
    if let Some(n) = value.downcast_ref::<NSNumber>() {
        return Some(n.doubleValue());
    }
    value.downcast_ref::<NSString>()?.to_string().parse().ok()
}

/// Builds a [`TrackInfo`] from a Swinsian notification's `userInfo` dictionary.
///
/// Both `currentTime` (playback position) and `length` (track duration) are
/// present, in seconds — enough to render the full bounded progress bar straight
/// from a notification, without waiting for the AppleScript poll.
pub fn track_info_from_user_info(user_info: &NSDictionary) -> TrackInfo {
    TrackInfo {
        artist: get_string(user_info, "artist").unwrap_or_default(),
        album: get_string(user_info, "album").unwrap_or_default(),
        title: get_string(user_info, "title").unwrap_or_default(),
        file_type: get_string(user_info, "fileType"),
        position: get_f64(user_info, "currentTime"),
        duration: get_f64(user_info, "length"),
    }
}

/// Renders a dictionary's full contents via Foundation's `-description`, for the
/// developer notification dump.
pub fn describe_dictionary(user_info: &NSDictionary) -> String {
    user_info
        .description()
        .downcast::<NSString>()
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "<unprintable userInfo>".to_string())
}
