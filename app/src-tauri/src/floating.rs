//! Floating windows that take clicks without bringing Quill forward: the
//! call prompt and live notes.
//!
//! On macOS, clicking any window of an app activates the app, which raises
//! all its windows: the main window would jump over the user's meeting.
//! Panels made with the "non-activating" style don't, but Tauri windows
//! can't be made into one (their class carries state that swapping it
//! would corrupt). So on macOS the window's web view moves into a panel
//! made here, and the Tauri window itself is never shown.
//!
//! On Windows, Tauri's `show()` activates the window even when it's
//! unfocusable: unfocusable only stops clicks from activating it. That took
//! the keyboard focus from the app being dictated into, so the paste landed
//! in Quill's indicator instead, often enough to notice. So on Windows
//! these windows, and the indicator, are shown and hidden without
//! activating them ([`show_unfocused`]).

use tauri::WebviewWindow;

/// Where a floating window goes, on the screen the mouse is on.
#[derive(Debug, Clone, Copy)]
pub enum Place {
    /// Centred, just above the Dock or taskbar.
    BottomCentre,
    /// At the right edge, just below the menu bar.
    RightTop,
    /// At the right edge, a third of the way down.
    RightThird,
}

/// Gap from the screen's usable edges.
const MARGIN: f64 = 12.0;

/// Shows `window` at `size` (logical points) in `place`.
pub fn show(window: &WebviewWindow, size: (f64, f64), place: Place) {
    #[cfg(target_os = "macos")]
    mac::show(window, size, place);
    #[cfg(not(target_os = "macos"))]
    other::show(window, size, place);
}

pub fn hide(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    mac::hide(window);
    #[cfg(not(target_os = "macos"))]
    let _ = hide_unfocused(window);
}

/// Shows `window` where it is without giving it the keyboard focus, for a
/// window that isn't [`adopt`]ed (the dictation indicator). Hide it with
/// [`hide_unfocused`], not `hide()`: on Windows, Tauri doesn't know it's
/// showing, so its `hide()` would do nothing.
pub fn show_unfocused(window: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    return win::show_window(window, true);
    #[cfg(not(windows))]
    window.show()
}

/// Hides a window shown with [`show_unfocused`].
pub fn hide_unfocused(window: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    return win::show_window(window, false);
    #[cfg(not(windows))]
    window.hide()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use tauri::WebviewWindow;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNOACTIVATE, ShowWindowAsync};

    /// Shows (without activating) or hides `window`. Async: the window
    /// belongs to the main thread, and a synchronous call from another one
    /// would wait for it. Tao queues its own `show()` the same way.
    pub fn show_window(window: &WebviewWindow, show: bool) -> tauri::Result<()> {
        // Tauri's HWND comes from a newer `windows` crate; it's the same handle.
        let hwnd = HWND(window.hwnd()?.0);
        let command = if show { SW_SHOWNOACTIVATE } else { SW_HIDE };
        // SAFETY: `hwnd` is the handle of a window that exists (Tauri just
        // returned it for `window`, which we borrow). ShowWindowAsync only
        // posts a message to the window's thread; it reads no memory of ours.
        if let Err(e) = unsafe { ShowWindowAsync(hwnd, command) }.ok() {
            tracing::debug!("couldn't show or hide a floating window: {e}");
        }
        Ok(())
    }
}

