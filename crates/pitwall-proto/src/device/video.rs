//! The video stream of a device tile: scrcpy's framing (v4 and later,
//! checked against scrcpy v5.0.1's `Streamer.java` and `demuxer.c`),
//! big-endian.
//!
//! ```text
//! once         u32 codec id ("h264" = 0x68323634)
//! session      u32 0x80000000 | flags, u32 width, u32 height
//!              (first, and again whenever capture restarts: a rotation,
//!              a fold, a new size)
//! media        u64 pts and flags, u32 size, then `size` bytes
//!              bit 63 clear (set means a session packet)
//!              bit 62 a config packet (SPS/PPS; pts 0)
//!              bit 61 a key frame; bits 0..60 the pts in µs
//! ```
//!
//! The payload is H.264 in Annex B (start codes). [`Reader`] takes bytes
//! as they arrive, split anywhere, and gives whole events.

/// `"h264"` as scrcpy's codec id.
pub const CODEC_H264: u32 = u32::from_be_bytes(*b"h264");
/// `"h265"`.
pub const CODEC_H265: u32 = u32::from_be_bytes(*b"h265");
/// `"\0av1"`.
pub const CODEC_AV1: u32 = u32::from_be_bytes(*b"\0av1");
/// The codec id scrcpy sends when the device couldn't set up its encoder.
pub const CODEC_ERROR: u32 = 1;

const SESSION: u64 = 1 << 63;
const FLAG_CONFIG: u64 = 1 << 62;
const FLAG_KEY: u64 = 1 << 61;
const PTS_MASK: u64 = FLAG_KEY - 1;

/// The largest packet accepted (a 4K key frame is far below it): a bigger
/// size means the stream is not what we think it is.
pub const MAX_PACKET: u32 = 32 << 20;

/// The codec id, as sent first.
pub fn codec(id: u32) -> [u8; 4] {
    id.to_be_bytes()
}

/// A session packet: the size of the frames that follow.
pub fn session(width: u32, height: u32) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0..4].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    b[4..8].copy_from_slice(&width.to_be_bytes());
    b[8..12].copy_from_slice(&height.to_be_bytes());
    b
}

/// One packet of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// SPS/PPS: the decoder is made again from these.
    pub config: bool,
    pub key: bool,
    /// Presentation time in microseconds (0 for a config packet).
    pub pts: u64,
    pub data: Vec<u8>,
}

impl Packet {
    /// Its header and payload, as sent.
    pub fn encode(&self) -> Vec<u8> {
        let mut flags = self.pts & PTS_MASK;
        if self.config {
            flags = FLAG_CONFIG;
        }
        if self.key {
            flags |= FLAG_KEY;
        }
        let mut out = Vec::with_capacity(12 + self.data.len());
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

/// What the stream said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The frames that follow are this size.
    Session { width: u32, height: u32 },
    Packet(Packet),
}

/// Why the stream can't be read on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A codec other than H.264 (the only one asked for).
    Codec(u32),
    /// The device couldn't encode its screen.
    Encoder,
    /// A packet larger than [`MAX_PACKET`].
    TooLarge(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Codec(c) => write!(f, "the device sent video as {:?}, not H.264", String::from_utf8_lossy(&c.to_be_bytes()).trim_matches('\0')),
            Error::Encoder => write!(f, "the device couldn't encode its screen"),
            Error::TooLarge(n) => write!(f, "a video packet of {n} bytes: not a stream Pitwall can read"),
        }
    }
}

/// Reads the stream from bytes as they come.
#[derive(Debug, Default)]
pub struct Reader {
    buf: Vec<u8>,
    /// How many bytes to skip first (scrcpy's dummy byte, its device name).
    skip: usize,
    codec: bool,
}

impl Reader {
    /// A reader for a stream that starts with its codec id.
    pub fn new() -> Reader {
        Reader::default()
    }

    /// A reader that first skips `n` bytes: scrcpy's forward tunnel sends
    /// one dummy byte on the first socket, and `send_device_meta` a 64-byte
    /// device name.
    pub fn skipping(n: usize) -> Reader {
        Reader { skip: n, ..Reader::default() }
    }

