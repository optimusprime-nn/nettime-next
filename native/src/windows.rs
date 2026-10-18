//! Small Windows helpers. Loading this module does not grant clock privileges.
use anyhow::{Result, ensure};
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE},
    Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn check(ok: i32) -> Result<()> {
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub fn require_admin() -> Result<()> {
    ensure!(
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0,
        "Administrator approval is required for this service action"
    );
    Ok(())
}
pub fn enable_clock_privilege() -> Result<()> {
    enable_privilege("SeSystemtimePrivilege")
}
fn enable_privilege(name: &str) -> Result<()> {
    unsafe {
        let mut raw = std::ptr::null_mut();
        check(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut raw,
        ))?;
        let token = Handle(raw);
        let mut state: TOKEN_PRIVILEGES = std::mem::zeroed();
        state.PrivilegeCount = 1;
        check(LookupPrivilegeValueW(
            std::ptr::null(),
            wide(name).as_ptr(),
            &mut state.Privileges[0].Luid,
        ))?;
        state.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
        check(AdjustTokenPrivileges(
            token.0,
            0,
            &state,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ))?;
        ensure!(
            GetLastError() != 1300,
            "Process token lacks requested privilege {name}"
        );
    }
    Ok(())
}
/// The file is flushed before an atomic, write-through rename. Parent ACLs are inherited.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|e| anyhow::anyhow!("Cannot create atomic-write nonce: {e}"))?;
    let temporary = path.with_extension(format!("{}.tmp", hex::encode(nonce)));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let outcome = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        check(unsafe {
            MoveFileExW(
                wide(&temporary).as_ptr(),
                wide(path).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        })
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

pub fn utc_label(utc: f64) -> String {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(file: *const FILETIME, time: *mut SYSTEMTIME) -> i32;
    }
    if !utc.is_finite() {
        return "—".into();
    }
    let ticks = ((utc + 11644473600.) * 10_000_000.) as u64;
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&file, &mut time) } == 0 {
        return "—".into();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

/// Replace inherited/user-created ACLs before trusting machine configuration.
pub fn protect_service_files(root: &Path, account: &str) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
            LookupAccountNameW, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        },
    };
    enable_privilege("SeRestorePrivilege")?;
    enable_privilege("SeTakeOwnershipPrivilege")?;
    let name = wide(account);
    let (mut size, mut domain_size, mut kind) = (0, 0, 0);
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            &mut domain_size,
            &mut kind,
        );
    }
    ensure!(size > 0, "Cannot resolve the virtual service identity");
    let mut sid = vec![0u32; (size as usize).div_ceil(4)];
    let mut domain = vec![0u16; domain_size as usize];
    check(unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut kind,
        )
    })?;
    let mut text = std::ptr::null_mut();
    check(unsafe { ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut text) })?;
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let account_sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    unsafe {
        LocalFree(text.cast());
    }
    fn apply(path: &Path, sddl: &str) -> Result<()> {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            std::fs::symlink_metadata(path)?.file_attributes() & 0x400 == 0,
            "Service paths must not be reparse points: {}",
            path.display()
        );
        unsafe {
            let mut sd = std::ptr::null_mut();
            check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(sddl).as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            ))?;
            let (mut present, mut defaulted, mut acl, mut owner) =
                (0, 0, std::ptr::null_mut(), std::ptr::null_mut());
            let result = (|| -> Result<()> {
                check(GetSecurityDescriptorDacl(
                    sd,
                    &mut present,
                    &mut acl,
                    &mut defaulted,
                ))?;
                check(GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted))?;
                ensure!(
                    present != 0 && !acl.is_null() && !owner.is_null(),
                    "Service ACL/owner is missing"
                );
                let status = SetNamedSecurityInfoW(
                    wide(path).as_ptr(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION
                        | DACL_SECURITY_INFORMATION
                        | PROTECTED_DACL_SECURITY_INFORMATION,
                    owner,
                    std::ptr::null_mut(),
                    acl,
                    std::ptr::null_mut(),
                );
                ensure!(
                    status == 0,
                    "Cannot protect service path {}: Windows error {status}",
                    path.display()
                );
                Ok(())
            })();
            LocalFree(sd);
            result
        }
    }
    let read = format!(
        "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)(A;OICI;FRFX;;;LS)(A;OICI;FRFX;;;{account_sid})"
    );
    let data = format!(
        "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)(A;OICI;0x1301bf;;;LS)(A;OICI;0x1301bf;;;{account_sid})"
    );
    std::fs::create_dir_all(root)?;
    apply(root, &read)?;
    let folder = root.join("data");
    std::fs::create_dir_all(&folder)?;
    apply(&folder, &data)?;
    for (directory, acl) in [(root, &read), (folder.as_path(), &data)] {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Service paths must not contain reparse points"
            );
            if metadata.is_file() {
                apply(&entry.path(), acl)?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn utc_is_explicit_and_independent_of_local_timezone() {
        assert_eq!(super::utc_label(0.), "1970-01-01 00:00:00 UTC");
        assert_eq!(super::utc_label(86401.), "1970-01-02 00:00:01 UTC");
    }
}
