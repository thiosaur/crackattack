//! Non-blocking screenshot capture.
//!
//! The capture is split across frames to avoid blocking the render thread:
//! - Frame N: GPU copy texture → staging buffer, request async mapping
//! - Frame N+1+: Poll mapping status, extract pixels, spawn thread for PNG encoding
//! - Later frames: each save thread is tracked until its result is known.
//!   `is_idle()` is true only once every PNG is written or has failed, so the exit path can
//!   drain all pending writes. Only the GPU readback is exclusive: a slow write does not
//!   block the next capture of the timeline.
//!
//! Two targets:
//! - **Manual** (F12 in game): timestamped name in `<Pictures>/Crack Attack/`.
//! - **Named** (miniscript `capture <name>`): deterministic name `<script>-<name>.png` in the
//!   campaign folder (`session:qa.capture_dir`, default `<Pictures>/Crack Attack/qa/`).
//!   An existing file is a failure, because a stale file would give a false pass when
//!   comparing by name. Outcomes are logged for the end-of-script summary.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;

/// Capture request, carried by `DebugState` and consumed when the frame is submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotRequest {
    /// Manual capture (F12): timestamped name, root folder.
    Manual,
    /// Miniscript capture: final file name (`<script>-<name>.png`), campaign folder.
    Named(String),
}

/// Resolved save target.
enum Target {
    Timestamped,
    Path(PathBuf),
}

/// GPU readback in progress. Only one at a time: it is the only exclusive phase.
struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    /// Format of the texture being read. It decides the swizzle (B,G,R,A to R,G,B,A, or a
    /// direct copy). The surface can be Rgba8 (possible on Vulkan/DX12), so BGRA must not be
    /// assumed.
    format: wgpu::TextureFormat,
    ready: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    /// Whether map_async has been called (deferred until after queue.submit).
    mapped: bool,
    target: Target,
}

/// Result of a save thread: the written path, or an error message.
type SaveResult = Result<PathBuf, String>;
/// Save thread in flight, with the logged name (named captures only).
type SaveInFlight = (std::thread::JoinHandle<SaveResult>, Option<String>);

/// Async screenshot capture manager.
///
/// Two decoupled phases: the GPU readback (exclusive, 1-2 frames) and the PNG save threads
/// (independent, tracked in parallel until the disk result). A slow write therefore does not
/// block the next capture of the timeline.
pub struct ScreenshotCapture {
    /// Campaign folder for named captures (`session:qa.capture_dir`), set at bootstrap.
    capture_dir: Option<String>,
    readback: Option<Readback>,
    /// Save threads in flight (with the logged name for named captures).
    saves: Vec<SaveInFlight>,
    /// Outcomes of named captures (file name to written path or error), for the end-of-script
    /// summary and the exit code. Manual captures are not listed.
    outcomes: Vec<(String, Result<PathBuf, String>)>,
}

impl ScreenshotCapture {
    pub fn new(capture_dir: Option<String>) -> Self {
        Self { capture_dir, readback: None, saves: Vec::new(), outcomes: Vec::new() }
    }

    /// True when no work is pending, neither the readback nor any disk write.
    pub fn is_idle(&self) -> bool {
        self.readback.is_none() && self.saves.is_empty()
    }

    /// Outcomes of named captures (campaign summary).
    pub fn outcomes(&self) -> &[(String, Result<PathBuf, String>)] {
        &self.outcomes
    }

    /// Records a named-capture failure that happened before any GPU work
    /// (request overwritten, exit before rendering, etc.).
    pub fn record_failure(&mut self, filename: String, error: String) {
        log::error!("Screenshot \"{filename}\": {error}");
        self.outcomes.push((filename, Err(error)));
    }

