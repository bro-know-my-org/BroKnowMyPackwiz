use super::{Control, Error, ErrorCode, Event, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn run(mut command: Command, cwd: &Path, logs: &Path, control: &Control) -> Result<()> {
    control.check()?;
    fs::create_dir_all(logs)?;
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(logs.join("stdout.log"))?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(logs.join("stderr.log"))?;
    let mut child = Running(
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .spawn()?,
    );
    let out = child.0.stdout.take().unwrap();
    let err = child.0.stderr.take().unwrap();
    let output_control = control.clone();
    let error_control = control.clone();
    let output = thread::spawn(move || capture(out, stdout, &output_control));
    let errors = thread::spawn(move || capture(err, stderr, &error_control));
    let mut cancelled = false;
    let status = loop {
        if control.check().is_err() {
            control.emit(Event::Phase("stopping".into()));
            cancelled = true;
            let _ = child.0.kill();
            break child.0.wait()?;
        }
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let output_result = output
        .join()
        .map_err(|_| Error::new(ErrorCode::Failed, "stdout_capture_failed"))?;
    let error_result = errors
        .join()
        .map_err(|_| Error::new(ErrorCode::Failed, "stderr_capture_failed"))?;
    if cancelled {
        return Err(Error::new(ErrorCode::Cancelled, "cancelled"));
    }
    output_result?;
    error_result?;
    control.check()?;
    if !status.success() {
        return Err(Error::new(
            ErrorCode::Failed,
            format!("command_exit: {status}"),
        ));
    }
    Ok(())
}
fn capture(mut reader: impl Read, mut log: File, control: &Control) -> Result<()> {
    let mut bytes = [0; 8192];
    let mut line = Vec::new();
    loop {
        let count = reader.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        log.write_all(&bytes[..count])?;
        for byte in &bytes[..count] {
            if matches!(byte, b'\n' | b'\r') {
                emit(&mut line, control);
            } else {
                line.push(*byte);
                if line.len() >= 65536 {
                    emit(&mut line, control);
                }
            }
        }
    }
    emit(&mut line, control);
    log.sync_all()?;
    Ok(())
}
fn emit(bytes: &mut Vec<u8>, control: &Control) {
    if bytes.is_empty() {
        return;
    }
    let line = sanitize(&String::from_utf8_lossy(bytes));
    if !line.trim().is_empty() {
        control.emit(Event::Log(line));
    }
    bytes.clear();
}
fn sanitize(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' || (ch == '\u{1b}' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !ch.is_control() || ch == '\t' {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::durable;
    #[test]
    fn display_strips_terminal_controls_while_preserving_unicode_text() {
        assert_eq!(
            sanitize("\x1b[31m中文\x1b[0m\x1b]52;c;payload\x07 test"),
            "中文 test"
        );
    }
    #[test]
    fn fixture() {
        if std::env::var("BKMPW_TEST_PROCESS").as_deref() == Ok("wait") {
            println!("fixture_ready");
            std::io::stdout().flush().unwrap();
            thread::sleep(Duration::from_secs(30));
        }
    }
    #[test]
    fn cancellation_stops_child_and_keeps_diagnostic_output() {
        let root = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(&root).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let control = Control::with_events(sender);
        let cancel = control.clone();
        let waiter = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut ready = false;
            while std::time::Instant::now() < deadline {
                if let Ok(Event::Log(line)) = receiver.recv_timeout(Duration::from_millis(100)) {
                    if line.contains("fixture_ready") {
                        ready = true;
                        break;
                    }
                }
            }
            cancel.cancel();
            ready
        });
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "operation::process::tests::fixture",
                "--nocapture",
            ])
            .env("BKMPW_TEST_PROCESS", "wait");
        assert_eq!(
            run(command, &root, &root.join("logs"), &control)
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
        assert!(waiter.join().unwrap());
        assert!(
            fs::read_to_string(root.join("logs/stdout.log"))
                .unwrap()
                .contains("fixture_ready")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
