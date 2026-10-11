//! The mixer: a single rodio `Source`, plugged in once per game, that plays a few voices at
//! once (a voice is one rung, with its core and rhythm tracks in phase). It counts the
//! frames it produces: this is the music clock, read by the controller on the game side.
//!
//! The audio thread only mixes. Each voice is fed by a reader thread (open, seek, decode,
//! loop) that passes it chunks through a bounded channel. The controller sends commands
//! timed in absolute frames (start a voice at clock N, add gain ramps, stop it).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;

use super::track::PcmSource;

/// Frames per chunk passed from the reader to the mixer (about 93 ms at 44.1 kHz).
const CHUNK_FRAMES: usize = 4096;
/// Chunks the reader may run ahead (about 0.75 s).
const CHUNK_QUEUE: usize = 8;

/// A chunk of voice frames, each: core left, core right, rhythm left, rhythm right.
pub type Chunk = Vec<[i16; 4]>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Linear,
    /// Rising half of a constant-power crossfade (sin).
    EqualPowerIn,
    /// Falling half of a constant-power crossfade (cos).
    EqualPowerOut,
}

/// Gain change from `from` to `to` between frames `start` and `end` (mixer clock).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ramp {
    pub start: u64,
    pub end: u64,
    pub from: f32,
    pub to: f32,
    pub shape: Shape,
}

impl Ramp {
    fn eval(&self, clock: u64) -> f32 {
        let span = self.end.saturating_sub(self.start).max(1) as f32;
        let x = ((clock.saturating_sub(self.start)) as f32 / span).clamp(0.0, 1.0);
        let s = match self.shape {
            Shape::Linear => x,
            Shape::EqualPowerIn => (x * std::f32::consts::FRAC_PI_2).sin(),
            Shape::EqualPowerOut => 1.0 - (x * std::f32::consts::FRAC_PI_2).cos(),
        };
        self.from + (self.to - self.from) * s
    }
}

/// Gain over time: a value, and upcoming timed ramps (sorted by start).
#[derive(Debug, Clone)]
pub struct Envelope {
    value: f32,
    ramps: VecDeque<Ramp>,
}

impl Envelope {
    pub fn new(value: f32, ramps: Vec<Ramp>) -> Self {
        let mut e = Self { value, ramps: VecDeque::new() };
        e.extend(ramps);
        e
    }

    pub fn extend(&mut self, ramps: Vec<Ramp>) {
        self.ramps.extend(ramps);
        self.ramps.make_contiguous().sort_by_key(|r| r.start);
    }

    pub fn at(&mut self, clock: u64) -> f32 {
        while let Some(r) = self.ramps.front() {
            if clock >= r.end {
                self.value = r.to;
                self.ramps.pop_front();
                continue;
            }
            if clock >= r.start {
                return r.eval(clock);
            }
            break;
        }
        self.value
    }
}

/// Read position in a track that loops over [loop_start, loop_end). What precedes
/// `loop_start` (the pickup) is read only once, at the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopCursor {
    pub pos: u64,
    pub loop_start: u64,
    pub loop_end: u64,
}

impl LoopCursor {
    pub fn new(pos: u64, loop_start: u64, loop_end: u64) -> Self {
        let mut c = Self { pos, loop_start, loop_end };
        if c.pos >= loop_end {
            c.pos = loop_start + (c.pos - loop_end) % (loop_end - loop_start).max(1);
        }
        c
    }

    /// Next contiguous segment of at most `n` frames, as (start, length). Advances the
    /// position and wraps to the loop start when it reaches the loop end.
    pub fn next_segment(&mut self, n: u64) -> (u64, u64) {
        let start = self.pos;
        let len = n.min(self.loop_end - self.pos);
        self.pos += len;
        if self.pos >= self.loop_end {
            self.pos = self.loop_start;
        }
        (start, len)
    }
}

/// Produces the chunks of a voice: reads core and rhythm in phase, following the loop.
pub struct ChunkProducer {
    core: Box<dyn PcmSource>,
    rhythm: Box<dyn PcmSource>,
    cursor: LoopCursor,
    /// Actual position of the readers (so they only seek at the loop jump).
    file_pos: Option<u64>,
    core_buf: Vec<[i16; 2]>,
    rhythm_buf: Vec<[i16; 2]>,
}