    /// Initiate a GPU readback copy. Appends the copy command to `encoder`.
    /// The caller must still submit the encoder via `queue.submit()`.
    ///
    /// For a named capture, this resolves the path (creating the campaign folder), refuses an
    /// existing file, and records any failure.
    pub fn begin_capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        request: ScreenshotRequest,
    ) {
        if self.readback.is_some() {
            // Only the readback is exclusive (1-2 frames). Writes in flight do not block
            // a new capture.
            match request {
                ScreenshotRequest::Manual => {
                    log::warn!("Screenshot already in progress, ignoring request");
                }
                ScreenshotRequest::Named(name) => {
                    // Never drop a named capture silently: a lost one is a failure.
                    self.record_failure(name, "previous capture readback still in progress".to_string());
                }
            }
            return;
        }

        let target = match &request {
            ScreenshotRequest::Manual => Target::Timestamped,
            ScreenshotRequest::Named(name) => match named_capture_path(self.capture_dir.as_deref(), name) {
                Ok(path) => Target::Path(path),
                Err(e) => {
                    self.record_failure(name.clone(), e);
                    return;
                }
            },
        };

        let size = texture.size();
        let bytes_per_row = (size.width * 4).div_ceil(256) * 256;

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot_staging"),
            size: (bytes_per_row * size.height) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );

        self.readback = Some(Readback {
            buffer, width: size.width, height: size.height,
            bytes_per_row,
            ready: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
            mapped: false,
            target,
            format: texture.format(),
        });

        log::info!("Screenshot capture initiated ({}x{})", size.width, size.height);
    }

    /// Poll the state machine. Call once per frame after `device.poll(Poll)`.
    pub fn poll(&mut self) {
        // 1. Is a GPU readback in progress?
        if let Some(mut rb) = self.readback.take() {
            // First poll after queue.submit: issue the map_async request
            if !rb.mapped {
                let ready_cb = Arc::clone(&rb.ready);
                let failed_cb = Arc::clone(&rb.failed);
                rb.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_err() {
                        failed_cb.store(true, Ordering::Release);
                    }
                    ready_cb.store(true, Ordering::Release);
                });
                rb.mapped = true;
                self.readback = Some(rb);
            } else if rb.failed.load(Ordering::Acquire) {
                if let Target::Path(p) = &rb.target {
                    self.record_failure(file_name_of(p), "GPU buffer mapping failed".to_string());
                } else {
                    log::error!("Screenshot: GPU buffer mapping failed");
                }
                // buffer dropped, readback = None
            } else if !rb.ready.load(Ordering::Acquire) {
                self.readback = Some(rb); // not ready yet, try again next poll
            } else {
                // Data is ready — extract pixels
                let (width, height, bytes_per_row) = (rb.width, rb.height, rb.bytes_per_row);
                let slice = rb.buffer.slice(..);
                let data = slice.get_mapped_range();

                let rgba = extract_rgba(&data, width, height, bytes_per_row,
                    needs_rb_swap(rb.format));
                drop(data);
                rb.buffer.unmap();

                // Encode and write on a thread, tracked in `saves` until it finishes.
                let (path, named) = match rb.target {
                    Target::Timestamped => (None, None),
                    Target::Path(p) => {
                        let n = file_name_of(&p);
                        (Some(p), Some(n))
                    }
                };
                match std::thread::Builder::new()
                    .name("screenshot-save".into())
                    .spawn(move || save_screenshot_png(width, height, rgba, path))
                {
                    Ok(handle) => self.saves.push((handle, named)),
                    Err(e) => {
                        if let Some(n) = named {
                            self.record_failure(n, format!("spawning the writer thread: {e}"));
                        } else {
                            log::error!("Screenshot: failed to spawn save thread: {}", e);
                        }
                    }
                }
            }
        }

        // 2. Collect finished writes. The others stay in flight.
        let mut still_flying = Vec::new();
        for (handle, named) in self.saves.drain(..) {
            if !handle.is_finished() {
                still_flying.push((handle, named));
                continue;
            }
            let result = handle
                .join()
                .unwrap_or_else(|_| Err("writer thread panicked".to_string()));
            match (&result, named) {
                (_, Some(n)) => self.outcomes.push((n, result)),
                (Ok(p), None) => log::info!("Screenshot saved: {}", p.display()),
                (Err(e), None) => log::error!("Screenshot: {e}"),
            }
        }
        self.saves = still_flying;
    }
}

