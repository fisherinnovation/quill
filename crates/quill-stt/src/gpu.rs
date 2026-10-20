//! The graphics cards whisper.cpp can transcribe on, and which one to use.
//!
//! whisper.cpp takes the card as an index (`gpu_device`) among ggml's GPU
//! devices, dedicated and integrated alike, in the order the backend lists
//! them (for Vulkan, the driver's order), and uses the first by default. On
//! a PC that lists its integrated graphics before a dedicated card, that's
//! the slower one. So Quill chooses: the card the user picked, by name, or
//! else a dedicated card, the one with the most memory.

use serde::{Deserialize, Serialize};

/// A graphics card whisper.cpp can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gpu {
    /// As the driver names it, e.g. "NVIDIA GeForce RTX 4070". Also how a
    /// choice is remembered: positions shift when cards or drivers change.
    pub name: String,
    /// Integrated graphics (part of the CPU), not a dedicated card.
    pub integrated: bool,
    /// Its memory in bytes, 0 if the backend doesn't say. For integrated
    /// graphics, the share of system memory it can use.
    pub memory_bytes: u64,
}

/// Which card to transcribe on, as a setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum GpuChoice {
    /// A dedicated card if there is one, the one with the most memory;
    /// otherwise integrated graphics.
    #[default]
    Automatic,
    /// No graphics card: slower, for a card that misbehaves.
    Cpu,
    /// This card, by [`Gpu::name`]; Automatic if it's no longer there.
    Named(String),
}

/// The graphics cards whisper.cpp can use, in its order: a card's position
/// is its `gpu_device`. Empty without a GPU backend or a usable card.
pub fn gpus() -> Vec<Gpu> {
    // Listing sets up the backends, which log; send that through `tracing`.
    whisper_rs::install_logging_hooks();
    imp::gpus()
}

/// The position in `gpus` to transcribe on for `choice`, or `None` for the
/// CPU.
pub fn choose(gpus: &[Gpu], choice: &GpuChoice) -> Option<usize> {
    match choice {
        GpuChoice::Cpu => None,
        GpuChoice::Named(name) => gpus
            .iter()
            .position(|g| g.name == *name)
            .or_else(|| automatic(gpus)),
        GpuChoice::Automatic => automatic(gpus),
    }
}

/// Dedicated before integrated, then the most memory; on a tie, the first
/// listed.
fn automatic(gpus: &[Gpu]) -> Option<usize> {
    gpus.iter()
        .enumerate()
        .max_by_key(|(i, g)| (!g.integrated, g.memory_bytes, std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

// ggml's device registry is a C API: the calls below are FFI.
#[allow(unsafe_code)]
mod imp {
    use std::ffi::{CStr, c_char};

    use whisper_rs_sys as sys;

    use super::Gpu;

    /// A string ggml owns, or "" for none.
    fn text(ptr: *const c_char) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: ggml returns NUL-terminated strings that live as long as
        // the device, which is the whole program.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .trim()
            .to_string()
    }

    pub fn gpus() -> Vec<Gpu> {
        // The same filter whisper.cpp applies when it counts `gpu_device`
        // (`whisper_backend_init_gpu`), so positions match.
        let dedicated = sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU;
        let integrated = sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU;
        // SAFETY: no arguments; ggml sets up its registry on first use, and
        // it lives for the rest of the program.
        let count = unsafe { sys::ggml_backend_dev_count() };
        let mut gpus = Vec::new();
        for i in 0..count {
            // SAFETY: `i` is below the device count.
            let device = unsafe { sys::ggml_backend_dev_get(i) };
            if device.is_null() {
                continue;
            }
            // SAFETY: `device` is a valid device from the registry.
            let kind = unsafe { sys::ggml_backend_dev_type(device) };
            if kind != dedicated && kind != integrated {
                continue;
            }
            // SAFETY: as above.
            let mut name = text(unsafe { sys::ggml_backend_dev_description(device) });
            if name.is_empty() {
                // SAFETY: as above.
                name = text(unsafe { sys::ggml_backend_dev_name(device) });
            }
            let (mut free, mut total) = (0_usize, 0_usize);
            // SAFETY: a valid device, and two out-parameters ggml writes.
            unsafe { sys::ggml_backend_dev_memory(device, &mut free, &mut total) };
            gpus.push(Gpu {
                name,
                integrated: kind == integrated,
                memory_bytes: total as u64,
            });
        }
        gpus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn gpu(name: &str, integrated: bool, gb: u64) -> Gpu {
        Gpu {
            name: name.into(),
            integrated,
            memory_bytes: gb * GB,
        }
    }

    #[test]
    fn automatic_prefers_a_dedicated_card_listed_after_integrated_graphics() {
        // Integrated graphics can report more (shared) memory than a card.
        let gpus = [gpu("Intel UHD 770", true, 16), gpu("RTX 4070", false, 12)];
        assert_eq!(choose(&gpus, &GpuChoice::Automatic), Some(1));
    }

    #[test]
    fn automatic_picks_the_dedicated_card_with_the_most_memory_then_the_first() {
        let gpus = [
            gpu("RTX 3060", false, 8),
            gpu("RTX 4090", false, 24),
            gpu("RTX 4090", false, 24),
        ];
        assert_eq!(choose(&gpus, &GpuChoice::Automatic), Some(1));
    }

    #[test]
    fn automatic_uses_integrated_graphics_when_thats_all_there_is() {
        let gpus = [gpu("Intel Iris Xe", true, 8)];
        assert_eq!(choose(&gpus, &GpuChoice::Automatic), Some(0));
        assert_eq!(choose(&[], &GpuChoice::Automatic), None);
    }

    #[test]
    fn a_named_card_is_used_wherever_it_is_listed_and_automatic_if_its_gone() {
        let gpus = [gpu("Intel UHD 770", true, 16), gpu("RTX 4070", false, 12)];
        let intel = GpuChoice::Named("Intel UHD 770".into());
        assert_eq!(choose(&gpus, &intel), Some(0));
        let gone = GpuChoice::Named("RTX 2060".into());
        assert_eq!(choose(&gpus, &gone), Some(1));
        assert_eq!(choose(&gpus, &GpuChoice::Cpu), None);
    }

    #[test]
    fn the_choice_is_saved_readably() {
        let named = GpuChoice::Named("RTX 4070".into());
        let json = serde_json::to_string(&named).unwrap();
        assert_eq!(json, r#"{"kind":"named","name":"RTX 4070"}"#);
        assert_eq!(
            serde_json::to_string(&GpuChoice::Automatic).unwrap(),
            r#"{"kind":"automatic"}"#
        );
        assert_eq!(serde_json::from_str::<GpuChoice>(&json).unwrap(), named);
    }
}
