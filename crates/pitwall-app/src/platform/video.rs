//! H.264 decoding for device tiles (docs/spec/device.md): a stream's
//! config packet (SPS, PPS) and frames in, pictures out.
//!
//! - macOS: VideoToolbox (`video_macos.rs`), hardware, into IOSurface
//!   backed `420f` pixel buffers, the format GPUI's `surface()` draws
//!   without a copy.
//! - Windows, Linux: not built (Media Foundation and VA-API/openh264 are
//!   the plan, docs/spec/device.md "Other systems"): [`Decoder::new`] says
//!   so and the tile shows it.

/// A decoded picture.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    #[cfg(target_os = "macos")]
    pub buffer: objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer>,
}

// A CVPixelBuffer is reference counted and immutable once decoded: it is
// handed from the decoding thread to the UI thread whole.
unsafe impl Send for Frame {}

/// An H.264 decoder.
pub struct Decoder {
    #[cfg(target_os = "macos")]
    inner: super::video_macos::Decoder,
}

impl Decoder {
    /// A decoder, or why there is none on this system.
    pub fn new() -> Result<Decoder, String> {
        #[cfg(target_os = "macos")]
        {
            Ok(Decoder { inner: super::video_macos::Decoder::new() })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(UNSUPPORTED.into())
        }
    }

    /// The parameter sets that the next frames use (Annex B).
    pub fn config(&mut self, data: &[u8]) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.inner.config(data)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = data;
            Err(UNSUPPORTED.into())
        }
    }

    /// One access unit (Annex B); its picture, if it made one.
    pub fn decode(&mut self, data: &[u8], pts: u64) -> Result<Option<Frame>, String> {
        #[cfg(target_os = "macos")]
        {
            self.inner.decode(data, pts)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (data, pts);
            Err(UNSUPPORTED.into())
        }
    }
}

#[cfg(not(target_os = "macos"))]
const UNSUPPORTED: &str = "Device screens need a video decoder Pitwall doesn't have on this system yet";

/// The picture as an element filling its parent, its aspect kept
/// (letterboxed, as `device::geometry::fit` assumes).
pub fn picture(frame: &Frame) -> gpui::AnyElement {
    use gpui::{IntoElement, Styled};
    #[cfg(target_os = "macos")]
    {
        gpui::surface(frame.buffer.clone()).object_fit(gpui::ObjectFit::Contain).size_full().into_any_element()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        gpui::div().size_full().into_any_element()
    }
}

/// Whether device screens can be shown here.
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use pitwall_proto::device::video::{nal_type, nal_units, NAL_PPS, NAL_SPS};

    /// Three frames of ffmpeg's test pattern, 96×64, x264 baseline,
    /// Annex B (made with `ffmpeg -f lavfi -i testsrc=size=96x64:rate=10
    /// -frames:v 3 -c:v libx264 -profile:v baseline -f h264`).
    const FIXTURE: &[u8] = include_bytes!("../device/testdata/testsrc-96x64.h264");

    #[test]
    fn a_real_h264_stream_decodes_to_420f_surfaces_of_its_size() {
        let nals = nal_units(FIXTURE);
        let mut config = Vec::new();
        let mut frames = Vec::new();
        for n in &nals {
            let mut b = vec![0, 0, 0, 1];
            b.extend_from_slice(n);
            match nal_type(n) {
                NAL_SPS | NAL_PPS => config.extend(b),
                1 | 5 => frames.push(b),
                _ => {}
            }
        }
        assert_eq!(frames.len(), 3);
        let mut d = Decoder::new().unwrap();
        // A frame before any config: refused, not a crash.
        assert!(d.decode(&frames[0], 0).is_err());
        d.config(&config).unwrap();
        let mut got = 0;
        for (i, f) in frames.iter().enumerate() {
            if let Some(p) = d.decode(f, i as u64 * 100_000).unwrap() {
                assert_eq!((p.width, p.height), (96, 64));
                assert_eq!(objc2_core_video::CVPixelBufferGetPixelFormatType(&p.buffer), u32::from_be_bytes(*b"420f"));
                got += 1;
            }
        }
        assert_eq!(got, 3);
        // Garbage is an error, never a crash.
        assert!(d.config(&[0, 0, 0, 1, 0x67, 0xff]).is_err() || d.decode(&[0, 0, 0, 1, 0x65, 1, 2, 3], 0).map(|f| f.is_none()).unwrap_or(true));
    }
}
