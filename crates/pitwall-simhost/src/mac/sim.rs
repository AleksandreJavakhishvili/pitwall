//! Xcode's private simulator interfaces, loaded at run time from the
//! user's Xcode and reached only by name (no headers, no link-time
//! symbols), so a missing one is an error here, never a crash at load.
//!
//! What is used, and from where (docs/spec/device.md, "Private
//! interfaces"; verified with Xcode 26.1.1):
//! - CoreSimulator (`/Library/Developer/PrivateFrameworks`):
//!   `SimServiceContext +sharedServiceContextForDeveloperDir:error:`,
//!   `-defaultDeviceSetWithError:`, `-deviceSetWithPath:error:`;
//!   `SimDeviceSet -devices`; `SimDevice -UDID -name -state -io`;
//!   `SimDeviceIOClient -ioPorts`; a port's `-descriptor`, its `-state`
//!   (`-displayClass`, 0 = the main screen) and `-framebufferSurface`
//!   (an IOSurface).
//! - SimulatorKit (`<Xcode>/Contents/Developer/Library/PrivateFrameworks`):
//!   the class `SimulatorKit.SimDeviceLegacyHIDClient`
//!   (`-initWithDevice:error:`,
//!   `-sendWithMessage:freeWhenDone:completionQueue:completion:`) and the
//!   C functions `IndigoHIDMessageForMouseNSEvent`,
//!   `IndigoHIDMessageForKeyboardArbitrary`, `IndigoHIDMessageForButton`.
//!
//! How these fit together was learned from Meta's idb (MIT,
//! github.com/facebook/idb: FBSimulatorControl/Framebuffer and /HID,
//! PrivateHeaders/SimulatorApp/Indigo.h); the code here is Pitwall's own.

use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};

use block2::RcBlock;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};

use super::ffi::*;

/// `SimDeviceState` booted.
const BOOTED: u64 = 3;

fn nsstring(s: &str) -> *mut AnyObject {
    let c = CString::new(s).unwrap_or_default();
    let cls = AnyClass::get(c"NSString").expect("Foundation");
    unsafe { msg_send![cls, stringWithUTF8String: c.as_ptr()] }
}

fn string(o: *mut AnyObject) -> String {
    if o.is_null() {
        return String::new();
    }
    let p: *const std::ffi::c_char = unsafe { msg_send![o, UTF8String] };
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

extern "C" {
    fn objc_msgSend();
}

/// `[o s]` returning an object, sent without objc2's method check: the
/// port descriptors are XPC proxies (ROCKRemoteProxy) that answer
/// `respondsToSelector:` but forward the call, so the runtime has no
/// method to check the types against.
unsafe fn id0(o: *mut AnyObject, s: Sel) -> *mut AnyObject {
    let f: unsafe extern "C" fn(*mut AnyObject, Sel) -> *mut AnyObject = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(o, s)
}

/// `[o s]` returning an unsigned short, as [`id0`].
unsafe fn u16_0(o: *mut AnyObject, s: Sel) -> u16 {
    let f: unsafe extern "C" fn(*mut AnyObject, Sel) -> u16 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(o, s)
}

fn responds(o: *mut AnyObject, s: Sel) -> bool {
    !o.is_null() && unsafe { msg_send![o, respondsToSelector: s] }
}

fn error_text(e: *mut AnyObject) -> String {
    if e.is_null() {
        return "no reason given".into();
    }
    string(unsafe { msg_send![e, localizedDescription] })
}

/// The Xcode in use: `DEVELOPER_DIR`, else `xcode-select -p`.
pub fn developer_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DEVELOPER_DIR") {
        return Some(PathBuf::from(d));
    }
    let out = std::process::Command::new("/usr/bin/xcode-select").arg("-p").output().ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// That Xcode's version (`CFBundleShortVersionString` of its Info.plist).
