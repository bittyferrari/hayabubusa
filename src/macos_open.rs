//! macOS: receive files opened from Finder ("Open With…", double-click once Hayabubusa is the
//! default viewer, dropping onto the Dock icon).
//!
//! Finder does not put the path on the command line; AppKit hands it to the application
//! delegate as `application:openURLs:`. winit's delegate does not implement that method, so we
//! add it to the delegate's class at runtime. This is done from a
//! `NSApplicationWillFinishLaunchingNotification` observer: by then winit has installed its
//! delegate, and AppKit has not yet dispatched the "open documents" event of the launch.
//!
//! Paths are delivered as `resident::Msg::Open`, the same message the Windows single-instance
//! hand-over uses, so `App` treats both the same way.

use crate::resident::Msg;
use block2::RcBlock;
use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSApplicationWillFinishLaunchingNotification};
use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSURL};
use std::ptr::NonNull;
use std::sync::OnceLock;

struct State {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

static STATE: OnceLock<State> = OnceLock::new();

/// Call once from `main`, before the event loop starts.
pub fn install(ctx: egui::Context) -> Receiver<Msg> {
    let (tx, rx) = unbounded();
    if STATE.set(State { tx, ctx }).is_err() {
        return crossbeam_channel::never();
    }
    let block = RcBlock::new(|_: NonNull<NSNotification>| add_open_handler());
    // SAFETY: valid notification name, no object filter, block runs on the posting (main) thread.
    unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let token = center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
            None,
            &block,
        );
        // Keep observing for the whole process lifetime.
        std::mem::forget(token);
    }
    rx
}

fn add_open_handler() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return;
    };
    let obj: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
    let cls: &AnyClass = obj.class();
    let imp: unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, &NSArray<NSURL>) =
        open_urls;
    // SAFETY: the type encoding matches `open_urls` (void, self, _cmd, id, id).
    // `class_addMethod` leaves an existing implementation untouched.
    unsafe {
        objc2::ffi::class_addMethod(
            cls as *const AnyClass as *mut AnyClass,
            sel!(application:openURLs:),
            std::mem::transmute::<_, Imp>(imp),
            c"v@:@@".as_ptr(),
        );
    }
}

/// `-[delegate application:(NSApplication*)app openURLs:(NSArray<NSURL*>*)urls]`
unsafe extern "C-unwind" fn open_urls(
    _this: &AnyObject,
    _cmd: Sel,
    _app: &AnyObject,
    urls: &NSArray<NSURL>,
) {
    let Some(st) = STATE.get() else {
        return;
    };
    // The viewer shows one image and browses its folder: open the first local file.
    if let Some(p) = urls.iter().find_map(|u| u.to_file_path()) {
        let _ = st.tx.send(Msg::Open(Some(p)));
        st.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        st.ctx.request_repaint();
    }
}
