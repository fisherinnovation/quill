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
//!
//! They're dragged by the page, not the OS ([`drag_start`]): on macOS the
//! window Tauri would drag is the hidden one, not the panel.

pub use quill_core::settings::Spot;
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

/// Shows `window` at `size` (logical points): with its top-left corner at
/// `spot`, where the user dragged it, if that's still on a screen (moved
/// in to fit on it), else in `place` on the screen the mouse is on.
pub fn show(window: &WebviewWindow, size: (f64, f64), place: Place, spot: Option<Spot>) {
    #[cfg(target_os = "macos")]
    mac::show(window, size, place, spot);
    #[cfg(not(target_os = "macos"))]
    other::show(window, size, place, spot);
}

/// Resizes a window that's showing, keeping its top-left corner where it is.
pub fn resize(window: &WebviewWindow, size: (f64, f64)) {
    #[cfg(target_os = "macos")]
    mac::resize(window, size);
    #[cfg(not(target_os = "macos"))]
    other::resize(window, size);
}

/// The pointer went down on the window's drag handle.
pub fn drag_start(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    mac::drag_start(window);
    #[cfg(not(target_os = "macos"))]
    other::drag_start(window);
}

/// The pointer has moved `(dx, dy)` logical points (y down) since
/// [`drag_start`]: the window follows it.
pub fn drag_by(window: &WebviewWindow, (dx, dy): (f64, f64)) {
    #[cfg(target_os = "macos")]
    mac::drag_by(window, (dx, dy));
    #[cfg(not(target_os = "macos"))]
    other::drag_by(window, (dx, dy));
}

/// The pointer went up: `done` gets where the window's top-left corner
/// ended up, on the main thread on macOS.
pub fn drag_end(window: &WebviewWindow, done: impl FnOnce(Spot) + Send + 'static) {
    #[cfg(target_os = "macos")]
    mac::drag_end(window, done);
    #[cfg(not(target_os = "macos"))]
    other::drag_end(window, done);
}

