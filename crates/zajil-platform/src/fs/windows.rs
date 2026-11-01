//! The Windows side of [`super`].

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use std::os::windows::io::{FromRawHandle, RawHandle};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION, ERROR_LOCK_VIOLATION,
    ERROR_NOT_SUPPORTED, ERROR_SHARING_VIOLATION, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
    GetTokenInformation, INHERIT_ONLY_ACE, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetCompressedFileSizeW, GetDiskFreeSpaceExW, GetFileInformationByHandle,
    INVALID_FILE_SIZE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::not_regular;

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::metadata(dir).map(|_| ())
}

pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    // Each missing level, from the top, with an access list for the
    // user alone that what is made inside inherits (review: inheriting
    // the parent's list did not make the folder private).
    let missing: Vec<&Path> = dir
        .ancestors()
        .take_while(|level| std::fs::symlink_metadata(level).is_err())
        .collect();
    for level in missing.into_iter().rev() {
        let (attributes, _descriptor) = owner_only_inherited("FA")?;
        let path_w = wide(&std::path::absolute(level)?);
        // SAFETY: `path_w` is a NUL-terminated UTF-16 string and
        // `attributes` points to a valid descriptor, both alive for
        // the call.
        if unsafe { CreateDirectoryW(path_w.as_ptr(), &attributes) } == 0 {
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::AlreadyExists {
                return Err(err);
            }
        }
    }
    Ok(())
}

pub fn dir_has_acl(_dir: &Path) -> io::Result<bool> {
    // The access list itself is what `only_user_or_root_can_change`
    // reads on Windows.
    Ok(false)
}

pub fn open_private_tool(path: &Path) -> io::Result<File> {
    // Shared for reading only: while it is open, nobody can write,
    // delete or rename it, nor rename a folder above it.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = file.metadata()?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !meta.file_type().is_file()
        || link_count(&file)? > 1
    {
        return Err(not_regular(path));
    }
    if !only_user_or_root_can_change(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} can be changed by another user", path.display()),
        ));
    }
    Ok(file)
}

pub fn make_executable(_file: &File) -> io::Result<()> {
    Ok(())
}

pub fn make_ordinary(_file: &File) -> io::Result<()> {
    Ok(())
}

pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
}

pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    // Without MOVEFILE_REPLACE_EXISTING the call fails if `to` exists;
    // the standard library maps that error to AlreadyExists.
    move_file(from, to, MOVEFILE_WRITE_THROUGH)
}

pub fn is_transient(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error().map(|code| code as u32),
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
    )
}

pub fn open_regular_nofollow(path: &Path, write: bool) -> io::Result<File> {
    // Open the link or junction itself rather than its target.
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = file.metadata()?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !meta.file_type().is_file()
        // 0: replaced or removed after it was opened (see unix).
        || link_count(&file)? > 1
    {
        return Err(not_regular(path));
    }
    Ok(file)
}

pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

pub fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Security attributes that allow `access` (SDDL rights, such as `FA`
/// for all file access or `GA` for all generic access) to the current
/// user only: "D:P(A;;<access>;;;<sid>)", a protected DACL (P: nothing
/// inherited) with one entry allowing (A) it. Kept alive by the
/// returned guard, which must outlive every use of the attributes.
pub(crate) fn owner_only(access: &str) -> io::Result<(SECURITY_ATTRIBUTES, LocalMemory)> {
    security_attributes(&format!(
        "D:P(A;;{access};;;{})",
        UserSid::current()?.to_text()?
    ))
}

/// [`owner_only`] for a folder: what is created inside inherits it
/// (object and container inherit).
fn owner_only_inherited(access: &str) -> io::Result<(SECURITY_ATTRIBUTES, LocalMemory)> {
    security_attributes(&format!(
        "D:P(A;OICI;{access};;;{})",
        UserSid::current()?.to_text()?
    ))
}

fn security_attributes(sddl: &str) -> io::Result<(SECURITY_ATTRIBUTES, LocalMemory)> {
    let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is a NUL-terminated UTF-16 string alive for the
    // call, and `descriptor` a writable pointer; the size is optional.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let guard = LocalMemory(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    Ok((attributes, guard))
}

/// Whether the process `pid` runs as the current user (the other end
/// of a pipe, `zajil_platform::ipc`).
pub(crate) fn runs_as_current_user(pid: u32) -> io::Result<bool> {
    // SAFETY: plain call; a null handle is checked below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(process);
    let (ours, theirs) = (UserSid::current()?, UserSid::of_process(process.0)?);
    // SAFETY: both are valid SIDs, alive for the call.
    Ok(unsafe { EqualSid(ours.as_psid(), theirs.as_psid()) } != 0)
}

pub fn create_private(path: &Path) -> io::Result<File> {
    // All file access (FA) for the current user only, given to
    // CreateFileW, so the file never exists with other permissions,
    // not even for an instant.
    let (attributes, _descriptor) = owner_only("FA")?;
    // `absolute` also removes `.` and `..`, which the `\\?\` form of
    // long paths requires.
    let path_w = wide(&std::path::absolute(path)?);
    // SAFETY: `path_w` is a NUL-terminated UTF-16 string and
    // `attributes` points to a valid descriptor, both alive for the
    // call. The sharing mode is the one the standard library uses.
    let handle = unsafe {
        CreateFileW(
            path_w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a valid file handle that nothing else owns;
    // the `File` closes it.
    Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
}

/// The rights that let someone change a file or a folder: write or
/// add to it, delete it or what is in it, change its permissions or
/// owner, or anything (generic).
const CHANGE: u32 = 0x0002 // FILE_WRITE_DATA, FILE_ADD_FILE
    | 0x0004 // FILE_APPEND_DATA, FILE_ADD_SUBDIRECTORY
    | 0x0010 // FILE_WRITE_EA
    | 0x0040 // FILE_DELETE_CHILD
    | 0x0100 // FILE_WRITE_ATTRIBUTES
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000 // GENERIC_ALL
    | 0x4000_0000; // GENERIC_WRITE

/// TrustedInstaller's service SID.
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// A SID as text (`S-1-5-11`).
fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid; `text` receives a LocalAlloc'ed string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _text = LocalMemory(text.cast());
    // SAFETY: a NUL-terminated UTF-16 string, alive until `_text` drops.
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    // SAFETY: `len` UTF-16 units precede the NUL.
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(text, len)
    }))
}