    /// Takes `bytes`; returns every whole event they complete.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>, Error> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut at = 0;
        if self.skip > 0 {
            let n = self.skip.min(self.buf.len());
            self.skip -= n;
            at = n;
        }
        loop {
            let rest = &self.buf[at..];
            if !self.codec {
                if rest.len() < 4 {
                    break;
                }
                match u32::from_be_bytes(rest[0..4].try_into().unwrap()) {
                    CODEC_H264 => {}
                    CODEC_ERROR => return Err(Error::Encoder),
                    other => return Err(Error::Codec(other)),
                }
                self.codec = true;
                at += 4;
                continue;
            }
            if rest.len() < 12 {
                break;
            }
            let flags = u64::from_be_bytes(rest[0..8].try_into().unwrap());
            if flags & SESSION != 0 {
                out.push(Event::Session {
                    width: u32::from_be_bytes(rest[4..8].try_into().unwrap()),
                    height: u32::from_be_bytes(rest[8..12].try_into().unwrap()),
                });
                at += 12;
                continue;
            }
            let size = u32::from_be_bytes(rest[8..12].try_into().unwrap());
            if size > MAX_PACKET {
                return Err(Error::TooLarge(size));
            }
            if rest.len() < 12 + size as usize {
                break;
            }
            let config = flags & FLAG_CONFIG != 0;
            out.push(Event::Packet(Packet {
                config,
                key: flags & FLAG_KEY != 0,
                pts: if config { 0 } else { flags & PTS_MASK },
                data: rest[12..12 + size as usize].to_vec(),
            }));
            at += 12 + size as usize;
        }
        self.buf.drain(..at);
        Ok(out)
    }
}

/// The NAL units of an Annex B buffer (start codes removed), in order.
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    // Where each start code begins and its NAL starts.
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (n, &(_, begin)) in starts.iter().enumerate() {
        let next = starts.get(n + 1);
        let mut end = next.map_or(data.len(), |&(code, _)| code);
        // A 4-byte start code's leading zero belongs to it, not to this NAL.
        while next.is_some() && end > begin && data[end - 1] == 0 {
            end -= 1;
        }
        if end > begin {
            out.push(&data[begin..end]);
        }
    }
    out
}

/// The NAL unit type (`nal_unit_type`, the low 5 bits of its first byte).
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |b| b & 0x1f)
}

/// H.264 NAL types the decoder cares about.
pub const NAL_IDR: u8 = 5;
pub const NAL_SPS: u8 = 7;
pub const NAL_PPS: u8 = 8;