fn file_name_of(p: &std::path::Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Campaign folder for named captures: `session:qa.capture_dir` (relative to the cwd),
/// otherwise `<Pictures>/Crack Attack/qa/`.
fn qa_capture_dir(capture_dir: Option<&str>) -> Result<PathBuf, String> {
    if let Some(dir) = capture_dir {
        return Ok(PathBuf::from(dir));
    }
    screenshot_dir()
        .map(|d| d.join("qa"))
        .ok_or_else(|| "capture folder not found".to_string())
}

/// Resolves the path of a named capture: creates the folder, refuses an existing file.
fn named_capture_path(capture_dir: Option<&str>, filename: &str) -> Result<PathBuf, String> {
    named_capture_path_in(&qa_capture_dir(capture_dir)?, filename)
}

/// Core of the resolution, testable without a session: creates `dir`, refuses an existing file.
fn named_capture_path_in(dir: &std::path::Path, filename: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("creating folder {}: {e}", dir.display()))?;
    let path = dir.join(filename);
    // Atomic reservation: `create_new` ensures two processes sharing the folder cannot
    // accept the same name. The final write overwrites the empty file. An aborted run can
    // leave it behind, but a campaign runs in an empty folder anyway.
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(_) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "{} already exists: a campaign runs in an empty folder \
             (a stale file would skew the comparison by name)",
            path.display()
        )),
        Err(e) => Err(format!("reserving {}: {e}", path.display())),
    }
}

/// Verdict of a capture campaign. It succeeds only if every capture planned by the timeline
/// was requested and written without error. A campaign cut short before its first capture
/// (max-frames, gate timeout) must fail, not pass silently.
pub fn campaign_verdict(
    planned: usize,
    demanded: u32,
    outcomes: &[(String, Result<PathBuf, String>)],
) -> Result<usize, String> {
    let errors: Vec<String> = outcomes.iter()
        .filter_map(|(n, r)| r.as_ref().err().map(|e| format!("{n}: {e}")))
        .collect();
    if !errors.is_empty() {
        return Err(format!("failed capture(s): {}", errors.join(" ; ")));
    }
    let written = outcomes.iter().filter(|(_, r)| r.is_ok()).count();
    if demanded as usize != planned || written != planned {
        return Err(format!(
            "planned={planned} requested={demanded} written={written}: campaign truncated or incomplete"
        ));
    }
    Ok(written)
}

fn screenshot_dir() -> Option<PathBuf> {
    let dir = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .map(|d| d.join("Crack Attack"))?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("Screenshot: failed to create directory {}: {}", dir.display(), e);
        return None;
    }
    Some(dir)
}

/// True if the format stores channels as B,G,R,A (the PNG expects R,G,B,A).
/// Other surface formats (Rgba8, …) are copied as they are.
fn needs_rb_swap(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

/// Removes the row padding (`bytes_per_row` is aligned to 256) and swaps R and B if needed.
fn extract_rgba(data: &[u8], width: u32, height: u32, bytes_per_row: u32, swap_rb: bool) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * bytes_per_row) as usize;
        for col in 0..width {
            let i = start + (col * 4) as usize;
            if swap_rb {
                rgba.push(data[i + 2]);
                rgba.push(data[i + 1]);
                rgba.push(data[i]);
                rgba.push(data[i + 3]);
            } else {
                rgba.extend_from_slice(&data[i..i + 4]);
            }
        }
    }
    rgba
}

