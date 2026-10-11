//! Streaming reader for the 16-bit stereo PCM WAV rung tracks, with sample-exact seeking
//! (PCM WAV can be read from any frame: byte offset = frame x 4).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::track::PcmSource;

/// What is kept from a WAV header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    /// Number of stereo frames.
    pub frames: u64,
    /// Offset of the PCM data in the file.
    data_offset: u64,
}

/// Reads the header. Requires 16-bit stereo PCM (format 1, or WAVE_FORMAT_EXTENSIBLE with
/// the PCM sub-format).
pub fn probe(path: &Path) -> Result<WavInfo, String> {
    let mut f = BufReader::new(File::open(path).map_err(|e| format!("{} : {e}", path.display()))?);
    parse_header(&mut f).map_err(|e| format!("{} : {e}", path.display()))
}

fn parse_header(r: &mut (impl Read + Seek)) -> Result<WavInfo, String> {
    let mut riff = [0u8; 12];
    r.read_exact(&mut riff).map_err(|_| "truncated RIFF header".to_string())?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err("not a WAV file (RIFF/WAVE)".into());
    }
    let mut fmt: Option<(u16, u32, u16)> = None; // (channels, sample rate, bits)
    let mut pos = 12u64;
    loop {
        let mut head = [0u8; 8];
        if r.read_exact(&mut head).is_err() {
            return Err("\"data\" chunk not found".into());
        }
        let size = u32::from_le_bytes(head[4..8].try_into().unwrap()) as u64;
        pos += 8;
        match &head[0..4] {
            b"fmt " => {
                let mut b = vec![0u8; size as usize];
                r.read_exact(&mut b).map_err(|_| "truncated \"fmt\" chunk".to_string())?;
                if b.len() < 16 {
                    return Err("\"fmt\" chunk too short".into());
                }
                let tag = u16::from_le_bytes([b[0], b[1]]);
                let pcm = tag == 1 || (tag == 0xFFFE && b.len() >= 26 && u16::from_le_bytes([b[24], b[25]]) == 1);
                if !pcm {
                    return Err(format!("format {tag:#x}: only PCM is read"));
                }
                fmt = Some((
                    u16::from_le_bytes([b[2], b[3]]),
                    u32::from_le_bytes(b[4..8].try_into().unwrap()),
                    u16::from_le_bytes([b[14], b[15]]),
                ));
            }
            b"data" => {
                let Some((channels, sample_rate, bits)) = fmt else {
                    return Err("\"data\" chunk before \"fmt\"".into());
                };
                if channels != 2 || bits != 16 {
                    return Err(format!("{channels} channels, {bits} bits: expected 16-bit stereo"));
                }
                return Ok(WavInfo { sample_rate, frames: size / 4, data_offset: pos });
            }
            _ => {}
        }
        // Chunks of odd size are followed by a padding byte.
        let skip = size + (size & 1);
        if !matches!(&head[0..4], b"fmt ") {
            r.seek(SeekFrom::Current(skip as i64)).map_err(|e| e.to_string())?;
        } else if size & 1 == 1 {
            r.seek(SeekFrom::Current(1)).map_err(|e| e.to_string())?;
        }
        pos += skip;
    }
}

pub struct WavReader {
    file: BufReader<File>,
    info: WavInfo,
    pos: u64,
    bytes: Vec<u8>,
}

impl WavReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let info = probe(path)?;
        let mut file = BufReader::with_capacity(1 << 16,
            File::open(path).map_err(|e| format!("{} : {e}", path.display()))?);
        file.seek(SeekFrom::Start(info.data_offset)).map_err(|e| e.to_string())?;
        Ok(Self { file, info, pos: 0, bytes: Vec::new() })
    }
}

impl PcmSource for WavReader {
    fn seek(&mut self, frame: u64) -> io::Result<()> {
        let frame = frame.min(self.info.frames);
        self.file.seek(SeekFrom::Start(self.info.data_offset + frame * 4))?;
        self.pos = frame;
        Ok(())
    }

    fn read(&mut self, out: &mut [[i16; 2]]) -> io::Result<usize> {
        let n = (out.len() as u64).min(self.info.frames - self.pos) as usize;
        self.bytes.resize(n * 4, 0);
        self.file.read_exact(&mut self.bytes)?;
        for (frame, b) in out.iter_mut().zip(self.bytes.chunks_exact(4)) {
            *frame = [i16::from_le_bytes([b[0], b[1]]), i16::from_le_bytes([b[2], b[3]])];
        }
        self.pos += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) fn write_test_wav(path: &Path, frames: &[[i16; 2]], sample_rate: u32) {
    let mut b = Vec::new();
    let data_len = frames.len() as u32 * 4;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(4 + 8 + 16 + 8 + 3 + 1 + 8 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    // An unknown chunk of odd size before "data": the padding byte must be skipped.
    b.extend_from_slice(b"LIST");
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&[1, 2, 3, 0]);
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for f in frames {
        b.extend_from_slice(&f[0].to_le_bytes());
        b.extend_from_slice(&f[1].to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_reader_seeks_to_exact_frame() {
        let dir = std::env::temp_dir().join(format!("ca-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(dir.clone());

        let frames: Vec<[i16; 2]> = (0..1000).map(|i| [i as i16, -(i as i16)]).collect();
        let path = dir.join("t.wav");
        write_test_wav(&path, &frames, 44100);

        let info = probe(&path).unwrap();
        assert_eq!((info.sample_rate, info.frames), (44100, 1000));
        let mut r = WavReader::open(&path).unwrap();
        r.seek(777).unwrap();
        let mut out = [[0i16; 2]; 300];
        assert_eq!(r.read(&mut out).unwrap(), 223);
        assert_eq!(out[0], [777, -777]);
        assert_eq!(out[222], [999, -999]);
    }
}
