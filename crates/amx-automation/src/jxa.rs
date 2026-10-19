//! `osascript -l JavaScript` process wrapper: the operation's script goes in on stdin, the JSON
//! request goes in as a single `argv` string (so JXA's own JSON.parse does the decoding — no
//! shell is ever involved, so no argument escaping is needed for non-ASCII mailbox names), and
//! one JSON object comes back on stdout.

use std::process::Stdio;
use std::time::Duration;

use amx_core::AmxError;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::op::{JxaRequest, JxaResult};

const JXA_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn run(request: &JxaRequest) -> Result<JxaResult, AmxError> {
    run_with_binary_and_timeout("osascript", request, JXA_TIMEOUT).await
}

#[cfg(test)]
async fn run_with_binary(bin: &str, request: &JxaRequest) -> Result<JxaResult, AmxError> {
    run_with_binary_and_timeout(bin, request, JXA_TIMEOUT).await
}

async fn run_with_binary_and_timeout(
    bin: &str,
    request: &JxaRequest,
    limit: Duration,
) -> Result<JxaResult, AmxError> {
    let op = request.op_name();
    let payload = serde_json::to_string(request).map_err(|e| AmxError::JxaFailed {
        op: op.to_string(),
        stderr: format!("failed to serialize request: {e}"),
    })?;
    let script = request.script();

    let attempt = async {
        // `kill_on_drop` matters on the timeout path below: if the script hangs past `limit`,
        // dropping this future must not leave an orphaned osascript process still mutating Mail.
        let mut child = Command::new(bin)
            .args(["-l", "JavaScript", "-", &payload])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to spawn {bin}: {e}"))?;

        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin
            .write_all(script.as_bytes())
            .await
            .map_err(|e| format!("failed to write script to {bin} stdin: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| format!("failed to close {bin} stdin: {e}"))?;
        drop(stdin);

        child
            .wait_with_output()
            .await
            .map_err(|e| format!("failed to read {bin} output: {e}"))
    };

    let output = timeout(limit, attempt)
        .await
        .map_err(|_| AmxError::JxaTimeout { op: op.to_string() })?
        .map_err(|stderr| AmxError::JxaFailed {
            op: op.to_string(),
            stderr,
        })?;

    if !output.status.success() {
        return Err(AmxError::JxaFailed {
            op: op.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: JxaResult =
        serde_json::from_str(stdout.trim()).map_err(|e| AmxError::JxaFailed {
            op: op.to_string(),
            stderr: format!("malformed JXA response {stdout:?}: {e}"),
        })?;

    if !result.ok {
        return Err(AmxError::JxaFailed {
            op: op.to_string(),
            stderr: result
                .error
                .clone()
                .unwrap_or_else(|| "unknown JXA failure".to_string()),
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::MailboxAddress;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn write_stub(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("osascript");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn locate_request() -> JxaRequest {
        JxaRequest::Locate {
            mailbox: MailboxAddress {
                account_name: "On My Mac".to_string(),
                mailbox_segments: vec!["AMX-TEST".to_string()],
            },
            message_id: "<abc@example.com>".to_string(),
        }
    }

    #[tokio::test]
    async fn a_successful_script_response_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let stub = write_stub(
            dir.path(),
            "cat > /dev/null\necho '{\"ok\": true, \"found\": true}'",
        );

        let result = run_with_binary(stub.to_str().unwrap(), &locate_request())
            .await
            .unwrap();
        assert!(result.found);
    }

    #[tokio::test]
    async fn a_script_reported_failure_surfaces_as_jxa_failed() {
        let dir = tempfile::tempdir().unwrap();
        let stub = write_stub(
            dir.path(),
            "cat > /dev/null\necho '{\"ok\": false, \"error\": \"no such mailbox\"}'",
        );

        let err = run_with_binary(stub.to_str().unwrap(), &locate_request())
            .await
            .unwrap_err();
        match err {
            AmxError::JxaFailed { op, stderr } => {
                assert_eq!(op, "locate");
                assert!(stderr.contains("no such mailbox"));
            }
            other => panic!("expected JxaFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_nonzero_exit_surfaces_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let stub = write_stub(dir.path(), "cat > /dev/null\necho 'boom' >&2\nexit 1");

        let err = run_with_binary(stub.to_str().unwrap(), &locate_request())
            .await
            .unwrap_err();
        match err {
            AmxError::JxaFailed { stderr, .. } => assert!(stderr.contains("boom")),
            other => panic!("expected JxaFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_ascii_mailbox_segment_survives_argv_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // $4 is the JSON payload: `osascript -l JavaScript - <payload>`.
        let stub = write_stub(
            dir.path(),
            &format!(
                "cat > /dev/null\nprintf '%s' \"$4\" > {}/captured.json\necho '{{\"ok\": true}}'",
                dir.path().to_str().unwrap()
            ),
        );

        let request = JxaRequest::SetReadState {
            mailbox: MailboxAddress {
                account_name: "iCloud".to_string(),
                mailbox_segments: vec!["[Gmail]".to_string(), "Tất cả thư".to_string()],
            },
            message_id: "<xyz@example.com>".to_string(),
            read: true,
        };

        run_with_binary(stub.to_str().unwrap(), &request)
            .await
            .unwrap();

        let captured = fs::read_to_string(dir.path().join("captured.json")).unwrap();
        let round_tripped: JxaRequest = serde_json::from_str(&captured).unwrap();
        match round_tripped {
            JxaRequest::SetReadState { mailbox, .. } => {
                assert_eq!(mailbox.mailbox_segments[1], "Tất cả thư");
            }
            other => panic!("expected SetReadState, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_hanging_script_times_out_and_the_child_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let stub = write_stub(dir.path(), "cat > /dev/null\nsleep 30");

        let err = run_with_binary_and_timeout(
            stub.to_str().unwrap(),
            &locate_request(),
            Duration::from_millis(200),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AmxError::JxaTimeout { op } if op == "locate"));
    }
}
