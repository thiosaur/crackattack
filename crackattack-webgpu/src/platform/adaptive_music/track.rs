//! The rung tracks: a seekable reading interface, and the choice of reader by file
//! extension (`.ogg`: Ogg Vorbis, otherwise PCM WAV).

use std::io;
use std::path::Path;

use super::{ogg, wav};

/// What is kept of a track to validate the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackInfo {
    pub sample_rate: u32,
    /// Number of stereo frames.
    pub frames: u64,
}

/// A source of stereo i16 frames that can be positioned to the exact frame. Implemented by
/// the WAV and Ogg Vorbis readers, and by in-memory buffers in tests.
pub trait PcmSource: Send {
    fn seek(&mut self, frame: u64) -> io::Result<()>;
    /// Fills `out` from the current position and returns the number of frames read (fewer
    /// than requested only at the end of the file).
    fn read(&mut self, out: &mut [[i16; 2]]) -> io::Result<usize>;
}

fn is_ogg(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ogg"))
}

pub fn probe(path: &Path) -> Result<TrackInfo, String> {
    if is_ogg(path) {
        ogg::probe(path)
    } else {
        wav::probe(path).map(|i| TrackInfo { sample_rate: i.sample_rate, frames: i.frames })
    }
}

pub fn open(path: &Path) -> Result<Box<dyn PcmSource>, String> {
    Ok(if is_ogg(path) { Box::new(ogg::OggReader::open(path)?) } else { Box::new(wav::WavReader::open(path)?) })
}

/// In-memory buffer (tests).
#[cfg(test)]
pub struct MemPcm {
    pub frames: Vec<[i16; 2]>,
    pub pos: usize,
}

#[cfg(test)]
impl PcmSource for MemPcm {
    fn seek(&mut self, frame: u64) -> io::Result<()> {
        self.pos = (frame as usize).min(self.frames.len());
        Ok(())
    }
    fn read(&mut self, out: &mut [[i16; 2]]) -> io::Result<usize> {
        let n = out.len().min(self.frames.len() - self.pos);
        out[..n].copy_from_slice(&self.frames[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}
