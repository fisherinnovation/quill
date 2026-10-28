//! What kind of text box has keyboard focus: one line (a search box, address
//! bar, form field or title) or several (a message, document or email).
//! Dictation leaves the final period off text going into one line.
//!
//! Only the focused element's role is read, never its text, and nothing is
//! kept. Some apps don't say (terminals, editors that draw their own text),
//! so it can be unknown.

use std::time::Duration;

/// The kind of text box that has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    SingleLine,
    MultiLine,
}

/// How long an app gets to answer. A hung app otherwise holds the caller
/// for seconds (6 s by default on macOS).
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
const TIMEOUT: Duration = Duration::from_millis(250);

/// Chrome, Edge and Electron apps build their accessibility tree when first
/// asked, and answer with the whole page until it's ready.
const RETRY_AFTER: Duration = Duration::from_millis(300);

/// The kind of text box with keyboard focus, if the app says. Asks again
/// after [`RETRY_AFTER`] if it doesn't, and each question can take up to
/// [`TIMEOUT`], so call it off any thread that must stay responsive.
/// macOS: needs the Accessibility permission, like pasting.
pub fn focused_field() -> Option<FieldKind> {
    query().or_else(|| {
        std::thread::sleep(RETRY_AFTER);
        query()
    })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn query() -> Option<FieldKind> {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_foundation::{CFRetained, CFString, CFType};

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> *mut c_void;
        fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *mut c_void,
        ) -> i32;
    }

    fn raw<T>(r: &T) -> *const c_void {
        (r as *const T).cast()
    }

    fn attribute(element: &CFType, name: &'static str) -> Option<CFRetained<CFType>> {
        let name = CFString::from_static_str(name);
        let mut value = std::ptr::null_mut();
        // SAFETY: `element` is a live AXUIElement and `name` a live CFString.
        // On success (0), `value` holds a +1 reference that CFRetained takes
        // over.
        let error = unsafe { AXUIElementCopyAttributeValue(raw(element), raw(&*name), &mut value) };
        if error != 0 {
            return None;
        }
        let value = NonNull::new(value.cast::<CFType>())?;
        // SAFETY: as above.
        Some(unsafe { CFRetained::from_raw(value) })
    }

    fn string(element: &CFType, name: &'static str) -> Option<String> {
        let value = attribute(element, name)?;
        value.downcast_ref::<CFString>().map(ToString::to_string)
    }

    // SAFETY: takes no arguments; returns a +1 reference or null.
    let system = unsafe { AXUIElementCreateSystemWide() };
    let system = NonNull::new(system.cast::<CFType>())?;
    // SAFETY: the +1 reference from above, which CFRetained takes over.
    let system = unsafe { CFRetained::from_raw(system) };
    // On the system-wide element this sets the timeout for every element.
    // SAFETY: `system` is a live AXUIElement.
    unsafe { AXUIElementSetMessagingTimeout(raw(&*system), TIMEOUT.as_secs_f32()) };
    let focused = attribute(&system, "AXFocusedUIElement")?;
    let role = string(&focused, "AXRole")?;
    let subrole = string(&focused, "AXSubrole");
    from_ax_role(&role, subrole.as_deref())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn query() -> Option<FieldKind> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation2};
    use windows::Win32::UI::WindowsAndMessaging::{
        ES_MULTILINE, GWL_STYLE, GetClassNameW, GetWindowLongW,
    };

    fn ask() -> Option<FieldKind> {
        // SAFETY: COM is initialized on this thread (below); the interfaces
        // are released when they drop, before CoUninitialize.
        unsafe {
            let automation: IUIAutomation2 =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
            let ms = TIMEOUT.as_millis() as u32;
            let _ = automation.SetConnectionTimeout(ms);
            let _ = automation.SetTransactionTimeout(ms);
            let element = automation.GetFocusedElement().ok()?;
            if element.CurrentIsPassword().is_ok_and(|p| p.as_bool()) {
                return None;
            }
            let control_type = element.CurrentControlType().ok()?.0;
            let aria_role = element
                .CurrentAriaRole()
                .map(|r| r.to_string())
                .unwrap_or_default();
            let aria_properties = element
                .CurrentAriaProperties()
                .map(|p| p.to_string())
                .unwrap_or_default();
            // A classic Win32 edit control says whether it's multi-line in
            // its window style. Other windows use those bits for other things.
            let win32_multiline = element
                .CurrentNativeWindowHandle()
                .ok()
                .filter(|w| !w.is_invalid())
                .and_then(|window| {
                    let mut class = [0_u16; 64];
                    let len = usize::try_from(GetClassNameW(window, &mut class)).ok()?;
                    let class = String::from_utf16_lossy(&class[..len]).to_lowercase();
                    (class == "edit" || class.starts_with("richedit"))
                        .then(|| GetWindowLongW(window, GWL_STYLE) & ES_MULTILINE != 0)
                });
            from_uia(control_type, &aria_role, &aria_properties, win32_multiline)
        }
    }

    // SAFETY: balanced by CoUninitialize below when it succeeds. UI
    // Automation clients are meant to run in the multithreaded apartment.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let field = ask();
    if init.is_ok() {
        // SAFETY: balances the CoInitializeEx above; nothing COM is alive.
        unsafe { CoUninitialize() };
    }
    field
}