/// Where a span of `len` starting at `start` goes to fit in the area's
/// span (`area_start`, `area_len`): moved in as little as it can be, and
/// to the area's start if it can't fit.
fn fit(start: f64, len: f64, (area_start, area_len): (f64, f64)) -> f64 {
    start.min(area_start + area_len - len).max(area_start)
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

    use super::{MARGIN, Place, Spot, fit};

    // AppKit objects live on the main thread, so the panels do too, and
    // where each was when a drag started (its top-left corner).
    thread_local! {
        static PANELS: RefCell<HashMap<String, Retained<NSPanel>>> = RefCell::new(HashMap::new());
        static DRAGS: RefCell<HashMap<String, NSPoint>> = RefCell::new(HashMap::new());
    }

    /// Runs `f` on the main thread with `window`'s panel, if it has one.
    fn with_panel(window: &WebviewWindow, f: impl FnOnce(&NSPanel, &str) + Send + 'static) {
        let label = window.label().to_string();
        let _ = window.run_on_main_thread(move || {
            PANELS.with(|panels| {
                if let Some(panel) = panels.borrow().get(&label) {
                    f(panel, &label);
                }
            });
        });
    }

    fn top_left(frame: NSRect) -> NSPoint {
        NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height)
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

    pub fn show(
        window: &WebviewWindow,
        (width, height): (f64, f64),
        place: Place,
        spot: Option<Spot>,
    ) {
        with_panel(window, move |panel, _| {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            // AppKit's y runs up from the bottom of the main screen.
            let frame = spot
                .and_then(|spot| {
                    // The middle of its top edge, a point inside it.
                    let inside = NSPoint::new(spot.x + width / 2.0, spot.y - 1.0);
                    let area = screen_at(mtm, inside)?.visibleFrame();
                    let height = height.min(area.size.height);
                    let x = fit(spot.x, width, (area.origin.x, area.size.width));
                    let y = fit(spot.y - height, height, (area.origin.y, area.size.height));
                    Some(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)))
                })
                .or_else(|| {
                    let area = screen_at(mtm, NSEvent::mouseLocation())
                        .or_else(|| NSScreen::mainScreen(mtm))?
                        .visibleFrame();
                    let height = height.min(area.size.height - 2.0 * MARGIN);
                    let right = area.origin.x + area.size.width - width - MARGIN;
                    let top = area.origin.y + area.size.height;
                    let origin = match place {
                        Place::BottomCentre => NSPoint::new(
                            area.origin.x + (area.size.width - width) / 2.0,
                            area.origin.y + MARGIN,
                        ),
                        Place::RightTop => NSPoint::new(right, top - height - MARGIN),
                        Place::RightThird => {
                            NSPoint::new(right, top - area.size.height / 3.0 - height)
                        }
                    };
                    Some(NSRect::new(origin, NSSize::new(width, height)))
                });
            let Some(frame) = frame else {
                return;
            };
            panel.setFrame_display(frame, true);
            // Shown without activating Quill.
            panel.orderFrontRegardless();
        });
    }

    pub fn resize(window: &WebviewWindow, (width, height): (f64, f64)) {
        with_panel(window, move |panel, _| {
            let top = top_left(panel.frame());
            let origin = NSPoint::new(top.x, top.y - height);
            panel.setFrame_display(NSRect::new(origin, NSSize::new(width, height)), true);
        });
    }

    pub fn drag_start(window: &WebviewWindow) {
        with_panel(window, |panel, label| {
            let from = top_left(panel.frame());
            DRAGS.with(|drags| drags.borrow_mut().insert(label.to_string(), from));
        });
    }

    pub fn drag_by(window: &WebviewWindow, (dx, dy): (f64, f64)) {
        with_panel(window, move |panel, label| {
            if let Some(from) = DRAGS.with(|drags| drags.borrow().get(label).copied()) {
                panel.setFrameTopLeftPoint(NSPoint::new(from.x + dx, from.y - dy));
            }
        });
    }

    pub fn drag_end(window: &WebviewWindow, done: impl FnOnce(Spot) + Send + 'static) {
        with_panel(window, move |panel, label| {
            if DRAGS
                .with(|drags| drags.borrow_mut().remove(label))
                .is_some()
            {
                let at = top_left(panel.frame());
                done(Spot { x: at.x, y: at.y });
            }
        });
    }

    pub fn hide(window: &WebviewWindow) {
        with_panel(window, |panel, _| panel.orderOut(None));
    }

    fn screen_at(mtm: MainThreadMarker, point: NSPoint) -> Option<Retained<NSScreen>> {
        NSScreen::screens(mtm).iter().find(|s| {
            let f = s.frame();
            point.x >= f.origin.x
                && point.x < f.origin.x + f.size.width
                && point.y >= f.origin.y
                && point.y < f.origin.y + f.size.height
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};

    use tauri::{Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

    use super::{MARGIN, Place, Spot, fit};

    /// Where each window being dragged was when its drag started, and its
    /// scale then (the page measures in logical points), by label.
    type Drags = HashMap<String, (PhysicalPosition<i32>, f64)>;
    static DRAGS: Mutex<Option<Drags>> = Mutex::new(None);

    fn drags<T>(f: impl FnOnce(&mut Drags) -> T) -> T {
        let mut drags = DRAGS.lock().unwrap_or_else(PoisonError::into_inner);
        f(drags.get_or_insert_with(HashMap::new))
    }

    pub fn show(
        window: &WebviewWindow,
        (width, height): (f64, f64),
        place: Place,
        spot: Option<Spot>,
    ) {
        let app = window.app_handle();
        let saved = spot.and_then(|spot| {
            let monitors = app.available_monitors().ok()?;
            // The middle of its top edge, a point inside it.
            let inside = |m: &&Monitor| {
                let a = m.work_area();
                let x = spot.x + width * m.scale_factor() / 2.0;
                let y = spot.y + 1.0;
                x >= f64::from(a.position.x)
                    && x < f64::from(a.position.x) + f64::from(a.size.width)
                    && y >= f64::from(a.position.y)
                    && y < f64::from(a.position.y) + f64::from(a.size.height)
            };
            let monitor = monitors.iter().find(inside)?;
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let (w, h) = (
                width * scale,
                (height * scale).min(f64::from(area.size.height)),
            );
            let x = fit(
                spot.x,
                w,
                (f64::from(area.position.x), f64::from(area.size.width)),
            );
            let y = fit(
                spot.y,
                h,
                (f64::from(area.position.y), f64::from(area.size.height)),
            );
            Some((
                x.round() as i32,
                y.round() as i32,
                w.round() as i32,
                h.round() as i32,
            ))
        });
        let frame = saved.or_else(|| {
            let monitor = app
                .cursor_position()
                .ok()
                .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
                .or_else(|| app.primary_monitor().ok().flatten())?;
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
            Some((x, y, w, h))
        });
        if let Some((x, y, w, h)) = frame {
            let _ = window.set_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
        if let Err(e) = super::show_unfocused(window) {
            tracing::warn!("couldn't show a floating window: {e}");
        }
    }

    pub fn resize(window: &WebviewWindow, (width, height): (f64, f64)) {
        // Windows keeps the top-left corner when the size changes.
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
    }

    pub fn drag_start(window: &WebviewWindow) {
        let (Ok(at), Ok(scale)) = (window.outer_position(), window.scale_factor()) else {
            return;
        };
        drags(|d| d.insert(window.label().to_string(), (at, scale)));
    }

    pub fn drag_by(window: &WebviewWindow, (dx, dy): (f64, f64)) {
        if let Some((from, scale)) = drags(|d| d.get(window.label()).copied()) {
            let x = from.x + (dx * scale).round() as i32;
            let y = from.y + (dy * scale).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
    }

    pub fn drag_end(window: &WebviewWindow, done: impl FnOnce(Spot) + Send + 'static) {
        if drags(|d| d.remove(window.label())).is_some()
            && let Ok(at) = window.outer_position()
        {
            done(Spot {
                x: f64::from(at.x),
                y: f64::from(at.y),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn a_window_is_moved_in_just_enough_to_fit() {
        let area = (100.0, 1000.0);
        assert_eq!(fit(300.0, 200.0, area), 300.0);
        // Hanging off the end, or before the start.
        assert_eq!(fit(1000.0, 200.0, area), 900.0);
        assert_eq!(fit(50.0, 200.0, area), 100.0);
        // Too big to fit: at the start.
        assert_eq!(fit(400.0, 1200.0, area), 100.0);
    }
}