/// An Annex B buffer as AVCC (each NAL prefixed with its 4-byte length),
/// leaving out parameter sets (they go in the format description).
pub fn to_avcc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for nal in nal_units(data) {
        if matches!(nal_type(nal), NAL_SPS | NAL_PPS) {
            continue;
        }
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// AVCC (4-byte lengths) as Annex B (4-byte start codes): what an encoder
/// on macOS gives, as the stream carries it.
pub fn avcc_to_annex_b(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i + 4 <= data.len() {
        let n = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + n > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    out
}

/// The SPS and PPS of a config packet.
pub fn parameter_sets(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let nals = nal_units(data);
    let sps = nals.iter().find(|n| nal_type(n) == NAL_SPS)?;
    let pps = nals.iter().find(|n| nal_type(n) == NAL_PPS)?;
    Some((sps, pps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> Vec<u8> {
        let mut s = codec(CODEC_H264).to_vec();
        s.extend(session(1080, 2400));
        s.extend(Packet { config: true, key: false, pts: 0, data: vec![0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3] }.encode());
        s.extend(Packet { config: false, key: true, pts: 16_666, data: vec![0, 0, 0, 1, 0x65, 9, 9, 9] }.encode());
        s.extend(Packet { config: false, key: false, pts: 33_333, data: vec![0, 0, 1, 0x41, 7] }.encode());
        // Rotated: a new session, then a new config.
        s.extend(session(2400, 1080));
        s.extend(Packet { config: true, key: false, pts: 0, data: vec![0, 0, 0, 1, 0x67, 4] }.encode());
        s
    }

    #[test]
    fn a_stream_read_in_one_piece_gives_sessions_and_packets() {
        let ev = Reader::new().push(&stream()).unwrap();
        assert_eq!(ev.len(), 6);
        assert_eq!(ev[0], Event::Session { width: 1080, height: 2400 });
        let Event::Packet(p) = &ev[1] else { panic!() };
        assert!(p.config && !p.key && p.pts == 0);
        let Event::Packet(p) = &ev[2] else { panic!() };
        assert!(!p.config && p.key && p.pts == 16_666);
        let Event::Packet(p) = &ev[3] else { panic!() };
        assert!(!p.key && p.pts == 33_333 && p.data == [0, 0, 1, 0x41, 7]);
        assert_eq!(ev[4], Event::Session { width: 2400, height: 1080 });
        let Event::Packet(p) = &ev[5] else { panic!() };
        assert!(p.config);
    }

    #[test]
    fn the_bits_are_scrcpys() {
        // scrcpy v5.0.1 demuxer.c: bit 62 config, bit 61 key frame.
        let p = Packet { config: false, key: true, pts: 5, data: vec![] }.encode();
        assert_eq!(&p[0..8], &((1u64 << 61) | 5).to_be_bytes());
        let p = Packet { config: true, key: false, pts: 99, data: vec![] }.encode();
        assert_eq!(&p[0..8], &(1u64 << 62).to_be_bytes());
        assert_eq!(session(1, 2)[0], 0x80);
    }

    #[test]
    fn a_stream_split_at_every_byte_reads_the_same() {
        let s = stream();
        let mut r = Reader::new();
        let mut all = Vec::new();
        for b in &s {
            all.extend(r.push(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(all, Reader::new().push(&s).unwrap());
    }

    #[test]
    fn scrcpys_dummy_byte_and_device_name_are_skipped() {
        let mut s = vec![0u8];
        s.extend([b'x'; 64]);
        s.extend(stream());
        let mut r = Reader::skipping(65);
        let (a, b) = s.split_at(30);
        let mut ev = r.push(a).unwrap();
        ev.extend(r.push(b).unwrap());
        assert_eq!(ev, Reader::new().push(&stream()).unwrap());
    }

    #[test]
    fn another_codec_an_encoder_error_or_a_huge_packet_is_refused() {
        assert_eq!(Reader::new().push(&codec(CODEC_H265)), Err(Error::Codec(CODEC_H265)));
        assert_eq!(Reader::new().push(&codec(CODEC_ERROR)), Err(Error::Encoder));
        assert!(Error::Codec(CODEC_AV1).to_string().contains("\"av1\""));
        let mut s = codec(CODEC_H264).to_vec();
        s.extend(0u64.to_be_bytes());
        s.extend((MAX_PACKET + 1).to_be_bytes());
        assert_eq!(Reader::new().push(&s), Err(Error::TooLarge(MAX_PACKET + 1)));
    }

    #[test]
    fn nal_units_split_on_three_and_four_byte_start_codes() {
        let d = [0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 3, 0];
        let n = nal_units(&d);
        assert_eq!(n, vec![&[0x67, 1][..], &[0x68, 2][..], &[0x65, 3, 0][..]]);
        assert_eq!(parameter_sets(&d), Some((&[0x67, 1][..], &[0x68, 2][..])));
        assert_eq!(parameter_sets(&[0, 0, 1, 0x65]), None);
    }

    #[test]
    fn avcc_and_annex_b_convert_both_ways_without_parameter_sets() {
        let annex = [0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 3, 4, 0, 0, 1, 0x41, 5];
        let avcc = to_avcc(&annex);
        assert_eq!(avcc, [0, 0, 0, 3, 0x65, 3, 4, 0, 0, 0, 2, 0x41, 5]);
        assert_eq!(avcc_to_annex_b(&avcc), [0, 0, 0, 1, 0x65, 3, 4, 0, 0, 0, 1, 0x41, 5]);
        // A truncated length stops the conversion rather than reading past.
        assert_eq!(avcc_to_annex_b(&[0, 0, 0, 9, 1]), Vec::<u8>::new());
    }
}