pub fn xcode_version(dev: &Path) -> String {
    let plist = dev.parent().map(|c| c.join("Info.plist")).unwrap_or_default();
    std::process::Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(plist.with_extension(""))
        .arg("CFBundleShortVersionString")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// The loaded frameworks (kept open for the process's life).
pub struct Frameworks {
    pub dev: PathBuf,
    pub xcode: String,
    _core: libloading::Library,
    kit: Option<libloading::Library>,
    /// Why SimulatorKit (input) isn't there.
    pub kit_error: Option<String>,
}

impl Frameworks {
    pub fn load() -> Result<Frameworks, String> {
        let dev = developer_dir().ok_or("no Xcode found (xcode-select -p)")?;
        let xcode = xcode_version(&dev);
        let core_paths = [
            PathBuf::from("/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator"),
            dev.join("Library/PrivateFrameworks/CoreSimulator.framework/CoreSimulator"),
        ];
        let core = core_paths
            .iter()
            .find_map(|p| unsafe { libloading::Library::new(p) }.ok())
            .ok_or_else(|| format!("CoreSimulator isn't installed (Xcode {xcode})"))?;
        let kit_path = dev.join("Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit");
        let (kit, kit_error) = match unsafe { libloading::Library::new(&kit_path) } {
            Ok(l) => (Some(l), None),
            Err(e) => (None, Some(format!("SimulatorKit could not be loaded from Xcode {xcode}: {e}"))),
        };
        Ok(Frameworks { dev, xcode, _core: core, kit, kit_error })
    }
}

/// A simulator.
pub struct Device {
    pub obj: *mut AnyObject,
    pub name: String,
}

impl Device {
    /// The device `udid` in the default set, or in the set at `set`.
    pub fn find(fw: &Frameworks, udid: &str, set: Option<&Path>) -> Result<Device, String> {
        let cls = AnyClass::get(c"SimServiceContext").ok_or("this Xcode's CoreSimulator has no SimServiceContext")?;
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let dev = nsstring(&fw.dev.to_string_lossy());
        let ctx: *mut AnyObject = unsafe { msg_send![cls, sharedServiceContextForDeveloperDir: dev, error: &mut err] };
        if ctx.is_null() {
            return Err(format!("CoreSimulator didn't start: {}", error_text(err)));
        }
        let devset: *mut AnyObject = match set {
            Some(p) => unsafe { msg_send![ctx, deviceSetWithPath: nsstring(&p.to_string_lossy()), error: &mut err] },
            None => unsafe { msg_send![ctx, defaultDeviceSetWithError: &mut err] },
        };
        if devset.is_null() {
            return Err(format!("no simulator device set: {}", error_text(err)));
        }
        let devices: *mut AnyObject = unsafe { msg_send![devset, devices] };
        let n: usize = unsafe { msg_send![devices, count] };
        for i in 0..n {
            let d: *mut AnyObject = unsafe { msg_send![devices, objectAtIndex: i] };
            let id: *mut AnyObject = unsafe { msg_send![d, UDID] };
            let id = string(unsafe { msg_send![id, UUIDString] });
            if id.eq_ignore_ascii_case(udid) {
                let name = string(unsafe { msg_send![d, name] });
                return Ok(Device { obj: d, name });
            }
        }
        Err(format!("no simulator {udid}"))
    }

    pub fn booted(&self) -> bool {
        let s: u64 = unsafe { msg_send![self.obj, state] };
        s == BOOTED
    }

    /// The main screen's framebuffer source (a port descriptor), if the
    /// simulator has one.
    pub fn screen(&self) -> Result<Screen, String> {
        if !responds(self.obj, sel!(io)) {
            return Err("this Xcode's SimDevice has no io".into());
        }
        let io: *mut AnyObject = unsafe { msg_send![self.obj, io] };
        if !responds(io, sel!(ioPorts)) {
            return Err("this Xcode's SimDeviceIOClient has no ioPorts".into());
        }
        let ports: *mut AnyObject = unsafe { msg_send![io, ioPorts] };
        let n: usize = if ports.is_null() { 0 } else { unsafe { msg_send![ports, count] } };
        let mut fallback = None;
        for i in 0..n {
            let port: *mut AnyObject = unsafe { msg_send![ports, objectAtIndex: i] };
            if !responds(port, sel!(descriptor)) {
                continue;
            }
            let desc = unsafe { id0(port, sel!(descriptor)) };
            if !responds(desc, sel!(framebufferSurface)) {
                continue;
            }
            let class = if responds(desc, sel!(state)) {
                let st = unsafe { id0(desc, sel!(state)) };
                if responds(st, sel!(displayClass)) {
                    let c = unsafe { u16_0(st, sel!(displayClass)) };
                    Some(c)
                } else {
                    None
                }
            } else {
                None
            };
            if class == Some(0) {
                return Ok(Screen { desc });
            }
            fallback.get_or_insert(Screen { desc });
        }
        fallback.ok_or_else(|| "the simulator has no screen to show (is it booted?)".into())
    }
}

pub struct Screen {
    desc: *mut AnyObject,
}

impl Screen {
    /// The surface the simulator draws its screen into now (it changes
    /// on rotation), retained by the caller's autorelease pool.
    pub fn surface(&self) -> Option<CFTypeRef> {
        let s = unsafe { id0(self.desc, sel!(framebufferSurface)) };
        (!s.is_null()).then_some(s as CFTypeRef)
    }
}

type MouseFn = unsafe extern "C" fn(*mut CGPoint, *mut CGPoint, u32, u64, CGSize, u32) -> *mut u8;
type KeyFn = unsafe extern "C" fn(i32, i32) -> *mut u8;
type ButtonFn = unsafe extern "C" fn(i32, i32, i32) -> *mut u8;

/// Input into the simulator, through SimulatorKit's HID client.
pub struct Hid {
    client: *mut AnyObject,
    queue: *mut c_void,
    mouse: MouseFn,
    key: KeyFn,
    button: ButtonFn,
}

unsafe impl Send for Hid {}

// Offsets in an Indigo message (idb's Indigo.h, pack 4).
const INNER_SIZE: usize = 0x18;
const EVENT_TYPE: usize = 0x1c;
const PAYLOAD: usize = 0x20;
const EVENT_KIND: usize = 0x20;
const TIMESTAMP: usize = 0x24;
const TOUCH: usize = 0x30;
const X_RATIO: usize = 0x3c;
const Y_RATIO: usize = 0x44;
/// `sizeof(IndigoTouch)` and `sizeof(IndigoPayload)` as idb lays them.
const TOUCH_SIZE: usize = 0x70;
const PAYLOAD_SIZE: usize = 0x90;
const MESSAGE_SIZE: usize = 0xB0 + PAYLOAD_SIZE;
const DIGITIZER: u32 = 0x32;
const HARDWARE: i32 = 0x33;

impl Hid {
    pub fn new(fw: &Frameworks, device: &Device) -> Result<Hid, String> {
        let kit = fw.kit.as_ref().ok_or_else(|| fw.kit_error.clone().unwrap_or_default())?;
        let sym = |name: &[u8]| -> Result<*const c_void, String> {
            unsafe { kit.get::<*const c_void>(name) }.map(|s| *s).map_err(|_| {
                format!("this Xcode ({}) has no {} in SimulatorKit", fw.xcode, String::from_utf8_lossy(&name[..name.len() - 1]))
            })
        };
        let mouse: MouseFn = unsafe { std::mem::transmute(sym(b"IndigoHIDMessageForMouseNSEvent\0")?) };
        let key: KeyFn = unsafe { std::mem::transmute(sym(b"IndigoHIDMessageForKeyboardArbitrary\0")?) };
        let button: ButtonFn = unsafe { std::mem::transmute(sym(b"IndigoHIDMessageForButton\0")?) };
        let cls = AnyClass::get(c"SimulatorKit.SimDeviceLegacyHIDClient")
            .ok_or_else(|| format!("this Xcode ({}) has no SimDeviceLegacyHIDClient", fw.xcode))?;
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let client: *mut AnyObject = unsafe {
            let a: *mut AnyObject = msg_send![cls, alloc];
            msg_send![a, initWithDevice: device.obj, error: &mut err]
        };
        if client.is_null() {
            return Err(format!("the simulator's HID client didn't start: {}", error_text(err)));
        }
        let queue = unsafe { dispatch_queue_create(c"pitwall.simhost.hid".as_ptr(), std::ptr::null()) };
        Ok(Hid { client, queue, mouse, key, button })
    }

    /// Sends a malloc'd Indigo message (the client frees it).
    fn send(&self, msg: *mut u8) {
        if msg.is_null() {
            return;
        }
        let done = RcBlock::new(|_err: *mut AnyObject| {});
        // Sent raw: the method's type encoding names SimulatorKit's own
        // message struct, which objc2's check can't match a pointer to.
        type Send = unsafe extern "C" fn(*mut AnyObject, Sel, *mut c_void, bool, *mut c_void, *const c_void);
        unsafe {
            let f: Send = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let block: &block2::Block<dyn Fn(*mut AnyObject)> = &done;
            f(self.client, sel!(sendWithMessage:freeWhenDone:completionQueue:completion:), msg as *mut c_void, true, self.queue, block as *const _ as *const c_void);
        }
    }

    /// One finger down (or moved, still down) or up at a screen ratio.
    pub fn touch(&self, down: bool, x: f64, y: f64) {
        unsafe {
            let mut p = CGPoint { x, y };
            // NSEventTypeLeftMouseDown / Up: the builder's "direction".
            let src = (self.mouse)(&mut p, std::ptr::null_mut(), DIGITIZER, if down { 1 } else { 2 }, CGSize { width: 1.0, height: 1.0 }, 0);
            if src.is_null() {
                return;
            }
            std::ptr::write_unaligned(src.add(X_RATIO) as *mut f64, x);
            std::ptr::write_unaligned(src.add(Y_RATIO) as *mut f64, y);
            // The builder makes a multi-touch message; the guest wants a
            // single touch: its contact, enveloped again, twice.
            let m = calloc(1, MESSAGE_SIZE) as *mut u8;
            if m.is_null() {
                free(src as *mut c_void);
                return;
            }
            std::ptr::write_unaligned(m.add(INNER_SIZE) as *mut u32, PAYLOAD_SIZE as u32);
            *m.add(EVENT_TYPE) = 2;
            std::ptr::write_unaligned(m.add(EVENT_KIND) as *mut u32, 0xB);
            std::ptr::write_unaligned(m.add(TIMESTAMP) as *mut u64, mach_absolute_time());
            std::ptr::copy_nonoverlapping(src.add(TOUCH), m.add(TOUCH), TOUCH_SIZE);
            free(src as *mut c_void);
            std::ptr::copy_nonoverlapping(m.add(PAYLOAD), m.add(PAYLOAD + PAYLOAD_SIZE), PAYLOAD_SIZE);
            let second = m.add(PAYLOAD + PAYLOAD_SIZE + 0x10);
            std::ptr::write_unaligned(second as *mut u32, 1);
            std::ptr::write_unaligned(second.add(4) as *mut u32, 2);
            self.send(m);
        }
    }

    /// A key of the HID keyboard page.
    pub fn key(&self, down: bool, usage: u16) {
        let m = unsafe { (self.key)(usage as i32, if down { 1 } else { 2 }) };
        self.send(m);
    }

    /// A hardware button (Indigo's button event sources).
    pub fn button(&self, source: i32, down: bool) {
        let m = unsafe { (self.button)(source, if down { 1 } else { 2 }, HARDWARE) };
        self.send(m);
    }
}

/// Indigo's event source for a button.
pub fn button_source(b: pitwall_proto::device::sim::Button) -> i32 {
    use pitwall_proto::device::sim::Button;
    match b {
        Button::Home => 0x0,
        Button::Lock => 0x1,
        Button::Siri => 0x400002,
    }
}
