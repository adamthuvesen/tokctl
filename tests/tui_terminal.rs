#![cfg(unix)]

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

struct DemoTerminal {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    original_modes: String,
    _directory: tempfile::TempDir,
}

impl DemoTerminal {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("create isolated demo directory");
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 36,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("create PTY, required for terminal verification");
        let original_modes = format!(
            "{:?}",
            pair.master
                .get_termios()
                .expect("inspect initial terminal modes")
        );
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tokctl"));
        command.args(["ui", "--demo", "--cache-dir"]);
        command.arg(directory.path().join("demo"));
        command.env_clear();
        command.env("HOME", directory.path());
        command.env("TERM", "xterm-256color");
        command.env("TZ", "UTC");
        let child = pair.slave.spawn_command(command).expect("start demo UI");
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("read terminal output");
        let writer = pair.master.take_writer().expect("write terminal input");
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = [0; 8192];
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 || sender.send(bytes[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            master: pair.master,
            writer,
            output,
            parser: vt100::Parser::new(36, 120, 0),
            original_modes,
            _directory: directory,
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).expect("send terminal input");
        self.writer.flush().expect("flush terminal input");
    }

    fn expect_screen(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self.parser.screen().contents().contains(expected) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.parser.process(&bytes),
                Err(error) => panic!(
                    "expected {expected:?} in terminal output: {error}\n{}",
                    self.parser.screen().contents()
                ),
            }
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize terminal");
    }

    fn quit(&mut self) {
        self.send(b"q");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.output.recv_timeout(Duration::from_millis(50)) {
                Ok(bytes) => self.parser.process(&bytes),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(status) = self.child.try_wait().expect("read child status") {
                assert!(status.success(), "UI exited with {status:?}");
                while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(50)) {
                    self.parser.process(&bytes);
                }
                break;
            }
            assert!(Instant::now() < deadline, "UI did not exit after q");
        }
        assert!(self.child.wait().expect("wait for UI exit").success());
        assert!(
            !self.parser.screen().alternate_screen(),
            "UI did not leave the alternate screen"
        );
        let modes = self.master.get_termios().expect("inspect terminal modes");
        assert_eq!(
            format!("{modes:?}"),
            self.original_modes,
            "UI did not restore terminal modes"
        );
    }
}

impl Drop for DemoTerminal {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[test]
fn demo_filters_drills_resizes_and_restores_terminal() {
    let mut terminal = DemoTerminal::start();
    terminal.expect_screen("MONTHS");
    terminal.send(b"at");
    terminal.expect_screen("PROVIDER");
    terminal.resize(20, 60);
    terminal.expect_screen("terminal too narrow");
    terminal.resize(36, 120);
    terminal.expect_screen("PROVIDER");
    terminal.send(b"]");
    terminal.expect_screen("section:repos");
    terminal.send(b"/agent-hub\r");
    terminal.expect_screen("agent-hub");
    terminal.send(b"\r");
    terminal.expect_screen("REPOS › agent-hub");
    terminal.quit();
}
