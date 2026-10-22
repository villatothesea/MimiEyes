//! MimiEyes (VENDORED.md #24): start a child process without stalling the
//! runtime. Tokio's `spawn` calls CreateProcessW right on the polling worker;
//! with antivirus scanning every process start that takes seconds on Windows,
//! and the worker driving sockets and timers froze with it (IPC dials, MCP
//! calls and queued sends timed out). `block_in_place` hands the worker's
//! duties to another thread for the duration.

pub fn spawn(command: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    let multi_thread = tokio::runtime::Handle::try_current()
        .is_ok_and(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if multi_thread {
        tokio::task::block_in_place(|| command.spawn())
    } else {
        command.spawn()
    }
}

/// [`spawn`], then collect stdout and stderr like `Command::output`.
pub async fn output(command: &mut tokio::process::Command) -> std::io::Result<std::process::Output> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    spawn(command)?.wait_with_output().await
}

#[cfg(test)]
mod tests {
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn output_collects_stdout_on_a_multi_thread_runtime() {
        let mut command = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/c", "echo hz"]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", "echo hz"]);
            c
        };
        let output = super::output(&mut command).await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hz");
    }

    #[tokio::test]
    async fn output_works_on_a_current_thread_runtime_too() {
        let mut command = tokio::process::Command::new("git");
        command.arg("--version");
        let output = super::output(&mut command).await.unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("git version"));
    }
}
