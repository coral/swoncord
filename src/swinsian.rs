#![allow(unexpected_cfgs)]

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::{msg_send, sel, sel_impl};

#[allow(dead_code)]
#[derive(Debug, Default, Hash, PartialEq, Clone)]
pub struct TrackInfo {
    pub artist: String,
    pub album: String,
    pub title: String,
    // pub genre: Option<String>,
    // pub composer: Option<String>,
    // pub track_number: Option<u32>,
    // pub disc_number: Option<u32>,
    // pub year: Option<String>,
    // //pub length: f64,
    // // pub current_time: f64,
    // pub track_uuid: String,
    // pub art_path: Option<String>,
    // pub thumbnail_path: Option<String>,
    // pub file_path: String,
    pub file_type: Option<String>,
    // pub bitrate: Option<String>,
    // pub grouping: Option<String>,
    // pub conductor: Option<String>,
    // pub comment: Option<String>,
}

impl From<id> for TrackInfo {
    fn from(value: id) -> Self {
        extract_track_info(value)
    }
}

#[derive(Debug, PartialEq)]
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

// look there's prob a better way to do this but cba
trait FromObjcObject {
    unsafe fn from_objc(obj: id) -> Option<Self>
    where
        Self: Sized;
}

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

impl FromObjcObject for u32 {
    unsafe fn from_objc(obj: id) -> Option<Self> {
        if obj != nil {
            // Try to get the value safely
            let result: u32 = msg_send![obj, unsignedIntValue];
            Some(result)
        } else {
            None
        }
    }
}

impl FromObjcObject for f64 {
    unsafe fn from_objc(obj: id) -> Option<Self> {
        if obj != nil {
            // Try to get the value safely
            let result: f64 = msg_send![obj, doubleValue];
            Some(result)
        } else {
            None
        }
    }
}

fn get<T: FromObjcObject>(user_info: id, key: &str) -> Option<T> {
    unsafe {
        let ns_key = NSString::alloc(nil).init_str(key);
        let value: id = msg_send![user_info, objectForKey:ns_key];
        if value == nil {
            return None;
        }
        let res = T::from_objc(value);
        res
    }
}

#[allow(dead_code)]
fn print_all_keys(user_info: id) {
    unsafe {
        let keys: id = msg_send![user_info, allKeys];
        let count: usize = msg_send![keys, count];

        for i in 0..count {
            let key: id = msg_send![keys, objectAtIndex:i];
            let _value: id = msg_send![user_info, objectForKey:key];

            let key_str = NSString::UTF8String(key);
            let key_rust = std::ffi::CStr::from_ptr(key_str).to_string_lossy();

            println!("Key: {}", key_rust);
        }
    }
}

fn extract_track_info(user_info: id) -> TrackInfo {
    //print_all_keys(user_info);

    TrackInfo {
        artist: get(user_info, "artist").unwrap_or_default(),
        album: get(user_info, "album").unwrap_or_default(),
        title: get(user_info, "title").unwrap_or_default(),
        // genre: get(user_info, "genre"),
        // composer: get(user_info, "composer"),
        // track_number: get(user_info, "trackNumber"),
        // disc_number: get(user_info, "discNumber"),
        // year: get(user_info, "year"),
        // // length: get(user_info, "length").unwrap_or(0.0),
        // //current_time: get(user_info, "currentTime").unwrap_or(0.0),
        // track_uuid: get(user_info, "track_uuid").unwrap_or_default(),
        // art_path: get(user_info, "artPath"),
        // thumbnail_path: get(user_info, "thumbnailPath"),
        // file_path: get(user_info, "filePath").unwrap_or_default(),
        file_type: get(user_info, "fileType"),
        // bitrate: get(user_info, "bitrate"),
        // grouping: get(user_info, "grouping"),
        // conductor: get(user_info, "conductor"),
        // comment: get(user_info, "comment"),
    }
}
