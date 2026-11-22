//! Secrets in the Windows Credential Manager: generic credentials named
//! `<SERVICE>/<account>`, kept for this user on this machine.

use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, FILETIME};
use windows_sys::Win32::Security::Credentials::{CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

use super::secrets::{SecretStore, SERVICE};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn target(account: &str) -> Vec<u16> {
    wide(&format!("{SERVICE}/{account}"))
}

fn failed(what: &str) -> String {
    format!("The Credential Manager couldn't {what} the secret (error {}).", unsafe { GetLastError() })
}

pub struct CredentialManager;

impl SecretStore for CredentialManager {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let name = target(account);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
        if ok == 0 {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND { Ok(None) } else { Err(failed("read")) };
        }
        let bytes = unsafe {
            let c = &*cred;
            let b = if c.CredentialBlob.is_null() { Vec::new() } else { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec() };
            CredFree(cred as *const _);
            b
        };
        String::from_utf8(bytes).map(Some).map_err(|_| "The saved secret isn't text.".to_string())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let mut name = target(account);
        let mut user = wide("Pitwall");
        let blob = secret.as_bytes();
        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            Comment: std::ptr::null_mut(),
            LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: user.as_mut_ptr(),
        };
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(failed("save"));
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let name = target(account);
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 && unsafe { GetLastError() } != ERROR_NOT_FOUND {
            return Err(failed("remove"));
        }
        Ok(())
    }
}
