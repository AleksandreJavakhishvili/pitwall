//! The simulator's screen to H.264 with VideoToolbox (public API): an
//! IOSurface is wrapped as a CVPixelBuffer (no copy) and encoded by the
//! hardware encoder; each encoded frame goes to the [`Sink`] as Annex B,
//! a config packet (SPS, PPS) before every key frame whose parameter sets
//! changed.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use pitwall_proto::device::video::{avcc_to_annex_b, Packet};

use super::ffi::*;

/// Where encoded packets go (stdout, framed or raw).
pub trait Sink: Send {
    fn packet(&mut self, p: Packet);
}

struct Shared {
    sink: Arc<Mutex<dyn Sink>>,
    /// The parameter sets last sent.
    config: Vec<u8>,
}

pub struct Encoder {
    session: CFTypeRef,
    pub width: u32,
    pub height: u32,
    shared: *mut Mutex<Shared>,
    force_key: bool,
}

// The session is used from the thread that owns the encoder; VideoToolbox
// calls back on its own threads into `shared`, which is behind a mutex.
unsafe impl Send for Encoder {}

extern "C" fn output(refcon: *mut c_void, _frame: *mut c_void, status: OSStatus, _info: u32, sample: *mut c_void) {
    if status != 0 || sample.is_null() {
        return;
    }
    let shared = unsafe { &*(refcon as *const Mutex<Shared>) };
    let Ok(mut shared) = shared.lock() else { return };
    unsafe {
        // A key frame has no NotSync attachment (or it is false).
        let atts = CMSampleBufferGetSampleAttachmentsArray(sample, 0);
        let mut key = true;
        if !atts.is_null() && CFArrayGetCount(atts) > 0 {
            let a = CFArrayGetValueAtIndex(atts, 0);
            let v = CFDictionaryGetValue(a, kCMSampleAttachmentKey_NotSync);
            key = v.is_null() || CFBooleanGetValue(v) == 0;
        }
        if key {
            let fd = CMSampleBufferGetFormatDescription(sample);
            let mut config = Vec::new();
            for i in 0..2 {
                let (mut ptr, mut size, mut count, mut header) = (std::ptr::null(), 0usize, 0usize, 0i32);
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fd, i, &mut ptr, &mut size, &mut count, &mut header) == 0 {
                    config.extend_from_slice(&[0, 0, 0, 1]);
                    config.extend_from_slice(std::slice::from_raw_parts(ptr, size));
                }
            }
            if !config.is_empty() && config != shared.config {
                shared.config = config.clone();
                if let Ok(mut s) = shared.sink.lock() {
                    s.packet(Packet { config: true, key: false, pts: 0, data: config });
                }
            }
        }
        let bb = CMSampleBufferGetDataBuffer(sample);
        if bb.is_null() {
            return;
        }
        let len = CMBlockBufferGetDataLength(bb);
        let mut data = vec![0u8; len];
        if CMBlockBufferCopyDataBytes(bb, 0, len, data.as_mut_ptr() as *mut c_void) != 0 {
            return;
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(sample).micros();
        let sink = shared.sink.clone();
        drop(shared);
        if let Ok(mut s) = sink.lock() {
            s.packet(Packet { config: false, key, pts, data: avcc_to_annex_b(&data) });
        };
    }
}

impl Encoder {
    /// A hardware H.264 encoder for frames of `width` × `height`.
    pub fn new(width: u32, height: u32, fps: u32, sink: Arc<Mutex<dyn Sink>>) -> Result<Encoder, String> {
        let shared = Box::into_raw(Box::new(Mutex::new(Shared { sink, config: Vec::new() })));
        let mut session: CFTypeRef = std::ptr::null();
        let st = unsafe {
            VTCompressionSessionCreate(
                std::ptr::null(),
                width as i32,
                height as i32,
                H264,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                output,
                shared as *mut c_void,
                &mut session,
            )
        };
        if st != 0 || session.is_null() {
            drop(unsafe { Box::from_raw(shared) });
            return Err(format!("VideoToolbox could not make an H.264 encoder ({st})"));
        }
        unsafe {
            VTSessionSetProperty(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
            VTSessionSetProperty(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
            VTSessionSetProperty(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
            // A key frame every 4 s at most, so a decoder that lost its
            // place recovers soon.
            let n = number((fps * 4) as i32);
            VTSessionSetProperty(session, kVTCompressionPropertyKey_MaxKeyFrameInterval, n);
            CFRelease(n);
            let n = number(fps as i32);
            VTSessionSetProperty(session, kVTCompressionPropertyKey_ExpectedFrameRate, n);
            CFRelease(n);
            // About 8 Mbit/s: sharp text on a phone screen.
            let n = number(8_000_000);
            VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, n);
            CFRelease(n);
        }
        Ok(Encoder { session, width, height, shared, force_key: true })
    }

    /// Encodes the surface's current contents at `micros`.
    pub fn encode(&mut self, surface: CFTypeRef, micros: u64) -> Result<(), String> {
        unsafe {
            let mut pb: CFTypeRef = std::ptr::null();
            let st = CVPixelBufferCreateWithIOSurface(std::ptr::null(), surface, std::ptr::null(), &mut pb);
            if st != 0 || pb.is_null() {
                return Err(format!("the screen's surface could not be read ({st})"));
            }
            let props = if self.force_key { dict1(kVTEncodeFrameOptionKey_ForceKeyFrame, kCFBooleanTrue) } else { std::ptr::null() };
            self.force_key = false;
            let st = VTCompressionSessionEncodeFrame(self.session, pb, CMTime::new(micros as i64, 1_000_000), kCMTimeInvalid, props, std::ptr::null_mut(), std::ptr::null_mut());
            if !props.is_null() {
                CFRelease(props);
            }
            CFRelease(pb);
            if st != 0 {
                return Err(format!("the encoder refused a frame ({st})"));
            }
        }
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            VTCompressionSessionCompleteFrames(self.session, kCMTimeInvalid);
            VTCompressionSessionInvalidate(self.session);
            CFRelease(self.session);
            drop(Box::from_raw(self.shared));
        }
    }
}
