//! Helpers for pulling Rust values out of Objective-C objects.

use crate::track::TrackInfo;
use objc2_foundation::{NSDictionary, NSString};

/// Looks up `key` in a notification's `userInfo` dictionary and returns it as a
/// `String`, or `None` if the key is missing or isn't a string.
fn get_string(user_info: &NSDictionary, key: &str) -> Option<String> {
    let value = user_info.objectForKey(&NSString::from_str(key))?;
    Some(value.downcast_ref::<NSString>()?.to_string())
}

/// Builds a [`TrackInfo`] from a Swinsian notification's `userInfo` dictionary.
pub fn track_info_from_user_info(user_info: &NSDictionary) -> TrackInfo {
    TrackInfo {
        artist: get_string(user_info, "artist").unwrap_or_default(),
        album: get_string(user_info, "album").unwrap_or_default(),
        title: get_string(user_info, "title").unwrap_or_default(),
        file_type: get_string(user_info, "fileType"),
        position: None,
        duration: None,
    }
}
