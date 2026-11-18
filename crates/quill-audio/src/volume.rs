//! The computer's output volume, lowered while dictating (Settings →
//! Dictation) so that what the speakers play is less likely to reach the
//! microphone and be typed with what the user says.
//!
//! [`lower_to`] lowers the default output device's volume and hands back
//! a [`Lowered`], which puts it back when dropped. The same device is put
//! back even if the default output changes in between. A volume already at
//! or below the level is left alone, and so is one the user changed while
//! it was lowered: their choice stands.

use crate::AudioError;

/// Volumes this close count as the same: devices round what they're set to.
const MARGIN: f32 = 0.01;

/// The output volume as it was before [`lower_to`] lowered it. Dropping it
/// puts it back, unless the user has changed the volume since.
pub struct Lowered(imp::Lowered);

impl Drop for Lowered {
    fn drop(&mut self) {
        if let Err(e) = self.0.restore() {
            tracing::warn!("couldn't put the output volume back: {e}");
        }
    }
}

/// Lowers the default output device's volume to `level` (0–1, as the
/// system's volume control shows it). `None` if there was nothing to do: the
/// volume was already that low, or the device has no volume of its own
/// (some HDMI and USB outputs).
pub fn lower_to(level: f32) -> Result<Option<Lowered>, AudioError> {
    Ok(imp::lower_to(level.clamp(0.0, 1.0))?.map(Lowered))
}

