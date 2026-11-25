//! VideoToolbox H.264 decoding (public API) for device tiles
//! ([`super::video`]). Synchronous: each frame is decoded before
//! `decode` returns, on the caller's (background) thread.

#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::CFRetained;
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetWidth};
use pitwall_proto::device::video::{parameter_sets, to_avcc};

use super::secrets_macos::Opaque;
use super::video::Frame;

type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
struct SampleTiming {
    duration: CMTime,
    pts: CMTime,
    dts: CMTime,
}

type OutputCallback = extern "C" fn(*mut c_void, *mut c_void, i32, u32, CFTypeRef, CMTime, CMTime);

#[repr(C)]
struct CallbackRecord {
    callback: OutputCallback,
    refcon: *mut c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, kcb: *const Opaque, vcb: *const Opaque) -> CFTypeRef;
    fn CFNumberCreate(alloc: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFTypeRef;
    static kCVPixelBufferIOSurfacePropertiesKey: CFTypeRef;
    static kCVPixelBufferMetalCompatibilityKey: CFTypeRef;
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    static kCMTimeInvalid: CMTime;
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(alloc: CFTypeRef, count: usize, ptrs: *const *const u8, sizes: *const usize, nal_header_len: i32, out: *mut CFTypeRef) -> i32;
    fn CMBlockBufferCreateWithMemoryBlock(alloc: CFTypeRef, block: *mut c_void, len: usize, block_alloc: CFTypeRef, custom: *const c_void, offset: usize, data_len: usize, flags: u32, out: *mut CFTypeRef) -> i32;
    fn CMBlockBufferReplaceDataBytes(src: *const c_void, dest: CFTypeRef, offset: usize, len: usize) -> i32;
    fn CMSampleBufferCreateReady(alloc: CFTypeRef, data: CFTypeRef, fd: CFTypeRef, n: isize, timing_n: isize, timing: *const SampleTiming, size_n: isize, sizes: *const usize, out: *mut CFTypeRef) -> i32;
}

#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    fn VTDecompressionSessionCreate(alloc: CFTypeRef, fd: CFTypeRef, decoder_spec: CFTypeRef, dest_attrs: CFTypeRef, callback: *const CallbackRecord, out: *mut CFTypeRef) -> i32;
    fn VTDecompressionSessionDecodeFrame(session: CFTypeRef, sample: CFTypeRef, flags: u32, frame_refcon: *mut c_void, info: *mut u32) -> i32;
    fn VTDecompressionSessionWaitForAsynchronousFrames(session: CFTypeRef) -> i32;
    fn VTDecompressionSessionInvalidate(session: CFTypeRef);
}

/// `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`: what GPUI's Metal
/// renderer draws surfaces from.
const FORMAT_420F: i32 = i32::from_be_bytes(*b"420f");

/// Where the callback leaves the picture of the frame being decoded.
type Slot = Option<CFRetained<CVPixelBuffer>>;

extern "C" fn output(refcon: *mut c_void, _frame: *mut c_void, status: i32, _flags: u32, image: CFTypeRef, _pts: CMTime, _dur: CMTime) {
    if status != 0 || image.is_null() {
        return;
    }
    let slot = unsafe { &mut *(refcon as *mut Slot) };
    // Retained here; the CFRetained releases it.
    let retained = unsafe { CFRetain(image) } as *mut CVPixelBuffer;
    *slot = NonNull::new(retained).map(|p| unsafe { CFRetained::from_raw(p) });
}

pub struct Decoder {
    session: CFTypeRef,
    format: CFTypeRef,
    slot: Box<Slot>,
    record: Box<CallbackRecord>,
}

// Used from one thread at a time (the tile's stream thread).
unsafe impl Send for Decoder {}

fn dict(keys: &[CFTypeRef], values: &[CFTypeRef]) -> CFTypeRef {
    unsafe { CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), keys.len() as isize, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks) }
}

impl Decoder {
    pub fn new() -> Decoder {
        let mut slot: Box<Slot> = Box::new(None);
        let refcon = &mut *slot as *mut Slot as *mut c_void;
        Decoder { session: std::ptr::null(), format: std::ptr::null(), slot, record: Box::new(CallbackRecord { callback: output, refcon }) }
    }

