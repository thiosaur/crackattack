//! Streaming reader for the Ogg Vorbis rung tracks, with frame-exact seeking.
//!
//! lewton only seeks to a page (`seek_absgp_pg`, which lands on a page at or before the
//! target). So the reader seeks back by a margin, then decodes and discards frames up to the
//! target. After a seek, the absolute position is only known once a packet that ends a page
//! has been decoded (`get_last_absgp`); everything decoded before that is discarded. If the
//! margin was not enough, the reader restarts from the beginning of the file, which is slower
//! but exact.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use lewton::inside_ogg::OggStreamReader;

use super::track::{PcmSource, TrackInfo};

/// How far before the target a seek lands. It covers the page found, the priming packet that
/// a seek loses, and the packet that fixes the position (a Vorbis block is at most 8192
/// samples).
const SEEK_MARGIN: u64 = 4 * 8192;

/// Number of bytes read at the end of the file to find the last page (a page is under 65,307 bytes).
const TAIL_BYTES: u64 = 1 << 17;

type Stream = OggStreamReader<BufReader<File>>;

fn open_stream(path: &Path) -> Result<Stream, String> {
    let file = File::open(path).map_err(|e| format!("{} : {e}", path.display()))?;
    OggStreamReader::new(BufReader::with_capacity(1 << 16, file)).map_err(|e| format!("{} : {e}", path.display()))
}

fn vorbis_err(e: lewton::VorbisError) -> io::Error {
    io::Error::other(e)
}

/// Sample rate and channels come from the Vorbis header. The length comes from the granule
/// position of the last page, which equals the total number of frames.
pub fn probe(path: &Path) -> Result<TrackInfo, String> {
    let stream = open_stream(path)?;
    let (channels, sample_rate) = (stream.ident_hdr.audio_channels, stream.ident_hdr.audio_sample_rate);
    if channels != 2 {
        return Err(format!("{}: {channels} channels, expected stereo", path.display()));
    }
    let frames = last_granule(path).map_err(|e| format!("{} : {e}", path.display()))?;
    Ok(TrackInfo { sample_rate, frames })
}

fn last_granule(path: &Path) -> Result<u64, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).map_err(|e| e.to_string())?;
    // Page header: "OggS", version 0, type, granule (u64 LE at bytes 6..14).
    (0..tail.len().saturating_sub(27)).rev()
        .find(|&i| &tail[i..i + 4] == b"OggS" && tail[i + 4] == 0)
        .map(|i| u64::from_le_bytes(tail[i + 6..i + 14].try_into().unwrap()))
        .ok_or_else(|| "no Ogg page at end of file".to_string())
}

pub struct OggReader {
    path: PathBuf,
    stream: Stream,
    frames: u64,
    /// Position of the next frame to be returned.
    pos: u64,
    /// Frames decoded ahead of time: `buf[buf_at..]` starts at `pos`.
    buf: Vec<[i16; 2]>,
    buf_at: usize,
}

impl OggReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let info = probe(path)?;
        Ok(Self { path: path.to_path_buf(), stream: open_stream(path)?, frames: info.frames, pos: 0, buf: Vec::new(), buf_at: 0 })
    }

    /// Decodes the next packet into `buf`. Returns `false` at the end of the stream.
    fn decode(&mut self) -> io::Result<bool> {
        let Some(samples) = self.stream.read_dec_packet_itl().map_err(vorbis_err)? else { return Ok(false) };
        self.buf.clear();
        self.buf.extend(samples.chunks_exact(2).map(|s| [s[0], s[1]]));
        self.buf_at = 0;
        Ok(true)
    }

    /// Restarts from the beginning of the file and decodes up to `frame`.
    fn seek_from_start(&mut self, frame: u64) -> io::Result<()> {
        self.stream = open_stream(&self.path).map_err(io::Error::other)?;
        self.pos = 0;
        self.buf.clear();
        self.buf_at = 0;
        while self.pos < frame {
            if !self.decode()? {
                break;
            }
            let skip = ((frame - self.pos) as usize).min(self.buf.len());
            self.buf_at = skip;
            self.pos += skip as u64;
        }
        Ok(())
    }
}

