//! Right Ctrl on its own as a dictation trigger, on Windows.
//!
//! Windows' shortcut API (`RegisterHotKey`) needs a key that isn't a
//! modifier, so right Ctrl on its own can't be registered. We watch the
//! keyboard with a low-level hook instead and report right Ctrl going down
//! and up. Right Ctrl is also half of shortcuts like Ctrl+C, so a key
//! pressed while it's held is reported as [`KeyEvent::Combo`]; the dictation
//! controller then cancels instead of starting to listen.
//!
//! The hook passes every key through unchanged and ignores simulated keys
//! (Quill's own paste among them). It needs no permission.

#![allow(unsafe_code)]

use std::cell::{Cell, OnceCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_RCONTROL;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, SetWindowsHookExW,
    WH_KEYBOARD_LL,
};

/// What right Ctrl did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Down,
    Up,
    /// Another key was pressed while right Ctrl was held: a shortcut like
    /// Ctrl+C, not dictation.
    Combo,
}

#[derive(Debug, thiserror::Error)]
#[error("couldn't start watching the right Ctrl key: {0}")]
pub struct RightCtrlError(String);

/// A running right-Ctrl watcher. Events are delivered only while active.
pub struct RightCtrl {
    active: Arc<AtomicBool>,
}

impl RightCtrl {
    /// Starts watching on its own thread and returns once the hook is in
    /// place. `on_event` runs on that thread: keep it quick (Windows drops
    /// a hook that's slow to answer), e.g. send on a channel.
    pub fn watch(on_event: impl Fn(KeyEvent) + Send + 'static) -> Result<Self, RightCtrlError> {
        let active = Arc::new(AtomicBool::new(true));
        let thread_active = Arc::clone(&active);
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("quill-right-ctrl".into())
            .spawn(move || run(Box::new(on_event), thread_active, &ready_tx))
            .map_err(|e| RightCtrlError(e.to_string()))?;
        ready_rx
            .recv()
            .map_err(|_| RightCtrlError("the watcher stopped".into()))??;
        Ok(Self { active })
    }

    /// Turns delivery on or off, e.g. when the user picks another shortcut.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::SeqCst);
    }
}

/// State the hook reads. Windows calls a low-level hook on the thread that
/// set it, so it lives in that thread's storage and `Cell`s are enough.
struct HookState {
    on_event: Box<dyn Fn(KeyEvent) + Send>,
    active: Arc<AtomicBool>,
    down: Cell<bool>,
    combo: Cell<bool>,
}

thread_local! {
    static STATE: OnceCell<HookState> = const { OnceCell::new() };
}

fn run(
    on_event: Box<dyn Fn(KeyEvent) + Send>,
    active: Arc<AtomicBool>,
    ready: &mpsc::Sender<Result<(), RightCtrlError>>,
) {
    STATE.with(|state| {
        let _ = state.set(HookState {
            on_event,
            active,
            down: Cell::new(false),
            combo: Cell::new(false),
        });
    });
    // SAFETY: `hook` matches HOOKPROC, and the module handle is this
    // process's own, valid for as long as it runs.
    let hooked = unsafe {
        GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(HINSTANCE::from(module)), 0)
        })
    };
    if let Err(e) = hooked {
        let _ = ready.send(Err(RightCtrlError(e.to_string())));
        return;
    }
    let _ = ready.send(Ok(()));
    tracing::info!("watching the right Ctrl key");
    // The hook is called from this thread's message loop. GetMessageW
    // returns 0 on WM_QUIT and -1 on error.
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid MSG for GetMessageW to fill.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
}

/// Called by Windows for each key, on the watcher thread.
unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A negative code means "pass it on without looking".
    if code >= 0 {
        // SAFETY: for a keyboard event, `lparam` points to a
        // KBDLLHOOKSTRUCT that's valid for the duration of the call.
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        STATE.with(|state| {
            if let Some(state) = state.get() {
                state.on_key(key);
            }
        });
    }
    // SAFETY: passing the event on unchanged, as every hook must.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

impl HookState {
    fn on_key(&self, key: &KBDLLHOOKSTRUCT) {
        if key.flags.contains(LLKHF_INJECTED) {
            return;
        }
        if !self.active.load(Ordering::Relaxed) {
            self.down.set(false);
            return;
        }
        let up = key.flags.contains(LLKHF_UP);
        if key.vkCode == u32::from(VK_RCONTROL.0) {
            if up {
                if self.down.replace(false) {
                    (self.on_event)(KeyEvent::Up);
                }
            } else if !self.down.replace(true) {
                // Not key repeat: a new press.
                self.combo.set(false);
                (self.on_event)(KeyEvent::Down);
            }
        } else if !up && self.down.get() && !self.combo.get() {
            self.combo.set(true);
            (self.on_event)(KeyEvent::Combo);
        }
    }
}