    fn close(&mut self) {
        unsafe {
            if !self.session.is_null() {
                VTDecompressionSessionWaitForAsynchronousFrames(self.session);
                VTDecompressionSessionInvalidate(self.session);
                CFRelease(self.session);
                self.session = std::ptr::null();
            }
            if !self.format.is_null() {
                CFRelease(self.format);
                self.format = std::ptr::null();
            }
        }
    }

    pub fn config(&mut self, data: &[u8]) -> Result<(), String> {
        let (sps, pps) = parameter_sets(data).ok_or("the stream's config has no SPS and PPS")?;
        self.close();
        unsafe {
            let ptrs = [sps.as_ptr(), pps.as_ptr()];
            let sizes = [sps.len(), pps.len()];
            let mut fd: CFTypeRef = std::ptr::null();
            let st = CMVideoFormatDescriptionCreateFromH264ParameterSets(std::ptr::null(), 2, ptrs.as_ptr(), sizes.as_ptr(), 4, &mut fd);
            if st != 0 || fd.is_null() {
                return Err(format!("the stream's parameter sets can't be read ({st})"));
            }
            self.format = fd;
            let fmt = FORMAT_420F;
            let n = CFNumberCreate(std::ptr::null(), 3, &fmt as *const i32 as *const c_void);
            let empty = dict(&[], &[]);
            let attrs = dict(
                &[kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey],
                &[n, empty, kCFBooleanTrue],
            );
            CFRelease(n);
            CFRelease(empty);
            let mut session: CFTypeRef = std::ptr::null();
            let st = VTDecompressionSessionCreate(std::ptr::null(), fd, std::ptr::null(), attrs, &*self.record, &mut session);
            CFRelease(attrs);
            if st != 0 || session.is_null() {
                self.close();
                return Err(format!("VideoToolbox can't decode this stream ({st})"));
            }
            self.session = session;
        }
        Ok(())
    }

    pub fn decode(&mut self, data: &[u8], pts: u64) -> Result<Option<Frame>, String> {
        if self.session.is_null() {
            return Err("a frame came before the stream's config".into());
        }
        let avcc = to_avcc(data);
        if avcc.is_empty() {
            return Ok(None);
        }
        unsafe {
            let mut bb: CFTypeRef = std::ptr::null();
            let st = CMBlockBufferCreateWithMemoryBlock(std::ptr::null(), std::ptr::null_mut(), avcc.len(), std::ptr::null(), std::ptr::null(), 0, avcc.len(), 0, &mut bb);
            if st != 0 || bb.is_null() {
                return Err(format!("no memory for a frame ({st})"));
            }
            if CMBlockBufferReplaceDataBytes(avcc.as_ptr() as *const c_void, bb, 0, avcc.len()) != 0 {
                CFRelease(bb);
                return Err("a frame couldn't be copied".into());
            }
            let timing = SampleTiming { duration: kCMTimeInvalid, pts: CMTime { value: pts as i64, timescale: 1_000_000, flags: 1, epoch: 0 }, dts: kCMTimeInvalid };
            let size = avcc.len();
            let mut sb: CFTypeRef = std::ptr::null();
            let st = CMSampleBufferCreateReady(std::ptr::null(), bb, self.format, 1, 1, &timing, 1, &size, &mut sb);
            CFRelease(bb);
            if st != 0 || sb.is_null() {
                return Err(format!("a frame couldn't be wrapped ({st})"));
            }
            *self.slot = None;
            let mut info = 0u32;
            let st = VTDecompressionSessionDecodeFrame(self.session, sb, 0, std::ptr::null_mut(), &mut info);
            CFRelease(sb);
            if st != 0 {
                return Err(format!("VideoToolbox couldn't decode a frame ({st})"));
            }
        }
        Ok(self.slot.take().map(|buffer| Frame { width: CVPixelBufferGetWidth(&buffer) as u32, height: CVPixelBufferGetHeight(&buffer) as u32, buffer }))
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.close();
    }
}
