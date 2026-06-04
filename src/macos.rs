//! macOS menu-bar app lifecycle (status item, menu, run loop).
//!
//! This owns the UI only; track observation lives in [`crate::source`]. Runs on
//! the main thread (enforced by [`MainThreadMarker`]) so macOS can dispatch
//! events to the app delegate.

use crate::consts;
use crate::error::Error;
use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSBundle, NSSize, NSString};

/// Owns the menu-bar status item and drives the `NSApplication` run loop.
pub struct Wrapper {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    menu: Retained<NSMenu>,
    /// Held for the app's lifetime so the status item isn't released.
    status_item: Option<Retained<NSStatusItem>>,
}

impl Wrapper {
    pub fn new(mtm: MainThreadMarker) -> Result<Self, Error> {
        Ok(Self {
            mtm,
            app: NSApplication::sharedApplication(mtm),
            menu: NSMenu::new(mtm),
            status_item: None,
        })
    }

    /// Populates the menu. Call before [`Wrapper::run`].
    pub fn configure(&mut self) {
        self.add_quit_item("Quit");
    }

    fn add_quit_item(&self, label: &str) {
        let title = NSString::from_str(label);
        let no_key = NSString::from_str("");
        // Safe: `terminate:` is a valid selector on NSApplication.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                self.mtm.alloc(),
                &title,
                Some(sel!(terminate:)),
                &no_key,
            )
        };
        self.menu.addItem(&item);
    }

    /// Installs the status item and enters the app run loop (blocks until quit).
    pub fn run(&mut self) {
        // Accessory: menu-bar only, hidden from the Dock and app switcher.
        self.app
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        self.app.activate();

        let item = self.setup_status_item();
        item.setMenu(Some(&self.menu));
        self.status_item = Some(item);

        self.app.run();
    }

    /// Creates the status-bar item and sets its icon (falling back to a text
    /// title if the icon can't be loaded, e.g. running outside the app bundle).
    fn setup_status_item(&self) -> Retained<NSStatusItem> {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(self.mtm) {
            match self.load_icon() {
                Some(icon) => button.setImage(Some(&icon)),
                None => button.setTitle(&NSString::from_str("Swoncord Dev")),
            }
        }
        item
    }

    /// Loads `icon.icns` from the app bundle, sized for the menu bar.
    fn load_icon(&self) -> Option<Retained<NSImage>> {
        let icon_name = NSString::from_str("icon.icns");
        let path = NSBundle::mainBundle().pathForResource_ofType(Some(&icon_name), None)?;
        let icon = NSImage::initWithContentsOfFile(NSImage::alloc(), &path)?;
        icon.setSize(NSSize {
            width: consts::ICON_SIZE,
            height: consts::ICON_SIZE,
        });
        Some(icon)
    }
}
