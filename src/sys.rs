//! Small Windows helpers: drives, Explorer, elevation.

use std::path::{Path, PathBuf};

pub struct Drive {
    pub root: PathBuf,
    pub total: u64,
    pub free: u64,
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Total and free bytes of the volume holding `path`.
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let w = wide(path.as_os_str());
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then_some((total, free))
}

pub fn drives() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = PathBuf::from(format!("{}:\\", (b'A' + i) as char));
            let (total, free) = disk_space(&root)?;
            Some(Drive { root, total, free })
        })
        .collect()
}

pub fn show_in_explorer(path: &Path) {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(path.as_os_str());
    let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
}

pub fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Relaunch this exe elevated (UAC prompt), passing `scan_path` along. Returns true if it started.
pub fn relaunch_as_admin(scan_path: Option<&Path>) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let Ok(exe) = std::env::current_exe() else { return false };
    let params = scan_path
        .map(|p| format!("\"{}\"", p.to_string_lossy().trim_end_matches('\\')))
        .unwrap_or_default();
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let params = wide(params.as_ref());
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}
