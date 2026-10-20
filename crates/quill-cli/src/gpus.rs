//! `quill-cli gpus`: the graphics cards Quill can transcribe on, and which
//! one it uses on its own (Settings → Models → Graphics card).

use quill_stt::{GpuChoice, gpu};

pub fn run() -> anyhow::Result<()> {
    let Some(backend) = quill_stt::gpu_backend() else {
        println!("This build transcribes on the CPU only.");
        return Ok(());
    };
    let gpus = gpu::gpus();
    if gpus.is_empty() {
        println!("{backend}: no usable graphics card, so Quill transcribes on the CPU.");
        return Ok(());
    }
    let automatic = gpu::choose(&gpus, &GpuChoice::Automatic);
    println!("{backend}:");
    for (i, g) in gpus.iter().enumerate() {
        let kind = if g.integrated {
            "integrated"
        } else {
            "dedicated"
        };
        let memory = g.memory_bytes as f64 / f64::from(1 << 30);
        let mark = if Some(i) == automatic {
            "  ← Automatic"
        } else {
            ""
        };
        println!("  {i}  {}  ({kind}, {memory:.1} GB){mark}", g.name);
    }
    Ok(())
}
