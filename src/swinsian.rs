#![allow(unexpected_cfgs)]

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::{msg_send, sel, sel_impl};

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct TrackInfo {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<String>,
    pub length: f64,
    pub current_time: f64,
    pub track_uuid: String,
    pub art_path: Option<String>,
    pub thumbnail_path: Option<String>,
    pub file_path: String,
    pub file_type: Option<String>,
    pub bitrate: Option<String>,
    pub grouping: Option<String>,
    pub conductor: Option<String>,
    pub comment: Option<String>,
}

impl From<id> for TrackInfo {
    fn from(value: id) -> Self {
        extract_track_info(value)
    }
}

#[derive(Debug)]
pub enum State {
    Playing,
    Stopped,
    Paused,
    Unknown,
}

impl From<&str> for State {
    fn from(value: &str) -> Self {
        match value {
            "com.swinsian.Swinsian-Track-Playing" => State::Playing,
            "com.swinsian.Swinsian-Track-Stopped" => State::Stopped,
            "com.swinsian.Swinsian-Track-Paused" => State::Paused,
            _ => State::Unknown,
        }
    }
}
// Trait for extracting values from Objective-C objects
trait FromObjcObject {
    unsafe fn from_objc(obj: id) -> Option<Self>
    where
        Self: Sized;
}

// Implement for String
impl FromObjcObject for String {
    unsafe fn from_objc(obj: id) -> Option<Self> {
        unsafe {
            if obj != nil {
                let str_ptr = NSString::UTF8String(obj);
                Some(
                    std::ffi::CStr::from_ptr(str_ptr)
                        .to_string_lossy()
                        .to_string(),
                )
            } else {
                None
            }
        }
    }
}

// Implement for u32
impl FromObjcObject for u32 {
    unsafe fn from_objc(obj: id) -> Option<Self> {
        if obj != nil {
            Some(msg_send![obj, unsignedIntValue])
        } else {
            None
        }
    }
}

// Implement for f64
impl FromObjcObject for f64 {
    unsafe fn from_objc(obj: id) -> Option<Self> {
        if obj != nil {
            Some(msg_send![obj, doubleValue])
        } else {
            None
        }
    }
}

// Generic function to get values from dictionary
fn get<T: FromObjcObject>(user_info: id, key: &str) -> Option<T> {
    unsafe {
        let ns_key = NSString::alloc(nil).init_str(key);
        let value: id = msg_send![user_info, objectForKey:ns_key];
        T::from_objc(value)
    }
}

fn extract_track_info(user_info: id) -> TrackInfo {
    TrackInfo {
        artist: get(user_info, "artist").unwrap_or_default(),
        album: get(user_info, "album").unwrap_or_default(),
        title: get(user_info, "title").unwrap_or_default(),
        genre: get(user_info, "genre"),
        composer: get(user_info, "composer"),
        track_number: get(user_info, "trackNumber"),
        disc_number: get(user_info, "discNumber"),
        year: get(user_info, "year"),
        length: get(user_info, "length").unwrap_or(0.0),
        current_time: get(user_info, "currentTime").unwrap_or(0.0),
        track_uuid: get(user_info, "track_uuid").unwrap_or_default(),
        art_path: get(user_info, "artPath"),
        thumbnail_path: get(user_info, "thumbnailPath"),
        file_path: get(user_info, "filePath").unwrap_or_default(),
        file_type: get(user_info, "fileType"),
        bitrate: get(user_info, "bitrate"),
        grouping: get(user_info, "grouping"),
        conductor: get(user_info, "conductor"),
        comment: get(user_info, "comment"),
    }
}