impl ChunkProducer {
    pub fn new(core: Box<dyn PcmSource>, rhythm: Box<dyn PcmSource>, cursor: LoopCursor) -> Self {
        Self { core, rhythm, cursor, file_pos: None, core_buf: Vec::new(), rhythm_buf: Vec::new() }
    }

    pub fn produce(&mut self) -> std::io::Result<Chunk> {
        let mut chunk = Vec::with_capacity(CHUNK_FRAMES);
        while chunk.len() < CHUNK_FRAMES {
            let (start, len) = self.cursor.next_segment((CHUNK_FRAMES - chunk.len()) as u64);
            if self.file_pos != Some(start) {
                self.core.seek(start)?;
                self.rhythm.seek(start)?;
            }
            let len = len as usize;
            self.core_buf.resize(len, [0; 2]);
            self.rhythm_buf.resize(len, [0; 2]);
            let n = self.core.read(&mut self.core_buf)?.min(self.rhythm.read(&mut self.rhythm_buf)?);
            if n < len {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "track too short"));
            }
            self.file_pos = Some(start + len as u64);
            chunk.extend(self.core_buf.iter().zip(&self.rhythm_buf).map(|(c, r)| [c[0], c[1], r[0], r[1]]));
        }
        Ok(chunk)
    }
}

/// Where a voice gets its chunks: a reader thread, or (tests) a producer called in place.
enum Feed {
    Thread(Receiver<Chunk>),
    #[cfg(test)]
    Inline(ChunkProducer),
}

enum Pull {
    Frame([i16; 4]),
    /// The reader is late: a missed frame.
    Late,
    Ended,
}

pub struct Voice {
    pub id: u32,
    start_clock: u64,
    feed: Feed,
    buf: Chunk,
    buf_pos: usize,
    started: bool,
    /// Frames to discard to stay on time (voice started late, or reader late).
    skip: u64,
    master: Envelope,
    rhythm: Envelope,
    stop_at: Option<u64>,
}

impl Voice {
    /// Starts the reader thread. `open` opens the two tracks (off the audio thread).
    pub fn spawn(
        id: u32,
        start_clock: u64,
        cursor: LoopCursor,
        master: Envelope,
        rhythm: Envelope,
        open: impl FnOnce() -> Result<(Box<dyn PcmSource>, Box<dyn PcmSource>), String> + Send + 'static,
    ) -> Voice {
        let (tx, rx): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(CHUNK_QUEUE);
        let spawned = std::thread::Builder::new()
            .name(format!("music-voice-{id}"))
            .spawn(move || {
                let (core, rhythm) = match open() {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("Adaptive music: {e}");
                        return;
                    }
                };
                let mut producer = ChunkProducer::new(core, rhythm, cursor);
                loop {
                    let chunk = match producer.produce() {
                        Ok(c) => c,
                        Err(e) => {
                            log::warn!("Adaptive music: playback interrupted: {e}");
                            return;
                        }
                    };
                    // The mixer dropped the voice: stop.
                    if tx.send(chunk).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("Adaptive music: cannot start player thread: {e}");
        }
        Self::with_feed(id, start_clock, Feed::Thread(rx), master, rhythm)
    }

    #[cfg(test)]
    pub fn inline(id: u32, start_clock: u64, producer: ChunkProducer, master: Envelope, rhythm: Envelope) -> Voice {
        Self::with_feed(id, start_clock, Feed::Inline(producer), master, rhythm)
    }

    fn with_feed(id: u32, start_clock: u64, feed: Feed, master: Envelope, rhythm: Envelope) -> Voice {
        Voice { id, start_clock, feed, buf: Vec::new(), buf_pos: 0, started: false, skip: 0,
            master, rhythm, stop_at: None }
    }

    fn pull(&mut self) -> Pull {
        if self.buf_pos >= self.buf.len() {
            let next = match &mut self.feed {
                Feed::Thread(rx) => match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => return Pull::Late,
                    Err(TryRecvError::Disconnected) => None,
                },
                #[cfg(test)]
                Feed::Inline(p) => p.produce().ok(),
            };
            let Some(chunk) = next else { return Pull::Ended };
            self.buf = chunk;
            self.buf_pos = 0;
        }
        let f = self.buf[self.buf_pos];
        self.buf_pos += 1;
        Pull::Frame(f)
    }

