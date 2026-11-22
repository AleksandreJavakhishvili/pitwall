//! Secrets in the login Keychain: generic passwords with service
//! [`SERVICE`] and the account name given, through Security.framework's
//! `SecItem*` calls (the app's own items; macOS may ask the user once when
//! a differently signed build reads them).

use std::ffi::c_void;

use super::secrets::{SecretStore, SERVICE};

type CFTypeRef = *const c_void;
type OSStatus = i32;

#[repr(C)]
struct Opaque {
    _p: [u8; 0],
}

const UTF8: u32 = 0x0800_0100;
const NOT_FOUND: OSStatus = -25300;
const DUPLICATE: OSStatus = -25299;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    static kCFBooleanTrue: CFTypeRef;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, kcb: *const Opaque, vcb: *const Opaque) -> CFTypeRef;
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: isize, encoding: u32, external: u8) -> CFTypeRef;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFTypeRef;
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    fn CFDataGetLength(data: CFTypeRef) -> isize;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecClass: CFTypeRef;
    static kSecClassGenericPassword: CFTypeRef;
    static kSecAttrService: CFTypeRef;
    static kSecAttrAccount: CFTypeRef;
    static kSecValueData: CFTypeRef;
    static kSecReturnData: CFTypeRef;
    static kSecMatchLimit: CFTypeRef;
    static kSecMatchLimitOne: CFTypeRef;
    fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemUpdate(query: CFTypeRef, update: CFTypeRef) -> OSStatus;
    fn SecItemDelete(query: CFTypeRef) -> OSStatus;
}

/// A CoreFoundation object this code made, released when dropped.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn cf_string(s: &str) -> Owned {
    Owned(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) })
}

fn cf_data(b: &[u8]) -> Owned {
    Owned(unsafe { CFDataCreate(std::ptr::null(), b.as_ptr(), b.len() as isize) })
}

fn dict(pairs: &[(CFTypeRef, CFTypeRef)]) -> Owned {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|p| p.1).collect();
    Owned(unsafe {
        CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), pairs.len() as isize, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks)
    })
}

fn failed(what: &str, status: OSStatus) -> String {
    format!("The Keychain couldn't {what} the secret (error {status}).")
}

pub struct Keychain;

impl Keychain {
    /// The query naming `account`'s item, plus `more`.
    fn query(account: &str, more: &[(CFTypeRef, CFTypeRef)], f: impl FnOnce(CFTypeRef) -> OSStatus) -> OSStatus {
        let service = cf_string(SERVICE);
        let acct = cf_string(account);
        let mut pairs = unsafe { vec![(kSecClass, kSecClassGenericPassword), (kSecAttrService, service.0), (kSecAttrAccount, acct.0)] };
        pairs.extend_from_slice(more);
        let q = dict(&pairs);
        f(q.0)
    }
}

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let mut out: CFTypeRef = std::ptr::null();
        let more = unsafe { [(kSecReturnData, kCFBooleanTrue), (kSecMatchLimit, kSecMatchLimitOne)] };
        let status = Self::query(account, &more, |q| unsafe { SecItemCopyMatching(q, &mut out) });
        let out = Owned(out);
        match status {
            0 => {
                if out.0.is_null() || unsafe { CFGetTypeID(out.0) != CFDataGetTypeID() } {
                    return Err(failed("read", status));
                }
                let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(out.0), CFDataGetLength(out.0) as usize) };
                String::from_utf8(bytes.to_vec()).map(Some).map_err(|_| "The secret in the Keychain isn't text.".to_string())
            }
            NOT_FOUND => Ok(None),
            s => Err(failed("read", s)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let data = cf_data(secret.as_bytes());
        let more = unsafe { [(kSecValueData, data.0)] };
        let status = Self::query(account, &more, |q| unsafe { SecItemAdd(q, std::ptr::null_mut()) });
        match status {
            0 => Ok(()),
            DUPLICATE => {
                let update = unsafe { dict(&[(kSecValueData, data.0)]) };
                match Self::query(account, &[], |q| unsafe { SecItemUpdate(q, update.0) }) {
                    0 => Ok(()),
                    s => Err(failed("save", s)),
                }
            }
            s => Err(failed("save", s)),
        }
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        match Self::query(account, &[], |q| unsafe { SecItemDelete(q) }) {
            0 | NOT_FOUND => Ok(()),
            s => Err(failed("remove", s)),
        }
    }
}
