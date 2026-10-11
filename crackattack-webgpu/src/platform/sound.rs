//! Audio playback — SFX (WAV) and music (OGG) via rodio.
//!
//! SFX are decoded at load time and cloned per play.
//! Music is streamed from disk via a dedicated Sink. During a game, when an adaptive ladder
//! (`music/adaptive/rungs.json`) is present, the game track is the adaptive mixer instead of
//! `game.ogg` (see `adaptive_music`).

use std::path::{Path, PathBuf};
use std::io::BufReader;
use std::fs::File;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use crackattack_core::gameplay_event::GameplayEvent;

use super::adaptive_music::{plan::RUNGS_FILE, AdaptiveSession};

/// Audio output, opened in the background.
///
/// cpal's CoreAudio backend can take seconds, or block indefinitely, to create a stream on some
/// macOS systems, so the device is opened on its own thread and the game never waits for it:
/// [`AudioOutput::poll`] picks the handle up once it is ready. The `OutputStream` itself stays on
/// that thread (it is `!Send`), parked for the process lifetime to keep the device open.
pub struct AudioOutput {
    handle: Option<OutputStreamHandle>,
    pending: Option<std::sync::mpsc::Receiver<Option<OutputStreamHandle>>>,
}

impl AudioOutput {
    /// Starts opening the default output device (or the first one that works) and returns at once.
    pub fn new() -> Self {
        log::info!("Opening audio output device...");
        let (tx, rx) = std::sync::mpsc::channel::<Option<OutputStreamHandle>>();
        std::thread::Builder::new()
            .name("audio-init".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let host = rodio::cpal::default_host();
                let open_device = |device: &rodio::cpal::Device| OutputStream::try_from_device(device);
                let name_of = |device: &rodio::cpal::Device| device.name().unwrap_or_else(|e| format!("unnamed ({e})"));

                // The default device first. Enumerating every output costs 100 to 500 ms on
                // macOS, so it is only done when the default device is missing or fails.
                let default_device = host.default_output_device();
                let default_name = default_device.as_ref().map(name_of);
                let stream = match &default_device {
                    Some(device) => match open_device(device) {
                        Ok(stream) => Some(stream),
                        Err(e) => {
                            log::warn!("Failed to open default output device: {}", e);
                            None
                        }
                    },
                    None => {
                        log::warn!("No default output device available");
                        None
                    }
                }
                .or_else(|| {
                    let devices = match host.output_devices() {
                        Ok(devices) => devices,
                        Err(e) => {
                            log::warn!("Failed to enumerate output devices: {}", e);
                            return None;
                        }
                    };
                    for device in devices {
                        let name = name_of(&device);
                        if Some(&name) == default_name.as_ref() {
                            continue; // already tried
                        }
                        log::info!("Trying fallback output device: {}", name);
                        match open_device(&device) {
                            Ok(stream) => return Some(stream),
                            Err(e) => log::warn!("Fallback output device failed: {}", e),
                        }
                    }
                    None
                });
                if stream.is_some() {
                    log::info!("Audio output device opened in {} ms", started.elapsed().as_millis());
                }

                match stream {
                    Some((_stream, handle)) => {
                        // Keep the stream (and this thread) only while the game listens: if
                        // the receiver is gone, parking would leak a thread and an open device.
                        if tx.send(Some(handle)).is_ok() {
                            loop { std::thread::park(); }
                        }
                    }
                    None => {
                        log::warn!("No usable audio output device found. Sound disabled.");
                        let _ = tx.send(None);
                    }
                }
            })
            .ok();
        Self { handle: None, pending: Some(rx) }
    }

    /// The output handle, once the device is open. Cheap: call it as often as needed.
    pub fn poll(&mut self) -> Option<&OutputStreamHandle> {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(handle) => {
                    self.handle = handle;
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        self.handle.as_ref()
    }

    /// Waits up to `timeout` for the device (`--sound-preview`, before the window opens).
    pub fn wait(&mut self, timeout: std::time::Duration) -> Option<&OutputStreamHandle> {
        if let Some(rx) = self.pending.take() {
            match rx.recv_timeout(timeout) {
                Ok(handle) => self.handle = handle,
                Err(_) => self.pending = Some(rx),
            }
        }
        self.handle.as_ref()
    }
}

