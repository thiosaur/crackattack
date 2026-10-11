//! Render dump for offline frame analysis.
//!
//! Outputs JSONL records per draw call for offline analysis.
//! Enable via RENDER_DUMP environment variable or explicit call.

use crate::renderer::gpu::uniforms::FrameUniforms;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

struct DumpState {
    writer: BufWriter<File>,
    current_frame: u32,
}

static DUMP_STATE: Mutex<Option<DumpState>> = Mutex::new(None);
/// Mirrors `DUMP_STATE.is_some()` without locking the mutex. The dump is off by
/// default, so `is_enabled()` (called on every draw call) avoids a lock per draw.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Write to dump file
macro_rules! dump_write {
    ($s:expr, $($arg:tt)*) => {
        { writeln!($s.writer, $($arg)*).ok(); }
    };
}

/// FNV-1a 64-bit hash (standard offset basis and prime).
pub fn hash_fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Convert hash to hex string (16 chars, lowercase)
pub fn hash_to_hex(hash: u64) -> String {
    format!("{:016x}", hash)
}

/// Enable dump to file
pub fn enable(filename: &str) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);

    match File::create(filename) {
        Ok(file) => {
            *state = Some(DumpState {
                writer: BufWriter::new(file),
                current_frame: 0,
            });
            ENABLED.store(true, Ordering::Relaxed);
            eprintln!("[RenderDump] Enabled, writing to {}", filename);
        }
        Err(e) => {
            eprintln!("[RenderDump] Failed to open {}: {}", filename, e);
        }
    }
}

/// Check if dump is enabled. Reads an atomic flag, so it never locks the mutex.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Called at start of frame
pub fn frame_start(frame: u32, uniforms: &FrameUniforms) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);
    if let Some(ref mut s) = *state {
        s.current_frame = frame;
        dump_write!(s,
            r#"{{"type":"frame_start","frame":{},"time_step":{},"play_offset_y":{:.6}}}"#,
            frame, uniforms.time_step, uniforms.play_offset_y
        );
    }
}

/// Called at end of frame
pub fn frame_end(frame: u32, total_draws: u32) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);
    if let Some(ref mut s) = *state {
        dump_write!(s,
            r#"{{"type":"frame_end","frame":{},"total_draws":{}}}"#,
            frame, total_draws
        );
        s.writer.flush().ok();
    }
}

/// Log instanced draw call (blocks).
///
/// `transform_hash` hashes the real uniform (the CPU copy of the GPU buffer).
/// Dumps are only comparable between runs of the same version.
pub fn draw_instanced(
    frame: u32,
    draw_idx: u32,
    mesh_name: &str,
    instance_count: u32,
    transform_data: &[u8],  // 352 bytes (TransformUniformsGPU)
    material_data: &[u8],   // 96 bytes (MaterialUniformsGPU)
    instance_data: &[u8],   // instance_count * sizeof(Instance)
) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);
    if let Some(ref mut s) = *state {
        let th = hash_to_hex(hash_fnv1a(transform_data));
        let mh = hash_to_hex(hash_fnv1a(material_data));
        let ih = hash_to_hex(hash_fnv1a(instance_data));

        dump_write!(s,
            r#"{{"type":"draw_instanced","frame":{},"draw_idx":{},"mesh":"{}","instance_count":{},"transform_hash":"{}","material_hash":"{}","instance_hash":"{}"}}"#,
            frame, draw_idx, mesh_name, instance_count,
            th, mh, ih
        );
    }
}

/// Log garbage instanced draw call.
///
/// Takes what the dispatcher knows: mesh name, instance range and texture id.
/// No hashes of the raw bytes are recorded.
pub fn draw_garbage_instanced(
    frame: u32,
    draw_idx: u32,
    mesh_name: &str,
    instance_count: u32,
    first_instance: u32,
    texture_id: u32,
) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);
    if let Some(ref mut s) = *state {
        dump_write!(s,
            r#"{{"type":"draw_garbage_instanced","frame":{},"draw_idx":{},"mesh":"{}","instance_count":{},"first_instance":{},"texture_id":{}}}"#,
            frame, draw_idx, mesh_name, instance_count, first_instance,
            texture_id
        );
    }
}

/// Log UI batch draw call
#[allow(clippy::too_many_arguments)]
pub fn draw_ui_batch(
    frame: u32,
    draw_idx: u32,
    vertex_count: u32,
    index_count: u32,
    texture_mode: &str,  // "None", "AlphaText", "Modulate"
    vertex_data: &[u8],
    index_data: &[u8],
) {
    let mut state = crate::sync_ext::lock_ignore_poison(&DUMP_STATE);
    if let Some(ref mut s) = *state {
        let vh = hash_to_hex(hash_fnv1a(vertex_data));
        let ih = hash_to_hex(hash_fnv1a(index_data));

        dump_write!(s,
            r#"{{"type":"draw_ui_batch","frame":{},"draw_idx":{},"vertex_count":{},"index_count":{},"texture_mode":"{}","vertex_hash":"{}","index_hash":"{}"}}"#,
            frame, draw_idx, vertex_count, index_count,
            texture_mode, vh, ih
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_fnv1a_empty() {
        assert_eq!(hash_fnv1a(b""), 0xcbf29ce484222325);
    }

    #[test]
    fn test_hash_fnv1a_hello() {
        let hash = hash_fnv1a(b"Hello, World!");
        assert_eq!(hash_to_hex(hash), "6ef05bd7cc857c54");
    }

    #[test]
    fn test_hash_to_hex() {
        assert_eq!(hash_to_hex(0xDEADBEEF12345678), "deadbeef12345678");
        assert_eq!(hash_to_hex(0x0000000000000001), "0000000000000001");
    }
}
