//! MimiEyes (VENDORED.md #7): the activity monitor's shared shapes.
//!
//! A small watcher process (`mimieyes monitor --pid N`, started with the
//! window) waits for the app to exit and books the run to
//! `logs/hz-activity.jsonl`: when it started and ended, the exit code, and
//! whether the PC slept meanwhile. On its clean exit paths the app first
//! leaves `logs/hz-quit-<pid>.json` saying why it is quitting (window
//! closed, quit); a kill or a crash leaves none, and the exit code tells.
//! Settings → Activity lists the runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const ACTIVITY_FILE: &str = "hz-activity.jsonl";

/// Why the app said it is quitting (written just before it does).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuitMarker {
    pub pid: u32,
    /// Epoch milliseconds.
    pub at: i64,
    pub reason: String,
}

/// One line of the activity log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecord {
    /// "start" when the watcher begins, "exit" once the app is gone.
    pub event: String,
    pub pid: u32,
    /// The run's start (epoch ms).
    pub started: i64,
    /// When it ended (epoch ms; exit records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
    /// A person-readable account of how it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Seconds the PC spent asleep during the run.
    #[serde(default)]
    pub slept_secs: u64,
    /// Seconds between the last wake from sleep and the exit, when the PC
    /// slept during the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub woke_secs_before_exit: Option<u64>,
    /// The app version that ran.
    #[serde(default)]
    pub version: String,
}

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

fn marker_path(data_dir: &Path, pid: u32) -> PathBuf {
    logs_dir(data_dir).join(format!("hz-quit-{pid}.json"))
}

/// The app is about to quit for `reason` (the first reason wins: closing the
/// window is what quits the app).
pub fn note_quit(data_dir: &Path, reason: &str) {
    let pid = std::process::id();
    let path = marker_path(data_dir, pid);
    if path.exists() {
        return;
    }
    let marker = QuitMarker {
        pid,
        at: chrono::Utc::now().timestamp_millis(),
        reason: reason.to_owned(),
    };
    if let Ok(text) = serde_json::to_string(&marker) {
        let _ = std::fs::create_dir_all(logs_dir(data_dir));
        let _ = std::fs::write(path, text);
    }
}

/// The watcher's read (and removal) of the app's marker.
pub fn take_marker(data_dir: &Path, pid: u32) -> Option<QuitMarker> {
    let path = marker_path(data_dir, pid);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&text).ok()
}

/// How a run ended, from the app's own marker or else its exit code.
pub fn explain(exit_code: u32, marker: Option<&QuitMarker>) -> String {
    if let Some(marker) = marker {
        return marker.reason.clone();
    }
    match exit_code {
        0 => "Exited normally".into(),
        1 => "Ended from outside (Task Manager, taskkill or another program)".into(),
        0x4001_0004 => "Ended by Windows (sign-out, restart or shutdown)".into(),
        0xC000_013A => "Stopped with Ctrl+C".into(),
        0xC000_0409 => "Crashed (abort — a fatal error)".into(),
        0xC000_0005 => "Crashed (access violation)".into(),
        0xC000_00FD => "Crashed (stack overflow)".into(),
        code => format!("Ended with code 0x{code:08X}"),
    }
}

/// Append one record.
pub fn append(data_dir: &Path, record: &ActivityRecord) {
    use std::io::Write as _;
    let Ok(mut line) = serde_json::to_string(record) else {
        return;
    };
    line.push('\n');
    let _ = std::fs::create_dir_all(logs_dir(data_dir));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir(data_dir).join(ACTIVITY_FILE))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// One run as Settings → Activity shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub pid: u32,
    pub started: i64,
    pub ended: Option<i64>,
    pub exit_code: Option<u32>,
    pub reason: String,
    pub slept_secs: u64,
    pub woke_secs_before_exit: Option<u64>,
    pub version: String,
}

/// Pair start and exit records into runs, newest first. A start with no exit
/// is the run going now (`current_pid`) — or one whose watcher was stopped
/// along with the app (both ended at once: sign-out, shutdown, a tree kill).
pub fn runs(text: &str, current_pid: u32) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for record in text
        .lines()
        .filter_map(|l| serde_json::from_str::<ActivityRecord>(l).ok())
    {
        let existing = runs
            .iter_mut()
            .find(|r| r.pid == record.pid && r.started == record.started);
        match (record.event.as_str(), existing) {
            ("exit", Some(run)) => {
                run.ended = record.ended;
                run.exit_code = record.exit_code;
                run.reason = record.reason.unwrap_or_default();
                run.slept_secs = record.slept_secs;
                run.woke_secs_before_exit = record.woke_secs_before_exit;
            }
            ("start", Some(_)) => {}
            (_, None) => runs.push(Run {
                pid: record.pid,
                started: record.started,
                ended: record.ended,
                exit_code: record.exit_code,
                reason: record.reason.unwrap_or_default(),
                slept_secs: record.slept_secs,
                woke_secs_before_exit: record.woke_secs_before_exit,
                version: record.version,
            }),
            _ => {}
        }
    }
    for run in &mut runs {
        if run.ended.is_none() {
            run.reason = if run.pid == current_pid {
                "Running now".into()
            } else {
                "Ended together with its monitor (sign-out, shutdown, or the process tree was killed)"
                    .into()
            };
        }
    }
    runs.sort_by(|a, b| b.started.cmp(&a.started));
    runs
}

pub fn read_runs(data_dir: &Path) -> Vec<Run> {
    let text = std::fs::read_to_string(logs_dir(data_dir).join(ACTIVITY_FILE)).unwrap_or_default();
    runs(&text, std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(pid: u32, started: i64) -> String {
        serde_json::to_string(&ActivityRecord {
            event: "start".into(),
            pid,
            started,
            ended: None,
            exit_code: None,
            reason: None,
            slept_secs: 0,
            woke_secs_before_exit: None,
            version: "0.2.113".into(),
        })
        .unwrap()
    }

    #[test]
    fn exits_are_explained_by_marker_then_code() {
        let marker = QuitMarker {
            pid: 1,
            at: 0,
            reason: "Window closed".into(),
        };
        assert_eq!(explain(0, Some(&marker)), "Window closed");
        assert!(explain(1, None).contains("from outside"));
        assert!(explain(0xC000_0409, None).contains("Crashed"));
        assert_eq!(explain(0x1234, None), "Ended with code 0x00001234");
    }

    #[test]
    fn starts_and_exits_pair_into_runs_newest_first() {
        let exit = serde_json::to_string(&ActivityRecord {
            event: "exit".into(),
            pid: 10,
            started: 1_000,
            ended: Some(5_000),
            exit_code: Some(0),
            reason: Some("Window closed".into()),
            slept_secs: 60,
            woke_secs_before_exit: Some(3),
            version: "0.2.113".into(),
        })
        .unwrap();
        let text = format!("{}\n{}\n{}\n{}\n", start(10, 1_000), exit, start(20, 9_000), start(30, 7_000));
        let runs = runs(&text, 20);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].pid, 20);
        assert_eq!(runs[0].reason, "Running now");
        assert!(runs[1].reason.contains("together with its monitor"));
        assert_eq!(runs[2].reason, "Window closed");
        assert_eq!(runs[2].slept_secs, 60);
    }
}
