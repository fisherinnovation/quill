//! The fn (🌐 Globe) key as a dictation trigger, on macOS.
//!
//! fn isn't a key the system shortcut APIs can register: macOS reports it
//! only as a modifier flag. So we watch keyboard events with a Core
//! Graphics event tap and report fn going down and up. Apple keyboards also
//! use fn as a modifier (fn+Delete, fn+arrows, fn+F-keys), so a key pressed
//! while fn is held is reported as [`FnEvent::Combo`]; the dictation
//! controller then cancels instead of starting to listen.
//!
//! The tap passes every event through unchanged. Creating it needs the
//! Accessibility permission, which dictation already asks for to paste.

#![allow(unsafe_code)]

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

/// What the fn key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnEvent {
    Down,
    Up,
    /// Another key was pressed while fn was held: a shortcut like fn+Delete,
    /// not dictation.
    Combo,
}

#[derive(Debug, thiserror::Error)]
pub enum FnKeyError {
    #[error("Quill needs Accessibility access to use the fn key")]
    NotAllowed,
    #[error("couldn't start watching the fn key: {0}")]
    Thread(String),
}

/// A running fn-key watcher. Events are delivered only while active.
pub struct FnKey {
    active: Arc<AtomicBool>,
}

impl FnKey {
    /// Starts watching on its own thread and returns once the event tap
    /// exists. `on_event` runs on that thread: keep it quick (the tap sits
    /// in the path of every keystroke), e.g. send on a channel.
    pub fn watch(on_event: impl Fn(FnEvent) + Send + 'static) -> Result<Self, FnKeyError> {
        let active = Arc::new(AtomicBool::new(true));
        let thread_active = Arc::clone(&active);
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("quill-fn-key".into())
            .spawn(move || run(Box::new(on_event), thread_active, &ready_tx))
            .map_err(|e| FnKeyError::Thread(e.to_string()))?;
        ready_rx
            .recv()
            .map_err(|_| FnKeyError::Thread("the watcher stopped".into()))??;
        Ok(Self { active })
    }

    /// Turns delivery on or off, e.g. when the user picks another shortcut.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::SeqCst);
    }
}

/// State the tap callback reads. Lives for the rest of the process (the
/// callback can't be unregistered safely while the run loop runs), and is
/// only touched on the watcher thread, so `Cell`s are enough.
struct TapState {
    on_event: Box<dyn Fn(FnEvent) + Send>,
    active: Arc<AtomicBool>,
    fn_down: Cell<bool>,
    combo: Cell<bool>,
    port: Cell<Option<NonNull<CFMachPort>>>,
}

fn run(
    on_event: Box<dyn Fn(FnEvent) + Send>,
    active: Arc<AtomicBool>,
    ready: &mpsc::Sender<Result<(), FnKeyError>>,
) {
    let state: &'static TapState = Box::leak(Box::new(TapState {
        on_event,
        active,
        fn_down: Cell::new(false),
        combo: Cell::new(false),
        port: Cell::new(None),
    }));
    let mask = (1_u64 << CGEventType::FlagsChanged.0) | (1_u64 << CGEventType::KeyDown.0);
    // SAFETY: `callback` matches CGEventTapCallBack, and `user_info` points
    // to `state`, which is leaked and so valid for as long as the tap can
    // call back.
    let port: Option<CFRetained<CFMachPort>> = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            Some(callback),
            std::ptr::from_ref(state).cast_mut().cast::<c_void>(),
        )
    };
    let Some(port) = port else {
        // macOS returns no tap when the app isn't allowed to watch input.
        let _ = ready.send(Err(FnKeyError::NotAllowed));
        return;
    };
    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
        let _ = ready.send(Err(FnKeyError::Thread("no run loop source".into())));
        return;
    };
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = ready.send(Err(FnKeyError::Thread("no run loop".into())));
        return;
    };
    // SAFETY: reading an immutable CoreFoundation constant.
    let common = unsafe { kCFRunLoopCommonModes };
    run_loop.add_source(Some(&source), common);
    CGEvent::tap_enable(&port, true);
    state.port.set(Some(NonNull::from(&*port)));
    let _ = ready.send(Ok(()));
    tracing::info!("watching the fn key");
    CFRunLoop::run();
    // Only reached if the run loop is stopped; keep the port alive till then.
    drop(port);
}

/// Called by macOS for each keyboard event, on the watcher thread.
unsafe extern "C-unwind" fn callback(
    _proxy: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `user_info` is the leaked `TapState` passed to tap_create,
    // valid for the rest of the process and only used on this thread.
    let state = unsafe { &*user_info.cast::<TapState>() };
    let pass = event.as_ptr();

    // macOS turns a tap off if it's slow, or on some user input; turn it
    // back on.
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(port) = state.port.get() {
            // SAFETY: the port is retained by `run` for as long as the run
            // loop (and so this callback) runs.
            CGEvent::tap_enable(unsafe { port.as_ref() }, true);
        }
        return pass;
    }
    if !state.active.load(Ordering::Relaxed) {
        state.fn_down.set(false);
        return pass;
    }
    // SAFETY: macOS passes a valid event for the duration of the callback.
    let event = unsafe { event.as_ref() };
    if kind == CGEventType::FlagsChanged {
        // fn going down or up shows as its flag changing. Other modifiers
        // pressed while fn is held keep the flag set.
        let down = CGEvent::flags(Some(event)).contains(CGEventFlags::MaskSecondaryFn);
        if down != state.fn_down.get() {
            state.fn_down.set(down);
            if down {
                state.combo.set(false);
                (state.on_event)(FnEvent::Down);
            } else {
                (state.on_event)(FnEvent::Up);
            }
        } else if down && !state.combo.get() {
            state.combo.set(true);
            (state.on_event)(FnEvent::Combo);
        }
    } else if kind == CGEventType::KeyDown && state.fn_down.get() && !state.combo.get() {
        state.combo.set(true);
        (state.on_event)(FnEvent::Combo);
    }
    pass
}

/// What macOS itself does when fn / 🌐 is pressed on its own (Keyboard
/// settings → "Press 🌐 key to"). Anything but [`GlobeAction::Nothing`]
/// also happens when fn triggers dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobeAction {
    Nothing,
    ChangeInputSource,
    ShowEmoji,
    StartDictation,
    /// Never changed; macOS's default, which varies by keyboard.
    Default,
}

/// Reads the "Press 🌐 key to" setting.
pub fn globe_action() -> GlobeAction {
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.HIToolbox", "AppleFnUsageType"])
        .output();
    match output {
        Ok(out) if out.status.success() => match String::from_utf8_lossy(&out.stdout).trim() {
            "0" => GlobeAction::Nothing,
            "1" => GlobeAction::ChangeInputSource,
            "2" => GlobeAction::ShowEmoji,
            "3" => GlobeAction::StartDictation,
            _ => GlobeAction::Default,
        },
        _ => GlobeAction::Default,
    }
}