#[cfg(not(any(target_os = "macos", windows)))]
fn query() -> Option<FieldKind> {
    None
}

/// From macOS Accessibility's role and subrole for the focused element.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn from_ax_role(role: &str, subrole: Option<&str>) -> Option<FieldKind> {
    match (role, subrole) {
        // Passwords aren't dictated into.
        (_, Some("AXSecureTextField")) => None,
        // Search fields are text fields with a subrole; a web page's
        // `role=combobox` (Google's search box) is a combo box.
        ("AXTextField" | "AXSearchField" | "AXComboBox", _) => Some(FieldKind::SingleLine),
        ("AXTextArea", _) => Some(FieldKind::MultiLine),
        _ => None,
    }
}

/// From UI Automation: the control type, the ARIA role and properties
/// (Chromium's web pages and its address bar) and, for a classic Win32 edit
/// control, whether it's multi-line. The control type alone can't tell an
/// `<input>` from a `<textarea>`: both are Edits.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_uia(
    control_type: i32,
    aria_role: &str,
    aria_properties: &str,
    win32_multiline: Option<bool>,
) -> Option<FieldKind> {
    const COMBO_BOX: i32 = 50003;
    const EDIT: i32 = 50004;
    const DOCUMENT: i32 = 50030;
    // "multiline=false;readonly=true;…"
    let property = |name: &str| {
        aria_properties
            .split(';')
            .find_map(|p| p.split_once('=').filter(|(k, _)| *k == name))
            .map(|(_, v)| v == "true")
    };
    // Not a text box: a page still building its tree, or a label.
    if property("readonly") == Some(true) {
        return None;
    }
    if matches!(aria_role, "searchbox" | "combobox") {
        return Some(FieldKind::SingleLine);
    }
    let lines = |multi: bool| {
        if multi {
            FieldKind::MultiLine
        } else {
            FieldKind::SingleLine
        }
    };
    let multiline = property("multiline").or(win32_multiline);
    match control_type {
        COMBO_BOX => Some(FieldKind::SingleLine),
        DOCUMENT => Some(FieldKind::MultiLine),
        EDIT => multiline.map(lines),
        // A `contenteditable` (a group in Chromium) says it's multi-line.
        _ => (multiline == Some(true)).then_some(FieldKind::MultiLine),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_roles() {
        let single = Some(FieldKind::SingleLine);
        assert_eq!(from_ax_role("AXTextField", None), single);
        assert_eq!(from_ax_role("AXTextField", Some("AXSearchField")), single);
        assert_eq!(from_ax_role("AXComboBox", None), single);
        assert_eq!(from_ax_role("AXTextArea", None), Some(FieldKind::MultiLine));
        assert_eq!(from_ax_role("AXTextField", Some("AXSecureTextField")), None);
        assert_eq!(from_ax_role("AXWebArea", None), None);
        assert_eq!(from_ax_role("AXButton", None), None);
    }

    #[test]
    fn windows_controls() {
        let single = Some(FieldKind::SingleLine);
        let multi = Some(FieldKind::MultiLine);
        // As Chrome and Edge report them.
        let input = "expanded=false;multiline=false;required=false";
        let textarea = "expanded=false;multiline=true;required=false";
        assert_eq!(from_uia(50004, "searchbox", input, None), single);
        assert_eq!(from_uia(50004, "combobox", textarea, None), single);
        assert_eq!(from_uia(50004, "textbox", input, None), single);
        assert_eq!(from_uia(50004, "textbox", textarea, None), multi);
        assert_eq!(from_uia(50026, "group", textarea, None), multi);
        // The page itself, before the tree is ready, and other controls.
        let page = "readonly=true;expanded=false;multiline=false";
        assert_eq!(from_uia(50033, "region", page, None), None);
        assert_eq!(from_uia(50026, "group", input, None), None);
        assert_eq!(from_uia(50000, "button", input, None), None);
        // Classic Win32 edits, by their style; native controls.
        assert_eq!(from_uia(50004, "", "", Some(false)), single);
        assert_eq!(from_uia(50004, "", "", Some(true)), multi);
        assert_eq!(from_uia(50004, "", "", None), None);
        assert_eq!(from_uia(50003, "", "", None), single);
        assert_eq!(from_uia(50030, "", "", None), multi);
    }
}