impl PcmSource for OggReader {
    fn seek(&mut self, frame: u64) -> io::Result<()> {
        let frame = frame.min(self.frames);
        if frame < SEEK_MARGIN {
            return self.seek_from_start(frame);
        }
        self.stream.seek_absgp_pg(frame - SEEK_MARGIN).map_err(vorbis_err)?;
        self.buf.clear();
        self.buf_at = 0;
        loop {
            let start = self.stream.get_last_absgp();
            // Near the start, the page found can be the header page, which cannot be read as
            // audio. Restart from the beginning (a real file error will surface there).
            let Ok(more) = self.decode() else { return self.seek_from_start(frame) };
            if !more {
                // Target is past the last packet: end of file.
                self.pos = self.frames;
                self.buf.clear();
                return Ok(());
            }
            // The position is unknown until a page end has been passed: discard the packet.
            let Some(start) = start else { continue };
            if start > frame {
                log::warn!("Music: {}: seek too short ({start} > {frame}), re-reading from the start",
                    self.path.display());
                return self.seek_from_start(frame);
            }
            let end = start + self.buf.len() as u64;
            if end > frame {
                self.buf_at = (frame - start) as usize;
                self.pos = frame;
                return Ok(());
            }
        }
    }

    fn read(&mut self, out: &mut [[i16; 2]]) -> io::Result<usize> {
        let want = (out.len() as u64).min(self.frames - self.pos) as usize;
        let mut n = 0;
        while n < want {
            if self.buf_at == self.buf.len() && !self.decode()? {
                break;
            }
            let take = (want - n).min(self.buf.len() - self.buf_at);
            out[n..n + take].copy_from_slice(&self.buf[self.buf_at..self.buf_at + take]);
            self.buf_at += take;
            n += take;
        }
        self.pos += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12 s excerpt of a rung (stress_150, core track), encoded like the game tracks.
    const EXCERPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/platform/adaptive_music/testdata/excerpt.ogg");

    fn read_all(r: &mut OggReader) -> Vec<[i16; 2]> {
        let mut all = Vec::new();
        let mut block = vec![[0i16; 2]; 4096];
        loop {
            let n = r.read(&mut block).unwrap();
            if n == 0 {
                return all;
            }
            all.extend_from_slice(&block[..n]);
        }
    }

    #[test]
    fn probe_matches_decoded_length() {
        let info = probe(Path::new(EXCERPT)).unwrap();
        assert_eq!(info.sample_rate, 44100);
        let all = read_all(&mut OggReader::open(Path::new(EXCERPT)).unwrap());
        assert_eq!(all.len() as u64, info.frames);
    }

    /// A seek returns exactly the frames obtained by reading from the start, for any target:
    /// the start, just after the margin, mid-page, and near the end.
    #[test]
    fn seek_is_sample_exact() {
        let path = Path::new(EXCERPT);
        let all = read_all(&mut OggReader::open(path).unwrap());
        let len = all.len() as u64;
        let mut r = OggReader::open(path).unwrap();
        for target in [0, 1, 777, SEEK_MARGIN - 1, SEEK_MARGIN, SEEK_MARGIN + 1, 100_003, 250_000, len / 2, len - 5000, len - 1] {
            r.seek(target).unwrap();
            let mut out = vec![[0i16; 2]; 3000];
            let n = r.read(&mut out).unwrap();
            let expect = &all[target as usize..(target as usize + 3000).min(all.len())];
            assert_eq!(n, expect.len(), "length read after a seek to {target}");
            assert!(out[..n] == *expect, "different frames after a seek to {target}");
        }
        r.seek(len).unwrap();
        assert_eq!(r.read(&mut [[0i16; 2]; 16]).unwrap(), 0);
    }
}
