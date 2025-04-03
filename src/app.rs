#![allow(unexpected_cfgs)]

use crate::error::Error;
use crate::swinsian::{State, TrackInfo};
use cocoa::{
    appkit::{
        NSApp, NSApplication, NSApplicationActivateIgnoringOtherApps, NSImage, NSMenu, NSMenuItem,
        NSRunningApplication, NSStatusBar, NSStatusItem, NSWindow
    },
    base::{YES, id, nil},
    foundation::{NSAutoreleasePool, NSString, NSSize, NSBundle},
};
use block::ConcreteBlock;
use objc::{class, msg_send, sel, sel_impl};
use crossbeam::channel::Sender;
pub struct Wrapper {
    menu: *mut objc::runtime::Object,
    notifciation_center: *mut objc::runtime::Object,
    _pool: *mut objc::runtime::Object,
    messages: Sender<(State, TrackInfo)>,
}

impl Wrapper {
    pub fn new(s: Sender<(State, TrackInfo)>) -> Result<Self, Error> {
        let t = unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let center: id = msg_send![class!(NSDistributedNotificationCenter), defaultCenter];

            Wrapper {
                _pool: pool,
                menu: NSMenu::new(nil).autorelease(),
                notifciation_center: center,
                messages: s,
            }
        };
        Ok(t)
    }

    pub fn configure(&mut self){
        unsafe {
        self.add_quit_item("Quit");

        let track_playing =
        NSString::alloc(nil).init_str("com.swinsian.Swinsian-Track-Playing");
        let track_stopped =
        NSString::alloc(nil).init_str("com.swinsian.Swinsian-Track-Stopped");
        let track_paused =
        NSString::alloc(nil).init_str("com.swinsian.Swinsian-Track-Paused");

        let sender= self.messages.clone();

        let block = ConcreteBlock::new(move |notification: id| {
            let name: id = msg_send![notification, name];
            let name_str = NSString::UTF8String(name);
            let name_rust = std::ffi::CStr::from_ptr(name_str).to_string_lossy();

            let user_info: id = msg_send![notification, userInfo];

            if user_info != nil {
                let track_info: TrackInfo = TrackInfo::from(user_info);
                sender.send((State::from(name_rust.as_ref()), track_info)).unwrap();
            }
        });

        let block = block.copy();

        let _: () = msg_send![self.notifciation_center, 
        addObserverForName:track_playing
        object:nil
        queue:nil
        usingBlock:&*block];

        let _: () = msg_send![self.notifciation_center, 
                    addObserverForName:track_stopped
                    object:nil
                    queue:nil
                    usingBlock:&*block];

        let _: () = msg_send![self.notifciation_center, 
                    addObserverForName:track_paused
                    object:nil
                    queue:nil
                    usingBlock:&*block];


    }
    }

    pub fn add_quit_item(&mut self, label: &str) {
        unsafe {
            let no_key = NSString::alloc(nil).init_str("");
            let pref_item = NSString::alloc(nil).init_str(label);
            let pref_action = sel!(terminate:);
            let menuitem = NSMenuItem::alloc(nil).initWithTitle_action_keyEquivalent_(
                pref_item,
                pref_action,
                no_key,
            );

            self.menu.addItem_(menuitem);
        }
    }

    pub fn run(&mut self) {
        unsafe {
            let app = NSApp();
            app.activateIgnoringOtherApps_(YES);
            
            // Set activation policy to hide from dock
            let _: () = msg_send![app, setActivationPolicy: 1]; // 1 = NSApplicationActivationPolicyAccessory

            let item = NSStatusBar::systemStatusBar(nil).statusItemWithLength_(-1.0);
           
            // Load and set the icon
            let bundle: id = NSBundle::mainBundle();
            let icon_path = NSString::alloc(nil).init_str("icon.icns");
            let resource_path: id = msg_send![bundle, pathForResource:icon_path ofType:nil];
   
            let icon = NSImage::alloc(nil).initWithContentsOfFile_(resource_path);
            
            if icon != nil {
                let _: () = msg_send![icon, setSize: NSSize::new(18.0, 18.0)];
                let _: () = msg_send![item, setImage:icon];
            } else {
                let title = NSString::alloc(nil).init_str("Swoncord Dev");
                item.setTitle_(title);
            }
            
            
            item.setMenu_(self.menu);

            let current_app = NSRunningApplication::currentApplication(nil);
            current_app.activateWithOptions_(NSApplicationActivateIgnoringOtherApps);

            app.run();
        }
    }

    
}

impl Drop for Wrapper {
    fn drop(&mut self) {
        unsafe {
            self._pool.drain();
        }
    }
}