/// Prepares a just-built window: on macOS, moves its web view into a
/// non-activating panel. `private` keeps it out of screen sharing.
pub fn adopt(window: &WebviewWindow, private: bool) {
    #[cfg(target_os = "macos")]
    mac::adopt(window, private);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, private);
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSEvent, NSFloatingWindowLevel,
        NSPanel, NSScreen, NSView, NSWindowCollectionBehavior, NSWindowSharingType,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tauri::WebviewWindow;

    use super::{MARGIN, Place};

    // AppKit objects live on the main thread, so the panels do too.
    thread_local! {
        static PANELS: RefCell<HashMap<String, Retained<NSPanel>>> = RefCell::new(HashMap::new());
    }

    pub fn adopt(window: &WebviewWindow, private: bool) {
        let label = window.label().to_string();
        let result = window.with_webview(move |webview| {
            let Some(mtm) = MainThreadMarker::new() else {
                tracing::warn!("floating panel set up off the main thread");
                return;
            };
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 100.0)),
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            // Above other apps, on every desktop and over full-screen ones,
            // never the key window unless something needs typing.
            panel.setFloatingPanel(true);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setHidesOnDeactivate(false);
            panel.setLevel(NSFloatingWindowLevel);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            // The page draws its own card and shadow.
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(false);
            if private {
                panel.setSharingType(NSWindowSharingType::None);
            }
            // SAFETY: on macOS `inner` is the WKWebView (an NSView), alive
            // for as long as the Tauri window; retaining it keeps it so.
            let Some(view) = (unsafe { Retained::retain(webview.inner().cast::<NSView>()) }) else {
                tracing::warn!("no web view to put in the floating panel");
                return;
            };
            let Some(content) = panel.contentView() else {
                return;
            };
            view.removeFromSuperview();
            view.setFrame(content.bounds());
            view.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            content.addSubview(&view);
            PANELS.with(|panels| panels.borrow_mut().insert(label, panel));
        });
        if let Err(e) = result {
            tracing::warn!("couldn't set up a floating panel: {e}");
        }
    }

    pub fn show(window: &WebviewWindow, (width, height): (f64, f64), place: Place) {
        let label = window.label().to_string();
        let _ = window.run_on_main_thread(move || {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            PANELS.with(|panels| {
                let panels = panels.borrow();
                let Some(panel) = panels.get(&label) else {
                    return;
                };
                let Some(area) = screen_under_mouse(mtm).map(|s| s.visibleFrame()) else {
                    return;
                };
                // AppKit's y runs up from the bottom of the screen.
                let height = height.min(area.size.height - 2.0 * MARGIN);
                let right = area.origin.x + area.size.width - width - MARGIN;
                let top = area.origin.y + area.size.height;
                let origin = match place {
                    Place::BottomCentre => NSPoint::new(
                        area.origin.x + (area.size.width - width) / 2.0,
                        area.origin.y + MARGIN,
                    ),
                    Place::RightTop => NSPoint::new(right, top - height - MARGIN),
                    Place::RightThird => NSPoint::new(right, top - area.size.height / 3.0 - height),
                };
                panel.setFrame_display(NSRect::new(origin, NSSize::new(width, height)), true);
                // Shown without activating Quill.
                panel.orderFrontRegardless();
            });
        });
    }

    pub fn hide(window: &WebviewWindow) {
        let label = window.label().to_string();
        let _ = window.run_on_main_thread(move || {
            PANELS.with(|panels| {
                if let Some(panel) = panels.borrow().get(&label) {
                    panel.orderOut(None);
                }
            });
        });
    }

    fn screen_under_mouse(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        screens
            .iter()
            .find(|s| {
                let f = s.frame();
                mouse.x >= f.origin.x
                    && mouse.x < f.origin.x + f.size.width
                    && mouse.y >= f.origin.y
                    && mouse.y < f.origin.y + f.size.height
            })
            .or_else(|| NSScreen::mainScreen(mtm))
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

    use super::{MARGIN, Place};

    pub fn show(window: &WebviewWindow, (width, height): (f64, f64), place: Place) {
        let app = window.app_handle();
        let monitor = app
            .cursor_position()
            .ok()
            .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
            .or_else(|| app.primary_monitor().ok().flatten());
        if let Some(monitor) = monitor {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let px = |v: f64| (v * scale).round() as i32;
            let (area_w, area_h) = (
                i32::try_from(area.size.width).unwrap_or(0),
                i32::try_from(area.size.height).unwrap_or(0),
            );
            let (w, h) = (px(width), px(height).min(area_h - 2 * px(MARGIN)));
            let right = area.position.x + area_w - w - px(MARGIN);
            let (x, y) = match place {
                Place::BottomCentre => (
                    area.position.x + (area_w - w) / 2,
                    area.position.y + area_h - h - px(MARGIN),
                ),
                Place::RightTop => (right, area.position.y + px(MARGIN)),
                Place::RightThird => (right, area.position.y + area_h / 3),
            };
            let _ = window.set_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
        if let Err(e) = super::show_unfocused(window) {
            tracing::warn!("couldn't show a floating window: {e}");
        }
    }
}
