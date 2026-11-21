//! One long-running ExifTool process (`-stay_open`): one command per file,
//! each answered with `{ready}`. Fast for thousands of files, and every file
//! gets its own result. Separate process, no shell, no console window.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// How long one file may take before ExifTool is stopped.
const PER_FILE: Duration = Duration::from_secs(60);

pub struct ExifTool {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl ExifTool {
    pub fn start(exe: &Path) -> std::io::Result<Self> {
        // Errors and normal output in one stream, so each answer ends at {ready}.
        let (reader, writer) = std::io::pipe()?;
        let mut cmd = Command::new(exe);
        cmd.args(["-stay_open", "True", "-@", "-"]).stdin(Stdio::piped()).stdout(writer.try_clone()?).stderr(writer);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        drop(cmd); // closes our copies of the pipe's write end
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(reader).lines() {
                let Ok(l) = l else { break };
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Ok(ExifTool { child, stdin, lines })
    }

    /// Run one command (arguments, one per line) and return its output lines.
    pub fn run(&mut self, args: &[String]) -> std::io::Result<Vec<String>> {
        for a in args {
            if a.contains(['\n', '\r']) {
                return Err(std::io::Error::other("an argument contains a line break"));
            }
            writeln!(self.stdin, "{a}")?;
        }
        writeln!(self.stdin, "-execute")?;
        self.stdin.flush()?;
        let mut out = vec![];
        loop {
            match self.lines.recv_timeout(PER_FILE) {
                Ok(l) if l.trim() == "{ready}" => return Ok(out),
                Ok(l) => out.push(l),
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    return Err(std::io::Error::other("ExifTool took too long"));
                }
                Err(RecvTimeoutError::Disconnected) => return Err(std::io::Error::other("ExifTool stopped")),
            }
        }
    }
}

impl Drop for ExifTool {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "-stay_open\nFalse");
        let _ = self.stdin.flush();
        for _ in 0..50 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
    }
}
