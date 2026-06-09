//! macOS menu-bar app lifecycle (status item, menu, run loop).
//!
//! This owns the UI only; track observation lives in [`crate::source`]. Runs on
//! the main thread (enforced by [`MainThreadMarker`]) so macOS can dispatch
//! events to the app delegate.

use crate::error::Error;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSControlStateValueOff, NSControlStateValueOn,
    NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use crossbeam::channel::Sender;
use objc2_foundation::{NSBundle, NSSize, NSString};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Menu-bar status icon size, in points.
const ICON_SIZE: f64 = 18.0;

/// Instance variables for [`ToggleTarget`].
struct ToggleIvars {
    /// Shared flag the Discord consumer reads to gate presence writes.
    enabled: Arc<AtomicBool>,
    /// Wakes the Discord consumer to clear/repopulate immediately on toggle.
    wake: Sender<()>,
    /// The menu item whose checkmark mirrors `enabled`.
    item: Retained<NSMenuItem>,
}

define_class!(
    // Objective-C target object for the "Enabled" menu item's action.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToggleIvars]
    struct ToggleTarget;

    impl ToggleTarget {
        #[unsafe(method(toggleEnabled:))]
        fn toggle_enabled(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars();
            let now_enabled = !ivars.enabled.load(Ordering::Relaxed);
            ivars.enabled.store(now_enabled, Ordering::Relaxed);
            ivars.item.setState(if now_enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            // Poke the consumer so it clears/repopulates without waiting for its
            // next poll window. A full channel already has a wake pending.
            let _ = ivars.wake.try_send(());
        }
    }
);

impl ToggleTarget {
    fn new(
        mtm: MainThreadMarker,
        enabled: Arc<AtomicBool>,
        wake: Sender<()>,
        item: Retained<NSMenuItem>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ToggleIvars {
            enabled,
            wake,
            item,
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Instance variables for [`AutostartTarget`].
struct AutostartIvars {
    /// The menu item whose checkmark mirrors the login-item registration.
    item: Retained<NSMenuItem>,
}

define_class!(
    // Objective-C target object for the "Autostart" menu item's action.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = AutostartIvars]
    struct AutostartTarget;

    impl AutostartTarget {
        #[unsafe(method(toggleAutostart:))]
        fn toggle_autostart(&self, _sender: Option<&AnyObject>) {
            // Register/unregister the main app as a login item. macOS 13+ via
            // SMAppService; only meaningful when running from the .app bundle.
            let service = unsafe { SMAppService::mainAppService() };
            let result = if autostart_enabled(&service) {
                unsafe { service.unregisterAndReturnError() }
            } else {
                unsafe { service.registerAndReturnError() }
            };
            if let Err(err) = result {
                log::warn!("autostart toggle failed: {err:?}");
            }
            // Re-read the authoritative status rather than assuming the toggle
            // took effect — registration can land in RequiresApproval, etc.
            set_check(&self.ivars().item, autostart_enabled(&service));
        }
    }
);

impl AutostartTarget {
    fn new(mtm: MainThreadMarker, item: Retained<NSMenuItem>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AutostartIvars { item });
        unsafe { msg_send![super(this), init] }
    }
}

/// Whether the main app is currently registered as a login item.
fn autostart_enabled(service: &SMAppService) -> bool {
    let status = unsafe { service.status() };
    status == SMAppServiceStatus::Enabled
}

/// Sets a menu item's checkmark from a boolean.
fn set_check(item: &NSMenuItem, on: bool) {
    item.setState(if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
}

/// Owns the menu-bar status item and drives the `NSApplication` run loop.
pub struct Wrapper {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    menu: Retained<NSMenu>,
    enabled: Arc<AtomicBool>,
    wake: Sender<()>,
    /// Held for the app's lifetime so the status item isn't released.
    status_item: Option<Retained<NSStatusItem>>,
    /// Held because `NSMenuItem::setTarget` is a weak reference.
    toggle_target: Option<Retained<ToggleTarget>>,
    /// Held because `NSMenuItem::setTarget` is a weak reference.
    autostart_target: Option<Retained<AutostartTarget>>,
}

impl Wrapper {
    pub fn new(
        mtm: MainThreadMarker,
        enabled: Arc<AtomicBool>,
        wake: Sender<()>,
    ) -> Result<Self, Error> {
        Ok(Self {
            mtm,
            app: NSApplication::sharedApplication(mtm),
            menu: NSMenu::new(mtm),
            enabled,
            wake,
            status_item: None,
            toggle_target: None,
            autostart_target: None,
        })
    }

    /// Populates the menu. Call before [`Wrapper::run`].
    pub fn configure(&mut self) {
        self.add_enabled_item();
        self.add_autostart_item();
        self.menu.addItem(&NSMenuItem::separatorItem(self.mtm));
        self.add_quit_item("Quit");
    }

    /// Adds the "Enabled" checkmark item, wired to toggle the shared flag.
    fn add_enabled_item(&mut self) {
        let title = NSString::from_str("Enabled");
        let no_key = NSString::from_str("");
        // Safe: `toggleEnabled:` is implemented on the target we set below.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                self.mtm.alloc(),
                &title,
                Some(sel!(toggleEnabled:)),
                &no_key,
            )
        };
        item.setState(if self.enabled.load(Ordering::Relaxed) {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });

        let target =
            ToggleTarget::new(self.mtm, self.enabled.clone(), self.wake.clone(), item.clone());
        // Safe: the target outlives the item (stored in `self.toggle_target`).
        unsafe {
            item.setTarget(Some(&target));
        }
        self.menu.addItem(&item);
        self.toggle_target = Some(target);
    }

    /// Adds the "Autostart" checkmark item, wired to register/unregister the
    /// app as a macOS login item via `SMAppService`.
    fn add_autostart_item(&mut self) {
        let title = NSString::from_str("Autostart");
        let no_key = NSString::from_str("");
        // Safe: `toggleAutostart:` is implemented on the target we set below.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                self.mtm.alloc(),
                &title,
                Some(sel!(toggleAutostart:)),
                &no_key,
            )
        };
        let service = unsafe { SMAppService::mainAppService() };
        set_check(&item, autostart_enabled(&service));

        let target = AutostartTarget::new(self.mtm, item.clone());
        // Safe: the target outlives the item (stored in `self.autostart_target`).
        unsafe {
            item.setTarget(Some(&target));
        }
        self.menu.addItem(&item);
        self.autostart_target = Some(target);
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
            width: ICON_SIZE,
            height: ICON_SIZE,
        });
        Some(icon)
    }
}