    /// Adds this voice's frame to `out`. Returns `false` when the voice is finished.
    fn mix(&mut self, clock: u64, out: &mut [f32; 2], late: &AtomicU64) -> bool {
        if self.stop_at.is_some_and(|s| clock >= s) {
            return false;
        }
        if clock < self.start_clock {
            return true;
        }
        if !self.started {
            self.started = true;
            self.skip = clock - self.start_clock;
        }
        while self.skip > 0 {
            match self.pull() {
                Pull::Frame(_) => self.skip -= 1,
                Pull::Late => break,
                Pull::Ended => return false,
            }
        }
        let master = self.master.at(clock);
        let rhythm = self.rhythm.at(clock);
        match self.pull() {
            Pull::Frame(f) => {
                if self.skip == 0 {
                    out[0] += master * (f[0] as f32 + rhythm * f[2] as f32) / 32768.0;
                    out[1] += master * (f[1] as f32 + rhythm * f[3] as f32) / 32768.0;
                } else {
                    self.skip -= 1;
                }
                true
            }
            Pull::Late => {
                self.skip += 1;
                late.fetch_add(1, Ordering::Relaxed);
                true
            }
            Pull::Ended => false,
        }
    }
}

pub enum Command {
    Start(Box<Voice>),
    /// Ramps to add to an existing voice, and the clock at which to stop it.
    Envelope { id: u32, master: Vec<Ramp>, rhythm: Vec<Ramp>, stop_at: Option<u64> },
}

/// Game side: send commands, read the clock.
pub struct MixerHandle {
    tx: Sender<Command>,
    clock: Arc<AtomicU64>,
    late: Arc<AtomicU64>,
}

impl MixerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
    /// Frames produced since the start of the game.
    pub fn clock(&self) -> u64 {
        self.clock.load(Ordering::Relaxed)
    }
    /// Missed frames (reader late) since the start.
    pub fn late_frames(&self) -> u64 {
        self.late.load(Ordering::Relaxed)
    }
}

pub struct AdaptiveMusic {
    rx: Receiver<Command>,
    voices: Vec<Voice>,
    clock: u64,
    shared_clock: Arc<AtomicU64>,
    late: Arc<AtomicU64>,
    sample_rate: u32,
    frame: [f32; 2],
    channel: usize,
}

impl AdaptiveMusic {
    pub fn new(sample_rate: u32) -> (AdaptiveMusic, MixerHandle) {
        let (tx, rx) = mpsc::channel();
        let clock = Arc::new(AtomicU64::new(0));
        let late = Arc::new(AtomicU64::new(0));
        let handle = MixerHandle { tx, clock: clock.clone(), late: late.clone() };
        let music = AdaptiveMusic { rx, voices: Vec::new(), clock: 0, shared_clock: clock, late,
            sample_rate, frame: [0.0; 2], channel: 0 };
        (music, handle)
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::Start(v) => self.voices.push(*v),
                Command::Envelope { id, master, rhythm, stop_at } => {
                    if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
                        v.master.extend(master);
                        v.rhythm.extend(rhythm);
                        if stop_at.is_some() {
                            v.stop_at = stop_at;
                        }
                    }
                }
            }
        }
    }

    fn compute_frame(&mut self) {
        self.drain_commands();
        let clock = self.clock;
        let mut out = [0.0f32; 2];
        let late = &self.late;
        self.voices.retain_mut(|v| v.mix(clock, &mut out, late));
        self.clock += 1;
        self.shared_clock.store(self.clock, Ordering::Relaxed);
        self.frame = [out[0].clamp(-1.0, 1.0), out[1].clamp(-1.0, 1.0)];
    }
}