// Core Audio is a C API: the property reads and writes below are FFI.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{NonNull, null};

    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectHasProperty, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertyElement, AudioObjectPropertyScope,
        AudioObjectPropertySelector, AudioObjectSetPropertyData,
        kAudioDevicePropertyPreferredChannelsForStereo, kAudioDevicePropertyVolumeScalar,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    };

    use super::MARGIN;
    use crate::AudioError;

    pub struct Lowered {
        device: AudioObjectID,
        /// Each channel's volume before, by element.
        was: Vec<(AudioObjectPropertyElement, f32)>,
        set: f32,
    }

    fn address(
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
        element: AudioObjectPropertyElement,
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: element,
        }
    }

    fn volume(element: AudioObjectPropertyElement) -> AudioObjectPropertyAddress {
        address(
            kAudioDevicePropertyVolumeScalar,
            kAudioObjectPropertyScopeOutput,
            element,
        )
    }

    /// A fixed-size property of `object`.
    fn get<T: Copy + Default>(
        object: AudioObjectID,
        address: AudioObjectPropertyAddress,
        what: &'static str,
    ) -> Result<T, AudioError> {
        let mut value = T::default();
        let mut size = u32::try_from(size_of::<T>()).unwrap_or(u32::MAX);
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

    fn set_volume(
        device: AudioObjectID,
        element: AudioObjectPropertyElement,
        level: f32,
        what: &'static str,
    ) -> Result<(), AudioError> {
        let address = volume(element);
        let mut level = level;
        // SAFETY: the volume is a Float32, and `level` is one.
        let status = unsafe {
            AudioObjectSetPropertyData(
                device,
                NonNull::from(&address),
                0,
                null(),
                u32::try_from(size_of::<f32>()).unwrap_or(4),
                NonNull::from(&mut level).cast::<c_void>(),
            )
        };
        if status != 0 {
            return Err(AudioError::CoreAudio { what, status });
        }
        Ok(())
    }

    /// Where `device`'s volume is: the main element if it has one, else
    /// each of its stereo channels (most built-in and USB outputs). Empty if
    /// it has no volume of its own.
    fn elements(device: AudioObjectID) -> Vec<AudioObjectPropertyElement> {
        // SAFETY: only asks whether the property exists.
        let has =
            |element| unsafe { AudioObjectHasProperty(device, NonNull::from(&volume(element))) };
        if has(kAudioObjectPropertyElementMain) {
            return vec![kAudioObjectPropertyElementMain];
        }
        let stereo: [AudioObjectPropertyElement; 2] = get(
            device,
            address(
                kAudioDevicePropertyPreferredChannelsForStereo,
                kAudioObjectPropertyScopeOutput,
                kAudioObjectPropertyElementMain,
            ),
            "find the stereo channels",
        )
        .unwrap_or([1, 2]);
        stereo.into_iter().filter(|&element| has(element)).collect()
    }

    pub fn lower_to(level: f32) -> Result<Option<Lowered>, AudioError> {
        let system = AudioObjectID::try_from(kAudioObjectSystemObject).unwrap_or(1);
        let device: AudioObjectID = get(
            system,
            address(
                kAudioHardwarePropertyDefaultOutputDevice,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
            "find the output device",
        )?;
        let mut was = Vec::new();
        for element in elements(device) {
            was.push((
                element,
                get(device, volume(element), "read the output volume")?,
            ));
        }
        if was.iter().all(|&(_, v)| v <= level + MARGIN) {
            return Ok(None);
        }
        for &(element, v) in &was {
            if v > level {
                set_volume(device, element, level, "lower the output volume")?;
            }
        }
        Ok(Some(Lowered {
            device,
            was,
            set: level,
        }))
    }

    impl Lowered {
        pub fn restore(&self) -> Result<(), AudioError> {
            for &(element, was) in &self.was {
                if was <= self.set {
                    continue;
                }
                let now: f32 = get(self.device, volume(element), "read the output volume")?;
                if (now - self.set).abs() <= MARGIN {
                    set_volume(self.device, element, was, "put the output volume back")?;
                }
            }
            Ok(())
        }
    }
}

// The Windows Core Audio interfaces are COM: the calls below are FFI.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };
    use windows::core::PCWSTR;

    use super::MARGIN;
    use crate::AudioError;

    pub struct Lowered {
        /// The output device's id, so it's the one put back.
        device: Vec<u16>,
        was: f32,
        set: f32,
    }

    fn failed(what: &'static str) -> impl FnOnce(windows::core::Error) -> AudioError {
        move |e| AudioError::Volume(format!("couldn't {what}: {e}"))
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
        // SAFETY: joins this thread to COM's multithreaded apartment. Already
        // joined (to either) is fine: the result is only that.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // SAFETY: creates the system's device enumerator.
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(failed("list the audio devices"))
    }

    fn endpoint(device: &IMMDevice) -> Result<IAudioEndpointVolume, AudioError> {
        // SAFETY: asks the device for its volume control.
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(failed("open the output volume"))
    }

    fn get(volume: &IAudioEndpointVolume) -> Result<f32, AudioError> {
        // SAFETY: reads a float.
        unsafe { volume.GetMasterVolumeLevelScalar() }.map_err(failed("read the output volume"))
    }

    fn set(volume: &IAudioEndpointVolume, level: f32) -> Result<(), AudioError> {
        // SAFETY: sets a float; no event context.
        unsafe { volume.SetMasterVolumeLevelScalar(level, std::ptr::null()) }
            .map_err(failed("change the output volume"))
    }

    pub fn lower_to(level: f32) -> Result<Option<Lowered>, AudioError> {
        // SAFETY: the default playback device, as the volume flyout shows it.
        let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(failed("find the output device"))?;
        // SAFETY: the id is a string COM allocated for us: copied, then freed.
        let id = unsafe {
            let id = device.GetId().map_err(failed("read the output device"))?;
            let copy = id.as_wide().to_vec();
            CoTaskMemFree(Some(id.0.cast_const().cast()));
            copy
        };
        let volume = endpoint(&device)?;
        let was = get(&volume)?;
        if was <= level + MARGIN {
            return Ok(None);
        }
        set(&volume, level)?;
        Ok(Some(Lowered {
            device: id,
            was,
            set: level,
        }))
    }

    impl Lowered {
        pub fn restore(&self) -> Result<(), AudioError> {
            let mut id = self.device.clone();
            id.push(0);
            // SAFETY: `id` is a nul-terminated UTF-16 string that outlives
            // the call.
            let device = unsafe { enumerator()?.GetDevice(PCWSTR(id.as_ptr())) }
                .map_err(failed("find the output device"))?;
            let volume = endpoint(&device)?;
            if (get(&volume)? - self.set).abs() <= MARGIN {
                set(&volume, self.was)?;
            }
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use crate::AudioError;

    pub struct Lowered;

    pub fn lower_to(_level: f32) -> Result<Option<Lowered>, AudioError> {
        Err(AudioError::Unsupported("changing the output volume"))
    }

    impl Lowered {
        pub fn restore(&self) -> Result<(), AudioError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Changes this computer's volume for a moment: run by hand, with
    /// `--ignored`, and listen.
    #[test]
    #[ignore = "changes the computer's output volume"]
    fn lowers_the_volume_and_puts_it_back() {
        let lowered = lower_to(0.05).unwrap();
        assert!(lowered.is_some(), "the volume was already at 5% or below");
        std::thread::sleep(std::time::Duration::from_secs(2));
        drop(lowered);
        // Put back: it can be lowered again.
        assert!(lower_to(0.05).unwrap().is_some(), "not put back");
        // Already low: nothing to do.
        assert!(lower_to(1.0).unwrap().is_none());
    }
}
