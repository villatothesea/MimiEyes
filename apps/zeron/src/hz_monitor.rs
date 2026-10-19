//! MimiEyes (VENDORED.md #7): the activity monitor.
//!
//! The headed app starts `mimieyes monitor --pid <its pid>` detached. The
//! watcher holds a handle to the app, so whatever ends it — a closed window,
//! a crash, Task Manager, Windows — it sees the exit: when, the exit code,
//! the app's own reason if it left one (`zeron_ui::hz_activity`), and how long
//! the PC slept during the run (the sleep-inclusive tick count against the
//! sleep-excluding interrupt time). Runs are booked to
//! `logs/hz-activity.jsonl`; Settings → Activity lists them.

use std::path::Path;

use zeron_ui::hz_activity::{self, ActivityRecord};

/// Start the watcher for this process (headed app start). Best effort: a
/// failure to spawn only loses the record, never the app.
pub fn spawn_for_self() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let pid = std::process::id().to_string();
        let spawn = |flags: u32| {
            std::process::Command::new(&exe)
                .args(["monitor", "--pid", &pid])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        // Out of the app's job when allowed, so a job-wide kill of the app
        // still leaves the watcher to tell it.
        let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        if spawn(base | CREATE_BREAKAWAY_FROM_JOB).is_err() {
            let _ = spawn(base);
        }
    }
}

/// `mimieyes monitor --pid N`: wait for N to exit and book the run.
pub fn run(pid: u32, data_dir: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::watch(pid, data_dir)
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, data_dir);
        Ok(())
    }
}

/// Wall-clock epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };
    use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

    /// Milliseconds the PC has slept since boot: the tick count runs through
    /// sleep, the unbiased interrupt time (100 ns units) does not.
    fn slept_ms() -> u64 {
        let mut unbiased = 0u64;
        // SAFETY: plain out-parameter call.
        unsafe { QueryUnbiasedInterruptTime(&mut unbiased) };
        // SAFETY: no arguments.
        let ticks = unsafe { GetTickCount64() };
        ticks.saturating_sub(unbiased / 10_000)
    }

    pub(super) fn watch(pid: u32, data_dir: &Path) -> anyhow::Result<()> {
        // SAFETY: OpenProcess returns a handle we own (or null).
        let handle = unsafe {
            OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)
        };
        if handle.is_null() {
            anyhow::bail!("process {pid} is not running");
        }
        let started = now_ms();
        let version = env!("CARGO_PKG_VERSION").to_owned();
        hz_activity::append(
            data_dir,
            &ActivityRecord {
                event: "start".into(),
                pid,
                started,
                ended: None,
                exit_code: None,
                reason: None,
                slept_secs: 0,
                woke_secs_before_exit: None,
                version: version.clone(),
            },
        );
        let slept_at_start = slept_ms();
        let mut slept_so_far = 0u64;
        let mut last_wake: Option<std::time::Instant> = None;
        loop {
            // A minute at a time, so a sleep is noticed near when it ends.
            // SAFETY: valid handle until CloseHandle below.
            let waited = unsafe { WaitForSingleObject(handle, 60_000) };
            let slept = slept_ms().saturating_sub(slept_at_start);
            if slept > slept_so_far + 5_000 {
                last_wake = Some(std::time::Instant::now());
            }
            slept_so_far = slept;
            if waited == WAIT_OBJECT_0 {
                break;
            }
        }
        let mut code = 0u32;
        // SAFETY: valid handle; out-parameter.
        unsafe {
            GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
        }
        let marker = hz_activity::take_marker(data_dir, pid);
        let mut reason = hz_activity::explain(code, marker.as_ref());
        let woke = last_wake.map(|t| t.elapsed().as_secs());
        if let Some(secs) = woke
            && secs < 120
        {
            reason.push_str(" — just after the PC woke from sleep");
        }
        hz_activity::append(
            data_dir,
            &ActivityRecord {
                event: "exit".into(),
                pid,
                started,
                ended: Some(now_ms()),
                exit_code: Some(code),
                reason: Some(reason),
                slept_secs: slept_so_far / 1_000,
                woke_secs_before_exit: woke,
                version,
            },
        );
        Ok(())
    }
}