/// Pre-decoded audio buffer. Stores PCM samples (i16) decoded at load time.
/// Playback clones the samples vec — no re-decoding per play.
#[derive(Clone)]
struct SfxBuffer {
    samples: Vec<i16>,
    channels: u16,
    sample_rate: u32,
}

impl SfxBuffer {
    fn load(path: &Path) -> Option<Self> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Failed to read audio file {}: {}", path.display(), e);
                return None;
            }
        };
        let decoder = match Decoder::new(std::io::Cursor::new(bytes)) {
            Ok(d) => d,
            Err(_) => {
                log::warn!("Failed to decode audio: {}", path.display());
                return None;
            }
        };
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let samples: Vec<i16> = decoder.collect();
        Some(Self { samples, channels, sample_rate })
    }

    fn source(&self) -> rodio::buffer::SamplesBuffer<i16> {
        rodio::buffer::SamplesBuffer::new(self.channels, self.sample_rate, self.samples.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicTrack {
    Prelude,
    Game,
    GameOver,
    YouWin,
}

impl MusicTrack {
    fn filename(self) -> &'static str {
        match self {
            MusicTrack::Prelude => "prelude.ogg",
            MusicTrack::Game => "game.ogg",
            MusicTrack::GameOver => "gameover.ogg",
            MusicTrack::YouWin => "youwin.ogg",
        }
    }

    /// The game music loops (matches the original 1.1.14). Prelude, game over and you win play once.
    /// The game and menu music loop; game over and you win play once, then chain to the menu
    /// music.
    fn loops(self) -> bool {
        matches!(self, MusicTrack::Game | MusicTrack::Prelude)
    }
}

/// Per-tick sound batch for deduplication.
/// Each slot stores the max volume requested this tick.
#[derive(Default)]
struct SoundBatch {
    block_fallen: f32,
    garbage_fallen: f32,
    combo: f32,
    garbage_shatter: f32,
    swap: f32,
    garbage_drop: f32,
    chain_low: f32,
    chain_high: f32,
    start_game_music: bool,
}

/// Tracks recently played sounds for overlap/repeat attenuation.
/// Each entry is (tick_played, duration_in_ticks).
#[derive(Default)]
struct SoundHistory {
    /// Tick at which each sound last played, 0 = never
    block_fallen_tick: u32,
    garbage_fallen_tick: u32,
    combo_tick: u32,
}

impl SoundHistory {
    /// Returns true if the given sound is still "ringing" at `current_tick`.
    fn is_recent(&self, last_tick: u32, duration: u32, current_tick: u32) -> bool {
        last_tick > 0 && current_tick.saturating_sub(last_tick) < duration
    }
}

pub struct SoundManager {
    output: AudioOutput,
    /// Track asked for before the device was open: started as soon as it is.
    track_waiting_for_output: Option<MusicTrack>,

    sfx_block_fallen: Option<SfxBuffer>,
    sfx_swap: Option<SfxBuffer>,
    sfx_combo: Option<SfxBuffer>,
    sfx_garbage_drop: Option<SfxBuffer>,
    sfx_garbage_shatter: Option<SfxBuffer>,
    sfx_chain_low: Option<SfxBuffer>,  // chains 2-3
    sfx_chain_high: Option<SfxBuffer>, // chains 4+
    sfx_garbage_fallen: Option<SfxBuffer>,
    sfx_menu_navigate: Option<SfxBuffer>,
    sfx_menu_confirm: Option<SfxBuffer>,
    /// Countdown: 3, 2, 1, Fight!
    sfx_countdown: [Option<SfxBuffer>; 4],

    music_sink: Option<Sink>,
    music_dir: PathBuf,
    /// Music folder to test (`--music DIR`): a track found here wins over the default.
    music_override: Option<PathBuf>,
    /// Fade-out in progress: (ticks left, total ticks).
    music_fade: Option<(u32, u32)>,
    current_track: Option<MusicTrack>,
    /// Adaptive music ladder folder (`music/adaptive/`, or `--rungs DIR`).
    rungs_dir: PathBuf,
    /// `--music-debug`: logs the stress and the rung every second.
    music_debug: bool,
    /// Adaptive music session for the current game (the Game track is then the adaptive mixer).
    adaptive: Option<AdaptiveSession>,

    sfx_volume: f32,
    music_volume: f32,

    // Mute toggles (independent of volume — user can toggle off then back on
    // without losing their volume setting).
    pub sfx_enabled: bool,
    pub music_enabled: bool,


    history: SoundHistory,
    tick_counter: u32,

    /// Count of ×4+ chains achieved this session. Once >= 2,
    /// chain_low plays for ×2-3 (differentiated feedback).
    /// Before that, all chains use chain_high (rewarding).
    high_chain_count: u32,
}

/// The 14 sound effects, one per event: expected file name and role (shown by
/// `--sound-preview`; the roles are terminal text only). A replacement sound set
/// (`--sounds DIR`) uses these names. Each file in the default set is a distinct sound.
pub const SFX_FILES: [(&str, &str); 14] = [
    ("swap.wav", "swap of two blocks"),
    ("block_fallen.wav", "block landing"),
    ("garbage_fallen.wav", "garbage landing"),
    ("combo.wav", "combo (3 or more blocks at once)"),
    ("chain_low.wav", "short chain"),
    ("chain.wav", "long chain"),
    ("garbage_drop.wav", "garbage received"),
    ("garbage_shatter.wav", "garbage shattering"),
    ("menu_navigate.wav", "menu navigation"),
    ("menu_confirm.wav", "menu confirmation"),
    ("countdown_3.wav", "countdown: 3"),
    ("countdown_2.wav", "countdown: 2"),
    ("countdown_1.wav", "countdown: 1"),
    ("countdown_go.wav", "countdown: Fight!"),
];

/// Loads a sound effect: first from the replacement sound set if one is given and contains
/// the file, otherwise from the default sounds. Also returns the folder that was used.
fn load_sfx(name: &str, default_dir: &Path, override_dir: Option<&Path>) -> (Option<SfxBuffer>, PathBuf) {
    if let Some(dir) = override_dir {
        let path = dir.join(name);
        if path.is_file() {
            return (SfxBuffer::load(&path), dir.to_path_buf());
        }
    }
    (SfxBuffer::load(&default_dir.join(name)), default_dir.to_path_buf())
}

/// `--sound-preview`: plays the 14 sound effects one by one (role and origin printed to the
/// terminal) before the window opens. Blocks for a few seconds.
pub fn preview_sfx(audio_output: &mut AudioOutput, data_dir: &Path, override_dir: Option<&Path>) {
    let Some(handle) = audio_output.wait(std::time::Duration::from_secs(5)).cloned() else {
        eprintln!("Sound preview impossible: no audio output");
        return;
    };
    let default_dir = data_dir.join("sounds");
    eprintln!("Sound effects preview{}:", override_dir
        .map(|d| format!(" (sound set: {})", d.display())).unwrap_or_default());
    for (name, role) in SFX_FILES {
        let (buf, dir) = load_sfx(name, &default_dir, override_dir);
        let origin = if Some(dir.as_path()) == override_dir { "tested set" } else { "default" };
        let Some(buf) = buf else {
            eprintln!("  {name:<20} {role} — UNREADABLE ({origin})");
            continue;
        };
        let secs = buf.samples.len() as f32 / (buf.channels.max(1) as f32 * buf.sample_rate.max(1) as f32);
        eprintln!("  {name:<20} {role} — {origin}, {secs:.2} s");
        if let Err(e) = handle.play_raw(buf.source().amplify(0.49).convert_samples()) {
            log::warn!("Failed to play SFX: {}", e);
        }
        std::thread::sleep(std::time::Duration::from_secs_f32(secs.min(3.0) + 0.6));
    }
}

impl SoundManager {
    /// Create a new SoundManager from a pre-initialized AudioOutput.
    /// Loads all SFX from `data_dir/sounds/`, or, file by file, from `sounds_override`
    /// when given (`--sounds DIR`, to test a sound set). Music files are in `data_dir/music/`.
    pub fn new(audio_output: AudioOutput, data_dir: &Path, sounds_override: Option<&Path>) -> Self {

        let sounds_dir = data_dir.join("sounds");
        let music_dir = data_dir.join("music");

        let load = |name: &str| -> Option<SfxBuffer> {
            let (buf, dir) = load_sfx(name, &sounds_dir, sounds_override);
            if buf.is_some() {
                if sounds_override.is_some() {
                    log::info!("SFX {} ← {}", name, dir.display());
                } else {
                    log::debug!("Loaded SFX: {}", name);
                }
            }
            buf
        };

        let mgr = SoundManager {
            output: audio_output,
            track_waiting_for_output: None,
            sfx_block_fallen: load("block_fallen.wav"),
            sfx_swap: load("swap.wav"),
            sfx_combo: load("combo.wav"),
            sfx_garbage_drop: load("garbage_drop.wav"),
            sfx_garbage_shatter: load("garbage_shatter.wav"),
            sfx_chain_low: load("chain_low.wav"),
            sfx_chain_high: load("chain.wav"),
            sfx_garbage_fallen: load("garbage_fallen.wav"),
            sfx_menu_navigate: load("menu_navigate.wav"),
            sfx_menu_confirm: load("menu_confirm.wav"),
            sfx_countdown: [
                load("countdown_3.wav"), load("countdown_2.wav"),
                load("countdown_1.wav"), load("countdown_go.wav"),
            ],
            music_sink: None,
            rungs_dir: music_dir.join("adaptive"),
            music_debug: false,
            adaptive: None,
            music_dir,
            music_override: None,
            music_fade: None,
            current_track: None,
            sfx_volume: 0.7,
            music_volume: 0.5,
            history: SoundHistory::default(),
            tick_counter: 0,
            high_chain_count: 0,
            sfx_enabled: true,
            music_enabled: false,
        };

        let sfx_count = [
            &mgr.sfx_block_fallen,
            &mgr.sfx_swap, &mgr.sfx_combo, &mgr.sfx_garbage_drop, &mgr.sfx_garbage_shatter,
            &mgr.sfx_chain_low, &mgr.sfx_chain_high, &mgr.sfx_garbage_fallen,
            &mgr.sfx_menu_navigate, &mgr.sfx_menu_confirm,
        ].into_iter().chain(&mgr.sfx_countdown).filter(|s| s.is_some()).count();
        log::info!("Sound system initialized: {}/{} SFX loaded", sfx_count, SFX_FILES.len());

        mgr
    }

    /// Set SFX volume from slider value (0.0–1.0).
    /// Quadratic curve applied at playback time.
    pub fn set_sfx_volume(&mut self, slider: f32) {
        self.sfx_volume = slider.clamp(0.0, 1.0);
    }

    /// Whether music tracks are present. The public repository and its packages do not ship
    /// any (licensing), in which case the Music settings are hidden.
    pub fn music_available(&self) -> bool {
        self.music_path(MusicTrack::Prelude).is_file()
    }

    /// `--music DIR`: tracks found in DIR replace the default ones.
    pub fn set_music_override(&mut self, dir: Option<PathBuf>) {
        self.music_override = dir;
    }

    /// `--rungs DIR`: adaptive music ladder to test, used instead of `music/adaptive/`.
    pub fn set_rungs_dir(&mut self, dir: Option<PathBuf>) {
        if let Some(dir) = dir {
            self.rungs_dir = dir;
        }
    }

    /// `--music-debug`: logs the stress and the rung.
    pub fn set_music_debug(&mut self, on: bool) {
        self.music_debug = on;
    }

    /// File of a track: from the test folder if it has one (`.ogg`, or a `.wav` of the same
    /// name), otherwise the default.
    fn music_path(&self, track: MusicTrack) -> PathBuf {
        let name = track.filename();
        self.music_override.as_ref()
            .and_then(|d| [d.join(name), d.join(name).with_extension("wav")]
                .into_iter().find(|p| p.is_file()))
            .unwrap_or_else(|| self.music_dir.join(name))
    }

    /// Set music volume from slider value (0.0–1.0).
    /// Applies immediately to the current music sink.
    pub fn set_music_volume(&mut self, slider: f32) {
        self.music_volume = slider.clamp(0.0, 1.0);
        if let Some(ref sink) = self.music_sink {
            let gain = self.music_volume * self.music_volume;
            sink.set_volume(gain);
        }
    }

    /// Play menu navigate sound
    pub fn play_ui_navigate(&self) {
        self.play_sfx(&self.sfx_menu_navigate, 0.10);
    }

    /// Play menu confirm sound
    pub fn play_ui_confirm(&self) {
        self.play_sfx(&self.sfx_menu_confirm, 0.40);
    }

    fn play_sfx(&self, buf: &Option<SfxBuffer>, volume: f32) {
        if !self.sfx_enabled { return; }
        // `poll` needs `&mut self`; the handle is read as left by the last `tick_music`.
        let Some(handle) = self.output.handle.as_ref() else { return };
        if let Some(ref sfx) = buf {
            // Perceptual curve (quadratic) on the global slider only.
            // Per-sound artistic volumes stay linear.
            let global_gain = self.sfx_volume * self.sfx_volume;
            let vol = (volume * global_gain).clamp(0.0, 1.0);
            let source = sfx.source().amplify(vol);
            if let Err(e) = handle.play_raw(source.convert_samples()) {
                log::warn!("Failed to play SFX: {}", e);
            }
        }
    }

    /// Process gameplay events for audio.
    /// Collects sounds into a batch, deduplicates per tick, then plays each once.
    pub fn process_events(&mut self, events: &[GameplayEvent]) {
        let mut batch = SoundBatch::default();

        // Volume reference (all files normalized to -1 dBFS peak):
        //   Gameplay-critical (combo, chain) : 0.50 - 0.60
        //   Feedback (swap, countdown)       : 0.30 - 0.40
        //   Ambient (landing, drop)          : 0.15 - 0.25
        for event in events {
            match event {
                GameplayEvent::CountdownTick(n) => {
                    let vol = match n {
                        3 => 0.25,
                        2 => 0.35,
                        _ => 0.45,
                    };
                    let slot = match n { 3 => 0, 2 => 1, _ => 2 };
                    self.play_sfx(&self.sfx_countdown[slot], vol);
                }
                GameplayEvent::CountdownGo => {
                    self.play_sfx(&self.sfx_countdown[3], 0.55);
                    batch.start_game_music = true;
                }
                GameplayEvent::BlockLanded(_) => {
                    batch.block_fallen = batch.block_fallen.max(0.12);
                }
                GameplayEvent::EliminationStarted(_) => {
                    batch.combo = batch.combo.max(0.35);
                }
                GameplayEvent::GarbageLanded(_) => {
                    batch.garbage_fallen = batch.garbage_fallen.max(0.20);
                }
                GameplayEvent::GarbageShattering(_) => {
                    batch.garbage_shatter = batch.garbage_shatter.max(0.55);
                }
                GameplayEvent::SwapPerformed => {
                    batch.swap = batch.swap.max(0.05);
                }
                GameplayEvent::GarbageDropped { .. } => {
                    batch.garbage_drop = batch.garbage_drop.max(0.35);
                }
                GameplayEvent::ChainProgressed(e) => {
                    if e.chain_level >= 4 {
                        self.high_chain_count += 1;
                        batch.chain_high = batch.chain_high.max(0.60);
                    } else if self.high_chain_count >= 2 {
                        // Differentiated: low sound for ×2-3 once player has proven skill
                        batch.chain_low = batch.chain_low.max(0.60);
                    } else {
                        // Beginner: always rewarding high sound
                        batch.chain_high = batch.chain_high.max(0.60);
                    }
                }
                GameplayEvent::BlockAwaking(_)
                | GameplayEvent::CreepShift
                | GameplayEvent::GameOver { .. } => {}
            }
        }

        self.tick_counter += 1;
        let tick = self.tick_counter;

        // Priorities: garbage shatter suppresses combo, any shatter suppresses swap
        if batch.garbage_shatter > 0.0 {
            batch.combo = 0.0;
        }
        if batch.combo > 0.0 || batch.garbage_shatter > 0.0 {
            batch.swap = 0.0;
        }
        // Also suppress swap if a shatter sound played recently (within ~10 ticks)
        if batch.swap > 0.0 && self.history.is_recent(self.history.combo_tick, 10, tick) {
            batch.swap = 0.0;
        }

        // Attenuation: reduce repeated landing sounds (garbage desync cascades)
        // If a landing sound played within 10 ticks (~200ms), halve the volume
        if batch.block_fallen > 0.0 && self.history.is_recent(self.history.block_fallen_tick, 10, tick) {
            batch.block_fallen *= 0.5;
        }
        if batch.garbage_fallen > 0.0 && self.history.is_recent(self.history.garbage_fallen_tick, 10, tick) {
            batch.garbage_fallen *= 0.5;
        }

        // Play each sound at most once per tick, at the max volume seen
        // Countdown sounds played directly in event loop (not batched)
        if batch.combo > 0.0 {
            self.play_sfx(&self.sfx_combo, batch.combo);
            self.history.combo_tick = tick;
        }
        if batch.garbage_shatter > 0.0 {
            self.play_sfx(&self.sfx_garbage_shatter, batch.garbage_shatter);
            self.history.combo_tick = tick; // shatter counts as combo for suppression
        }
        if batch.block_fallen > 0.0 {
            self.play_sfx(&self.sfx_block_fallen, batch.block_fallen);
            self.history.block_fallen_tick = tick;
        }
        if batch.garbage_fallen > 0.0 {
            self.play_sfx(&self.sfx_garbage_fallen, batch.garbage_fallen);
            self.history.garbage_fallen_tick = tick;
        }
        if batch.swap > 0.0      { self.play_sfx(&self.sfx_swap, batch.swap); }
        if batch.garbage_drop > 0.0 {
            self.play_sfx(&self.sfx_garbage_drop, batch.garbage_drop);
        }
        if batch.chain_low > 0.0  { self.play_sfx(&self.sfx_chain_low, batch.chain_low); }
        if batch.chain_high > 0.0 { self.play_sfx(&self.sfx_chain_high, batch.chain_high); }
        if batch.start_game_music { self.play_music(MusicTrack::Game); }
    }

    /// Play a music track (stops any current track).
    pub fn play_music(&mut self, track: MusicTrack) {
        // Stop current music
        self.stop_music();
        if !self.music_enabled { return; }
        let Some(handle) = self.output.poll().cloned() else {
            self.track_waiting_for_output = Some(track);
            return;
        };

        if track == MusicTrack::Game && self.start_adaptive(&handle) {
            return;
        }

        let path = self.music_path(track);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("Failed to open music {}: {}", path.display(), e);
                return;
            }
        };

        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to decode music {}: {}", path.display(), e);
                return;
            }
        };

        match Sink::try_new(&handle) {
            Ok(sink) => {
                let gain = self.music_volume * self.music_volume;
                sink.set_volume(gain);
                sink.append(source);
                self.music_sink = Some(sink);
                self.current_track = Some(track);
                log::info!("Playing music: {}", path.display());
            }
            Err(e) => {
                log::warn!("Failed to create music sink: {}", e);
            }
        }
    }

    /// Starts the adaptive game music if a valid ladder is present. Returns `false` when the
    /// caller should play `game.ogg` instead.
    fn start_adaptive(&mut self, handle: &OutputStreamHandle) -> bool {
        if !self.rungs_dir.join(RUNGS_FILE).is_file() {
            log::debug!("No adaptive music ladder in {}", self.rungs_dir.display());
            return false;
        }
        let plan = match AdaptiveSession::load(&self.rungs_dir) {
            Ok(plan) => plan,
            Err(e) => {
                log::warn!("Adaptive music ignored, falling back to game.ogg: {e}");
                return false;
            }
        };
        let sink = match Sink::try_new(handle) {
            Ok(sink) => sink,
            Err(e) => {
                log::warn!("Failed to create music sink: {}", e);
                return false;
            }
        };
        let rungs = plan.rungs.len();
        let (session, source) = AdaptiveSession::start(plan, self.music_debug);
        sink.set_volume(self.music_volume * self.music_volume);
        sink.append(source);
        self.music_sink = Some(sink);
        self.current_track = Some(MusicTrack::Game);
        self.adaptive = Some(session);
        log::info!("Playing adaptive music: {} rungs from {}", rungs, self.rungs_dir.display());
        true
    }

    /// Call every game tick, except while paused. The local player's stack (height, remaining
    /// rows, creep pace) drives the adaptive music. No effect on `game.ogg`.
    pub fn update_music_stack(&mut self, ui: &crackattack_core::frame_data::UIState, dt: f32) {
        if let Some(adaptive) = self.adaptive.as_mut() {
            adaptive.update(ui, dt);
        }
    }

    /// Stop current music.
    pub fn stop_music(&mut self) {
        self.track_waiting_for_output = None;
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
        self.adaptive = None;
        self.current_track = None;
        self.music_fade = None;
    }

    /// Fades the current music out over `ticks` ticks, then stops it.
    pub fn fade_out_music(&mut self, ticks: u32) {
        // Music still waiting for the device is dropped, not started after this request.
        self.track_waiting_for_output = None;
        if self.music_sink.is_some() && ticks > 0 {
            self.music_fade = Some((ticks, ticks));
            log::info!("Music fade-out ({} ticks)", ticks);
        }
    }

    /// Call every tick, menus included. Advances the fade-out, restarts a looping track that has
    /// ended (rodio has no end-of-track hook, so the check is an empty sink), and switches to the
    /// menu music once the you win or game over track (played once) has finished.
    pub fn tick_music(&mut self) {
        if self.output.poll().is_some() {
            if let Some(track) = self.track_waiting_for_output.take() {
                self.play_music(track);
            }
        }
        if let Some(track) = self.current_track {
            if track.loops() && self.music_fade.is_none() && self.is_music_finished() {
                self.play_music(track);
            }
        }
        if matches!(self.current_track, Some(MusicTrack::YouWin | MusicTrack::GameOver))
            && self.music_fade.is_none() && self.is_music_finished()
        {
            self.play_music(MusicTrack::Prelude);
        }
        self.tick_music_fade();
    }

    /// Whether a fade-out is in progress.
    pub fn is_music_fading(&self) -> bool {
        self.music_fade.is_some()
    }

    /// Advances the fade-out by one tick.
    fn tick_music_fade(&mut self) {
        let Some((left, total)) = self.music_fade else { return };
        if left <= 1 {
            log::info!("Music stopped after fade-out");
            self.stop_music();
            return;
        }
        let left = left - 1;
        self.music_fade = Some((left, total));
        if let Some(ref sink) = self.music_sink {
            let gain = self.music_volume * self.music_volume;
            sink.set_volume(gain * left as f32 / total as f32);
        }
    }

    /// Pause music.
    pub fn pause_music(&self) {
        if let Some(ref sink) = self.music_sink {
            sink.pause();
        }
    }

    /// Resume music.
    pub fn resume_music(&self) {
        if let Some(ref sink) = self.music_sink {
            sink.play();
        }
    }

    /// Returns the currently playing track (if any).
    pub fn current_track(&self) -> Option<MusicTrack> {
        self.current_track
    }

    /// Check if music finished playing (for auto-transitions).
    pub fn is_music_finished(&self) -> bool {
        match &self.music_sink {
            Some(sink) => sink.empty(),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Temporary folder, removed when the test ends, even on failure.
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `--sounds DIR`: a file present in the tested set wins; a missing file falls back to the
    /// default sound.
    #[test]
    fn sound_set_override_falls_back_per_file() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sounds");
        let tmp = TempDir(std::env::temp_dir()
            .join(format!("crackattack-sounds-test-{}", std::process::id())));
        std::fs::create_dir_all(&tmp.0).unwrap();
        // The tested set only provides swap.wav (content is combo.wav, to tell them apart).
        std::fs::copy(assets.join("combo.wav"), tmp.0.join("swap.wav")).unwrap();

        let (swap, dir) = load_sfx("swap.wav", &assets, Some(&tmp.0));
        assert_eq!(dir, tmp.0);
        let combo = SfxBuffer::load(&assets.join("combo.wav")).unwrap();
        assert_eq!(swap.unwrap().samples, combo.samples);

        let (chain, dir) = load_sfx("chain.wav", &assets, Some(&tmp.0));
        assert_eq!(dir, assets, "missing from the tested set -> default sound");
        assert!(chain.is_some());

        let (_, dir) = load_sfx("swap.wav", &assets, None);
        assert_eq!(dir, assets);
    }

    /// All 14 files of `SFX_FILES` exist in the default sounds.
    #[test]
    fn sfx_files_exist_in_default_set() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sounds");
        for (name, _) in SFX_FILES {
            assert!(assets.join(name).is_file(), "{name}");
        }
    }
}
