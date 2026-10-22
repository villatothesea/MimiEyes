//! MimiEyes (VENDORED.md #23): one window at a time. A headed launch while
//! MimiEyes is already open brings that window to the front and exits,
//! instead of opening a second viewport on the same engine.

/// The open MimiEyes window of another process, brought to the front.
/// `true` when one was found (the caller exits).
pub fn activate_existing() -> bool {
    #[cfg(windows)]
    {
        windows::activate_existing()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};

    const TRUE: i32 = 1;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    struct Search {
        own_pid: u32,
        exe_name: String,
        found: HWND,
    }

    /// The executable file name of `pid` (lowercase), if it can be read.
    fn exe_name_of(pid: u32) -> Option<String> {
        // SAFETY: a handle we own, closed below; buffer sized by `len`.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(str::to_lowercase)
        }
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the `Search` passed to EnumWindows below.
        let search = unsafe { &mut *(lparam as *mut Search) };
        // SAFETY: plain window queries on a handle EnumWindows gave us.
        unsafe {
            if IsWindowVisible(hwnd) == 0 && IsIconic(hwnd) == 0 {
                return TRUE;
            }
            let mut title = [0u16; 64];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            if n <= 0 || String::from_utf16_lossy(&title[..n as usize]) != "MimiEyes" {
                return TRUE;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == search.own_pid {
                return TRUE;
            }
            // `mimieyes.exe`, or `mimieyes.old-<ts>.exe` after an install
            // renamed the running copy aside.
            let stem = search.exe_name.trim_end_matches(".exe");
            if exe_name_of(pid).is_some_and(|name| name.starts_with(stem)) {
                search.found = hwnd;
                return 0; // stop
            }
        }
        TRUE
    }

    pub(super) fn activate_existing() -> bool {
        let exe_name = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .unwrap_or_else(|| "mimieyes.exe".into());
        let mut search = Search {
            own_pid: std::process::id(),
            exe_name,
            found: std::ptr::null_mut(),
        };
        // SAFETY: the callback only reads windows and writes `search`.
        unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
        if search.found.is_null() {
            return false;
        }
        // SAFETY: a live top-level window found above.
        unsafe {
            if IsIconic(search.found) != 0 {
                ShowWindow(search.found, SW_RESTORE);
            }
            SetForegroundWindow(search.found);
        }
        true
    }
}