/// The current user, SYSTEM, the Administrators or TrustedInstaller.
fn trusted(sid: PSID, user: &UserSid) -> io::Result<bool> {
    // SAFETY: `sid` is a valid SID inside a live descriptor.
    let well_known = unsafe {
        EqualSid(sid, user.as_psid()) != 0
            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
    };
    if well_known {
        return Ok(true);
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid; `text` receives a LocalAlloc'ed string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _text = LocalMemory(text.cast());
    // SAFETY: a NUL-terminated UTF-16 string, alive until `_text` drops.
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    // SAFETY: `len` UTF-16 units precede the NUL.
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    Ok(text == TRUSTED_INSTALLER)
}

pub fn others_only_add_in(dir: &Path) -> io::Result<bool> {
    // A volume's root (`C:\`) has no parent.
    Ok(dir.is_absolute() && dir.parent().is_none())
}

pub fn only_user_or_root_can_change(path: &Path) -> io::Result<bool> {
    // Not through a link: the link itself is what is checked.
    std::fs::symlink_metadata(path)?;
    let user = UserSid::current()?;
    let path_w = wide(&std::path::absolute(path)?);
    let (mut owner, mut dacl): (PSID, *mut ACL) = (std::ptr::null_mut(), std::ptr::null_mut());
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `path_w` is a NUL-terminated UTF-16 string alive for the
    // call; the outputs are writable pointers, the unused ones null.
    let status = unsafe {
        GetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _descriptor = LocalMemory(descriptor);
    // Its owner may always change its permissions.
    if owner.is_null() || !trusted(owner, &user)? {
        return Ok(false);
    }
    if dacl.is_null() {
        return Ok(false); // no list at all: everyone may change it
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    // SAFETY: `dacl` points into `descriptor`, alive here; `size` is a
    // writable structure of the size given.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    for index in 0..size.AceCount {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the entry count of `dacl`.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: every entry starts with an ACE_HEADER.
        let header = unsafe { *(ace as *const ACE_HEADER) };
        // An entry only for what is created inside it does not apply
        // to it.
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        match u32::from(header.AceType) {
            ACCESS_ALLOWED_ACE_TYPE => {
                let ace = ace as *const ACCESS_ALLOWED_ACE;
                // SAFETY: an allowing entry has this layout; its SID
                // starts at SidStart.
                let (mask, sid) =
                    unsafe { ((*ace).Mask, std::ptr::addr_of!((*ace).SidStart) as PSID) };
                if mask & CHANGE != 0 && !trusted(sid, &user)? {
                    // Who, for the log: a report says why (#334).
                    eprintln!(
                        "zajil: {} can be changed by {} ({mask:#x})",
                        path.display(),
                        sid_text(sid).unwrap_or_else(|_| "?".into())
                    );
                    return Ok(false);
                }
            }
            // Denying entries only take rights away.
            ACCESS_DENIED_ACE_TYPE => {}
            // Any other kind of allowing entry (object, callback…) is
            // not read: refused.
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub fn only_owner_can_access(path: &Path) -> io::Result<bool> {
    let user = UserSid::current()?;
    let path_w = wide(&std::path::absolute(path)?);
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `path_w` is a NUL-terminated UTF-16 string alive for the
    // call; the outputs are writable pointers, the unused ones null.
    let status = unsafe {
        GetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _descriptor = LocalMemory(descriptor);
    if dacl.is_null() {
        return Ok(false); // no list at all: everyone may open it
    }
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: `descriptor` is the valid descriptor returned above; the
    // outputs are writable.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Ok(false); // entries from the folder may be added later
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    // SAFETY: `dacl` points into `descriptor`, alive here; `size` is a
    // writable structure of the size given.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if size.AceCount == 0 {
        return Ok(false);
    }
    for index in 0..size.AceCount {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the entry count of `dacl`.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let ace = ace as *const ACCESS_ALLOWED_ACE;
        // SAFETY: every entry starts with an ACE_HEADER; the SID is read
        // only for allowing entries, which have this layout.
        let allows_only_user = unsafe {
            u32::from((*ace).Header.AceType) == ACCESS_ALLOWED_ACE_TYPE
                && EqualSid(std::ptr::addr_of!((*ace).SidStart) as PSID, user.as_psid()) != 0
        };
        if !allows_only_user {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Memory that a Windows call allocated with `LocalAlloc`.
pub(crate) struct LocalMemory(*mut core::ffi::c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a call documented to need
        // `LocalFree`, and is freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// The security identifier (SID) of the user running this process.
struct UserSid {
    /// A `TOKEN_USER` and the SID it points to. `u64` keeps it aligned.
    buffer: Vec<u64>,
}

impl UserSid {
    fn current() -> io::Result<Self> {
        // SAFETY: the pseudo-handle of this process needs no closing.
        Self::of_process(unsafe { GetCurrentProcess() })
    }

    /// The user a process runs as; `process` must allow
    /// PROCESS_QUERY_LIMITED_INFORMATION.
    fn of_process(process: HANDLE) -> io::Result<Self> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `process` is a process handle with query access;
        // `token` is a writable handle.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut needed = 0u32;
        // SAFETY: asks only for the size: no buffer, length 0.
        unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        // SAFETY: `buffer` has at least `needed` writable bytes.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(UserSid { buffer })
    }

    fn as_psid(&self) -> PSID {
        // SAFETY: the buffer holds the TOKEN_USER written by
        // GetTokenInformation; its SID lives in the same buffer.
        unsafe { (*(self.buffer.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    /// The SID as text, such as `S-1-5-21-…`.
    fn to_text(&self) -> io::Result<String> {
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: `as_psid` is a valid SID; `text` a writable pointer.
        if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _text = LocalMemory(text.cast());
        // SAFETY: the call returned a NUL-terminated UTF-16 string.
        let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
        // SAFETY: `len` characters before the NUL are readable.
        let units = unsafe { std::slice::from_raw_parts(text, len) };
        Ok(String::from_utf16_lossy(units))
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this module and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn open_like_a_scanner(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

pub fn write_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                buf = &buf[n..];
                offset = offset
                    .checked_add(n as u64)
                    .ok_or(io::ErrorKind::FileTooLarge)?;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

pub fn available_space(dir: &Path) -> io::Result<u64> {
    let dir_w = wide(dir);
    let mut available = 0u64;
    // SAFETY: `dir_w` is a NUL-terminated UTF-16 string and `available`
    // a writable u64, both alive for the call; the other outputs are
    // optional and passed as null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            dir_w.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

pub fn allocate(file: &File, len: u64) -> io::Result<()> {
    let mut returned = 0u32;
    // SAFETY: the handle is open for the lifetime of `file`; this
    // control code takes no input or output buffer, and `returned` is a
    // writable u32 alive for the call. No OVERLAPPED: the call is
    // synchronous.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as HANDLE,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        // FAT and exFAT have no sparse files; the file is simply longer.
        let unsupported = matches!(
            err.raw_os_error().map(|code| code as u32),
            Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED)
        );
        if !unsupported {
            return Err(err);
        }
    }
    file.set_len(len)
}

pub fn size_on_disk(path: &Path) -> io::Result<u64> {
    let path_w = wide(path);
    let mut high = 0u32;
    // SAFETY: `path_w` is a NUL-terminated UTF-16 string and `high` a
    // writable u32, both alive for the call. For a sparse file the
    // result is the size of its allocated ranges.
    let low = unsafe { GetCompressedFileSizeW(path_w.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(0) {
            return Err(err);
        }
    }
    Ok((u64::from(high) << 32) | u64::from(low))
}

fn link_count(file: &File) -> io::Result<u32> {
    // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is a valid value.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is open for the lifetime of `file`, and `info`
    // is a valid, writable structure of the expected type.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.nNumberOfLinks)
}

fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    let (from_w, to_w) = (wide(from), wide(to));
    // SAFETY: both buffers are NUL-terminated UTF-16 strings that live
    // until the call returns.
    let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), flags) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// NUL-terminated UTF-16, with the `\\?\` prefix for long absolute paths
/// (the plain form is limited to 260 characters).
fn wide(path: &Path) -> Vec<u16> {
    let text = path.as_os_str();
    let long =
        text.len() >= 248 && path.is_absolute() && !text.to_string_lossy().starts_with(r"\\");
    let prefix: &[u16] = if long {
        &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]
    } else {
        &[]
    };
    prefix
        .iter()
        .copied()
        .chain(text.encode_wide())
        .chain(Some(0))
        .collect()
}
