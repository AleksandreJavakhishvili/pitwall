//! The public C interfaces the helper uses: CoreFoundation, IOSurface,
//! CoreVideo, CoreMedia and VideoToolbox (all public frameworks, linked by
//! name; nothing private here).

#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;

pub type CFTypeRef = *const c_void;
pub type OSStatus = i32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    /// `value / timescale` seconds, valid.
    pub fn new(value: i64, timescale: i32) -> CMTime {
        CMTime { value, timescale, flags: 1, epoch: 0 }
    }

    /// In microseconds (0 when not valid).
    pub fn micros(&self) -> u64 {
        if self.flags & 1 == 0 || self.timescale <= 0 {
            return 0;
        }
        (self.value as i128 * 1_000_000 / self.timescale as i128).max(0) as u64
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

pub type VTOutputCallback = extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, *mut c_void);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub fn CFRelease(cf: CFTypeRef);
    pub static kCFBooleanTrue: CFTypeRef;
    pub static kCFBooleanFalse: CFTypeRef;
    pub static kCFTypeDictionaryKeyCallBacks: c_void;
    pub static kCFTypeDictionaryValueCallBacks: c_void;
    pub fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, kcb: *const c_void, vcb: *const c_void) -> CFTypeRef;
    pub fn CFNumberCreate(alloc: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
    pub fn CFArrayGetCount(a: CFTypeRef) -> isize;
    pub fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    pub fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    pub fn CFBooleanGetValue(b: CFTypeRef) -> u8;
}

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    pub fn IOSurfaceGetWidth(s: CFTypeRef) -> usize;
    pub fn IOSurfaceGetHeight(s: CFTypeRef) -> usize;
    pub fn IOSurfaceGetSeed(s: CFTypeRef) -> u32;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    pub fn CVPixelBufferCreateWithIOSurface(alloc: CFTypeRef, surface: CFTypeRef, attrs: CFTypeRef, out: *mut CFTypeRef) -> i32;
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    pub static kCMTimeInvalid: CMTime;
    pub static kCMSampleAttachmentKey_NotSync: CFTypeRef;
    pub fn CMSampleBufferGetFormatDescription(sb: *mut c_void) -> CFTypeRef;
    pub fn CMSampleBufferGetDataBuffer(sb: *mut c_void) -> CFTypeRef;
    pub fn CMSampleBufferGetSampleAttachmentsArray(sb: *mut c_void, create: u8) -> CFTypeRef;
    pub fn CMSampleBufferGetPresentationTimeStamp(sb: *mut c_void) -> CMTime;
    pub fn CMBlockBufferGetDataLength(bb: CFTypeRef) -> usize;
    pub fn CMBlockBufferCopyDataBytes(bb: CFTypeRef, offset: usize, len: usize, dest: *mut c_void) -> OSStatus;
    pub fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fd: CFTypeRef, index: usize, ptr: *mut *const u8, size: *mut usize, count: *mut usize, nal_header_len: *mut i32) -> OSStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    pub static kVTCompressionPropertyKey_RealTime: CFTypeRef;
    pub static kVTCompressionPropertyKey_AllowFrameReordering: CFTypeRef;
    pub static kVTCompressionPropertyKey_ProfileLevel: CFTypeRef;
    pub static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFTypeRef;
    pub static kVTCompressionPropertyKey_ExpectedFrameRate: CFTypeRef;
    pub static kVTCompressionPropertyKey_AverageBitRate: CFTypeRef;
    pub static kVTProfileLevel_H264_High_AutoLevel: CFTypeRef;
    pub static kVTEncodeFrameOptionKey_ForceKeyFrame: CFTypeRef;
    pub fn VTCompressionSessionCreate(
        alloc: CFTypeRef,
        width: i32,
        height: i32,
        codec: u32,
        encoder_spec: CFTypeRef,
        source_attrs: CFTypeRef,
        data_alloc: CFTypeRef,
        callback: VTOutputCallback,
        refcon: *mut c_void,
        out: *mut CFTypeRef,
    ) -> OSStatus;
    pub fn VTSessionSetProperty(session: CFTypeRef, key: CFTypeRef, value: CFTypeRef) -> OSStatus;
    pub fn VTCompressionSessionEncodeFrame(session: CFTypeRef, image: CFTypeRef, pts: CMTime, duration: CMTime, props: CFTypeRef, frame_refcon: *mut c_void, info: *mut u32) -> OSStatus;
    pub fn VTCompressionSessionCompleteFrames(session: CFTypeRef, until: CMTime) -> OSStatus;
    pub fn VTCompressionSessionInvalidate(session: CFTypeRef);
}

extern "C" {
    pub fn mach_absolute_time() -> u64;
    pub fn dispatch_queue_create(label: *const std::ffi::c_char, attr: *const c_void) -> *mut c_void;
    pub fn calloc(n: usize, size: usize) -> *mut c_void;
    pub fn free(p: *mut c_void);
}

/// `kCMVideoCodecType_H264` ('avc1').
pub const H264: u32 = u32::from_be_bytes(*b"avc1");
/// `kCFNumberSInt32Type`.
pub const CF_SINT32: isize = 3;

/// A CF number from an i32 (released by the caller).
pub fn number(v: i32) -> CFTypeRef {
    unsafe { CFNumberCreate(std::ptr::null(), CF_SINT32, &v as *const i32 as *const c_void) }
}

/// A one-entry CF dictionary (released by the caller).
pub fn dict1(key: CFTypeRef, value: CFTypeRef) -> CFTypeRef {
    unsafe { CFDictionaryCreate(std::ptr::null(), &key, &value, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks) }
}
