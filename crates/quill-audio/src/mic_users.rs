//! Which apps are using a microphone right now: the signal for noticing that
//! a call has started (Zoom, Teams, a browser tab in Meet).
//!
//! On macOS 14.2+ Core Audio lists its clients as process objects, each with
//! a PID, a bundle id and whether it's recording. Only those are read: never
//! audio, windows or what the app is showing.

/// A process recording from an input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicUser {
    pub pid: i32,
    /// E.g. `us.zoom.xos`, or a browser's helper process. `None` if the
    /// process has no bundle (a command-line tool).
    pub bundle_id: Option<String>,
}

/// The processes recording from a microphone (any input device) right now.
pub fn mic_users() -> Result<Vec<MicUser>, crate::AudioError> {
    imp::mic_users()
}

// Core Audio is a C API: the property reads below are FFI.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{NonNull, null};

    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertySelector,
        kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
        kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyPID,
    };
    use objc2_core_foundation::{CFRetained, CFString};

    use super::MicUser;
    use crate::AudioError;

    fn address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    /// A fixed-size property of `object`.
    fn get<T: Copy + Default>(
        object: AudioObjectID,
        selector: AudioObjectPropertySelector,
        what: &'static str,
    ) -> Result<T, AudioError> {
        let mut value = T::default();
        let mut size = u32::try_from(size_of::<T>()).unwrap_or(u32::MAX);
        let address = address(selector);
        // SAFETY: `value` is a `T` and `size` says how big it is; Core Audio
        // writes at most `size` bytes.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        if status != 0 {
            return Err(AudioError::CoreAudio { what, status });
        }
        Ok(value)
    }

    /// Every process object Core Audio knows about.
    fn processes() -> Result<Vec<AudioObjectID>, AudioError> {
        let system = AudioObjectID::try_from(kAudioObjectSystemObject).unwrap_or(1);
        let address = address(kAudioHardwarePropertyProcessObjectList);
        let mut size = 0_u32;
        // SAFETY: asks only for the size of the list.
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                system,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
            )
        };
        if status != 0 {
            return Err(AudioError::CoreAudio {
                what: "list audio processes",
                status,
            });
        }
        let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
        // A Vec's pointer is never null; empty means no processes.
        let Some(out) = NonNull::new(ids.as_mut_ptr()).filter(|_| !ids.is_empty()) else {
            return Ok(Vec::new());
        };
        // SAFETY: `ids` has room for `size` bytes of object ids.
        let status = unsafe {
            AudioObjectGetPropertyData(
                system,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                out.cast::<c_void>(),
            )
        };
        if status != 0 {
            return Err(AudioError::CoreAudio {
                what: "list audio processes",
                status,
            });
        }
        // The list can shrink between the two calls.
        ids.truncate(size as usize / size_of::<AudioObjectID>());
        Ok(ids)
    }

    fn bundle_id(process: AudioObjectID) -> Option<String> {
        let raw: usize = get(process, kAudioProcessPropertyBundleID, "read a bundle id").ok()?;
        // The property is a CFStringRef the caller owns (+1).
        let string = NonNull::new(raw as *mut CFString)?;
        // SAFETY: Core Audio returned a retained CFString; we take that
        // reference and release it when `owned` drops.
        let owned = unsafe { CFRetained::from_raw(string) };
        let id = owned.to_string();
        (!id.is_empty()).then_some(id)
    }

    pub fn mic_users() -> Result<Vec<MicUser>, AudioError> {
        let mut users = Vec::new();
        for process in processes()? {
            // A process can exit between listing and asking; skip it.
            let Ok(running) = get::<u32>(
                process,
                kAudioProcessPropertyIsRunningInput,
                "read input state",
            ) else {
                continue;
            };
            if running == 0 {
                continue;
            }
            let pid = get::<i32>(process, kAudioProcessPropertyPID, "read a PID").unwrap_or(-1);
            users.push(MicUser {
                pid,
                bundle_id: bundle_id(process),
            });
        }
        Ok(users)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::MicUser;
    use crate::AudioError;

    pub fn mic_users() -> Result<Vec<MicUser>, AudioError> {
        Err(AudioError::Unsupported(
            "Listing the apps using the microphone",
        ))
    }
}