impl Iterator for AdaptiveMusic {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.channel == 0 {
            self.compute_frame();
        }
        let s = self.frame[self.channel];
        self.channel ^= 1;
        Some(s)
    }
}

impl rodio::Source for AdaptiveMusic {
    fn current_frame_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { 2 }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<std::time::Duration> { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::adaptive_music::track::MemPcm;

    /// Test track: the value of each frame encodes its position.
    fn ramp_track(len: usize, offset: i16) -> Box<dyn PcmSource> {
        let frames = (0..len).map(|i| [(i % 20000) as i16 + offset, -((i % 20000) as i16)]).collect();
        Box::new(MemPcm { frames, pos: 0 })
    }

    #[test]
    fn loop_cursor_has_no_gap_nor_duplicate() {
        let mut c = LoopCursor::new(90, 10, 100);
        let mut seq = Vec::new();
        while seq.len() < 300 {
            let (s, n) = c.next_segment(7);
            seq.extend(s..s + n);
        }
        let expected: Vec<u64> = (90..100).chain((0..).flat_map(|_| 10..100)).take(seq.len()).collect();
        assert_eq!(seq, expected);
        // A position beyond the loop is brought back into it.
        assert_eq!(LoopCursor::new(105, 10, 100).pos, 15);
    }

    #[test]
    fn voice_plays_from_exact_position_and_loops_seamlessly() {
        let (loop_start, loop_end) = (1000u64, 9000u64);
        let producer = ChunkProducer::new(ramp_track(9500, 0), ramp_track(9500, 0),
            LoopCursor::new(8000, loop_start, loop_end));
        let (mut music, handle) = AdaptiveMusic::new(44100);
        // Starts at clock frame 100; rhythm muted, so only the core track is heard.
        handle.send(Command::Start(Box::new(Voice::inline(1, 100, producer,
            Envelope::new(1.0, vec![]), Envelope::new(0.0, vec![])))));
        let left: Vec<i32> = (0..2 * 3000).map(|_| music.next().unwrap())
            .step_by(2).map(|s| (s * 32768.0).round() as i32).collect();
        assert!(left[..100].iter().all(|&s| s == 0), "silence before the start");
        // Position 8000 at the start, loop end at 9000, back to 1000, with no gap or duplicate.
        let expected: Vec<i32> = (8000..9000).chain(1000..).take(2900).map(|p| p as i32).collect();
        assert_eq!(&left[100..], &expected[..]);
        assert_eq!(handle.clock(), 3000);
    }

    #[test]
    fn envelope_and_stop_are_applied_at_exact_frames() {
        let producer = ChunkProducer::new(ramp_track(20000, 0), ramp_track(20000, 0),
            LoopCursor::new(0, 0, 20000));
        let (mut music, handle) = AdaptiveMusic::new(44100);
        handle.send(Command::Start(Box::new(Voice::inline(7, 0, producer,
            Envelope::new(1.0, vec![]), Envelope::new(0.0, vec![])))));
        handle.send(Command::Envelope { id: 7, stop_at: Some(500), rhythm: vec![],
            master: vec![Ramp { start: 200, end: 400, from: 1.0, to: 0.0, shape: Shape::Linear }] });
        let left: Vec<f32> = (0..2 * 600).map(|_| music.next().unwrap()).step_by(2).collect();
        assert!((left[199] * 32768.0 - 199.0).abs() < 0.01);
        assert!((left[300] * 32768.0 - 150.0).abs() < 0.01); // gain 0.5 at mid-ramp
        assert!(left[500..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn equal_power_crossfade_keeps_power() {
        for i in 0..=10 {
            let c = 100 + i * 10;
            let out = Ramp { start: 100, end: 200, from: 1.0, to: 0.0, shape: Shape::EqualPowerOut }.eval(c);
            let inn = Ramp { start: 100, end: 200, from: 0.0, to: 1.0, shape: Shape::EqualPowerIn }.eval(c);
            assert!((out * out + inn * inn - 1.0).abs() < 1e-5);
        }
    }
}
