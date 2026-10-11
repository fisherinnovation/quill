//! Which app has focus: where dictated text is about to go.
//!
//! Only the app's identity is read (bundle id or executable, and display
//! name), never its windows or contents.

/// The app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontApp {
    /// Stable identifier: the bundle id on macOS (`com.tinyspeck.slackmacgap`),
    /// the lowercased executable name on Windows (`slack.exe`).
    pub id: String,
    /// What to show people, e.g. "Slack".
    pub name: String,
}

/// The app that currently has keyboard focus, if it can be told.
#[cfg(target_os = "macos")]
pub fn frontmost_app() -> Option<FrontApp> {
    use objc2_app_kit::NSWorkspace;
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let name = app.localizedName().map(|n| n.to_string());
    let id = app.bundleIdentifier().map(|i| i.to_string());
    match (id, name) {
        (Some(id), Some(name)) => Some(FrontApp { id, name }),
        (Some(id), None) => Some(FrontApp {
            name: id.rsplit('.').next().unwrap_or(&id).to_string(),
            id,
        }),
        (None, Some(name)) => Some(FrontApp {
            id: name.clone(),
            name,
        }),
        (None, None) => None,
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn frontmost_app() -> Option<FrontApp> {
    use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    use windows::core::PWSTR;

    // SAFETY: GetForegroundWindow takes no arguments and returns a handle
    // (possibly null), which we only pass to GetWindowThreadProcessId.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let mut pid = 0_u32;
    // SAFETY: `pid` is a valid u32 to write the process id into.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    // SAFETY: opening with the least access needed to read its image name;
    // the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0_u16; MAX_PATH as usize];
    let mut len = buffer.len() as u32;
    // SAFETY: `buffer` holds `len` u16s and the API writes at most that many,
    // updating `len` to what it wrote.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: `process` came from OpenProcess above and isn't used again.
    let _ = unsafe { CloseHandle(process) };
    ok.ok()?;
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    let file = path.rsplit(['\\', '/']).next()?.to_string();
    let stem = file.strip_suffix(".exe").unwrap_or(&file);
    let mut name: Vec<char> = stem.chars().collect();
    if let Some(first) = name.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    Some(FrontApp {
        id: file.to_lowercase(),
        name: name.into_iter().collect(),
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn frontmost_app() -> Option<FrontApp> {
    None
}