/// Encodes and writes the PNG. `target: None` uses a timestamped name in the root folder
/// (manual capture); `Some(path)` is an already resolved path (named capture).
fn save_screenshot_png(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    target: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = match target {
        Some(p) => p,
        None => {
            let dir = screenshot_dir().ok_or("cannot determine save directory".to_string())?;
            // The name is the epoch time in milliseconds. Two captures in the same
            // millisecond would overwrite each other, which is acceptable (likely the same image).
            use std::time::{SystemTime, UNIX_EPOCH};
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            dir.join(format!("crackattack_{}.png", millis))
        }
    };

    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| format!("invalid image dimensions {width}x{height}"))?;

    img.save(&path)
        .map(|()| path.clone())
        .map_err(|e| format!("failed to save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both conversions, with row padding (`bytes_per_row` > width*4).
    #[test]
    fn extract_rgba_swaps_bgra_and_strips_padding() {
        // 2x1 BGRA pixels: (B=1,G=2,R=3,A=4), (B=5,G=6,R=7,A=8) + 8 bytes of padding.
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE];
        let out = extract_rgba(&data, 2, 1, 16, true);
        assert_eq!(out, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn extract_rgba_copies_rgba_untouched() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE, 0xEE];
        let out = extract_rgba(&data, 2, 1, 16, false);
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn needs_rb_swap_only_for_bgra() {
        assert!(needs_rb_swap(wgpu::TextureFormat::Bgra8UnormSrgb));
        assert!(!needs_rb_swap(wgpu::TextureFormat::Rgba8UnormSrgb));
    }

    #[test]
    fn named_capture_path_creates_dir_and_refuses_existing() {
        let guard = crate::platform::test_support::TestDir::new("capture");
        let dir = guard.path().to_path_buf();
        let p = named_capture_path_in(&dir, "a.png").expect("creation + resolution");
        assert!(dir.is_dir(), "the campaign folder is created");
        std::fs::write(&p, b"x").unwrap();
        let err = named_capture_path_in(&dir, "a.png").unwrap_err();
        assert!(err.contains("already exists"), "stale file refused (false green): {err}");
    }

    /// A campaign cut short before its first capture (0 requested, 0 written) must fail
    /// whenever the timeline planned any.
    #[test]
    fn campaign_verdict_fails_on_truncated_run() {
        assert!(campaign_verdict(5, 0, &[]).is_err(), "truncated = failure");
        assert!(campaign_verdict(0, 0, &[]).is_ok(), "script without capture = OK");
        let ok = |n: &str| (n.to_string(), Ok(PathBuf::from(n)));
        let outcomes = [ok("a.png"), ok("b.png")];
        assert_eq!(campaign_verdict(2, 2, &outcomes), Ok(2), "complete = success");
        assert!(campaign_verdict(3, 2, &outcomes).is_err(), "a planned capture not reached = failure");
        let mixed = [ok("a.png"), ("b.png".to_string(), Err("disk full".to_string()))];
        assert!(campaign_verdict(2, 2, &mixed).is_err(), "a write error = failure");
    }

    /// Save threads are collected: their results (successes and failures) end up in
    /// `outcomes`, and `is_idle()` is true only once every thread is collected.
    #[test]
    fn poll_harvests_save_threads_to_outcomes() {
        let mut sc = ScreenshotCapture::new(None);
        sc.saves.push((
            std::thread::spawn(|| Ok(PathBuf::from("ok.png"))),
            Some("ok.png".to_string()),
        ));
        sc.saves.push((
            std::thread::spawn(|| Err("simulated failure".to_string())),
            Some("ko.png".to_string()),
        ));
        assert!(!sc.is_idle(), "writes in flight -> not idle");
        let start = std::time::Instant::now();
        while !sc.is_idle() {
            assert!(start.elapsed().as_secs() < 5, "harvest blocked");
            sc.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut names: Vec<&str> = sc.outcomes().iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["ko.png", "ok.png"]);
        assert!(sc.outcomes().iter().find(|(n, _)| n == "ko.png").unwrap().1.is_err());
        assert!(campaign_verdict(2, 2, sc.outcomes()).is_err(), "the simulated failure fails the campaign");
    }
}
