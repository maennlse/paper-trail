// SPDX-License-Identifier: GPL-3.0-or-later
//! Checked browser dispatch. Never include OAuth URLs or launcher output in errors.
use super::{check_login_cancelled, device::Cancellation};
use std::{
    io,
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

pub(super) fn wait_for_launch(
    result: Receiver<io::Result<()>>,
    cancel: &Cancellation,
) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        check_login_cancelled(cancel)?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "System browser launch timed out; check the default browser and desktop portal",
            ));
        }
        match result.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("System browser launcher disconnected"));
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn open(url: &str, cancel: &Cancellation) -> io::Result<()> {
    // The generic browser crate directly spawns the desktop entry's executable
    // and reports success even if that executable immediately fails. Let the
    // desktop's URL dispatcher handle browser activation and check its exit.
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    launch(command, cancel)
}

#[cfg(target_os = "linux")]
fn launch(mut command: std::process::Command, cancel: &Cancellation) -> io::Result<()> {
    use std::process::Stdio;
    check_login_cancelled(cancel)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            io::Error::other(
                "Could not start the system URL launcher; check that xdg-open is installed",
            )
        })?;
    let (tx, rx) = std::sync::mpsc::channel();
    // Always reap the child, including after cancellation/timeout. Never kill
    // it: a launcher can exec a browser and we must not kill the user's browser.
    std::thread::spawn(move || {
        let result = child.wait().map_err(|_| io::Error::other("Could not wait for the system browser launcher"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("System browser launch failed; check the default HTTPS browser and desktop portal"))
                }
            });
        let _ = tx.send(result);
    });
    wait_for_launch(rx, cancel)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn open(url: &str, cancel: &Cancellation) -> io::Result<()> {
    check_login_cancelled(cancel)?;
    webbrowser::open(url).map_err(|_| io::Error::other("Could not open the system browser"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_launch_wait_does_not_wait_for_a_portal_reply() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            wait_for_launch(rx, &cancel).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launcher_exit_is_checked_and_backend_output_is_not_exposed() {
        for (exit, success) in [("0", true), ("1", false)] {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", &format!("echo PRIVATE_URL >&2; exit {exit}")]);
            let result = launch(command, &Cancellation::default());
            assert_eq!(result.is_ok(), success);
            if let Err(error) = result {
                assert!(!error.to_string().contains("PRIVATE_URL"));
            }
        }
    }
}
