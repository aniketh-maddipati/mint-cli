use std::io::{Read, Write};

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::{AgentKind, AppEvent};
use crate::config::AgentCmd;

/// A running CLI agent driven through a pseudoterminal.
///
/// Child output is parsed into a `vt100` screen so it can be rendered with the
/// `tui-term` widget. Writes (keystrokes) go straight back into the PTY master.
pub struct PtySession {
    pub parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    rows: u16,
    cols: u16,
}

impl PtySession {
    /// Spawn `cmd` in a PTY sized to `rows` x `cols`. A background thread pumps
    /// child output into `tx` as `AppEvent::PtyOutput` messages.
    pub fn spawn(
        kind: AgentKind,
        cmd: &AgentCmd,
        rows: u16,
        cols: u16,
        tx: UnboundedSender<AppEvent>,
    ) -> Result<Self> {
        let rows = rows.max(1);
        let cols = cols.max(1);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;

        let mut builder = CommandBuilder::new(&cmd.command);
        for arg in &cmd.args {
            builder.arg(arg);
        }
        if let Ok(cwd) = std::env::current_dir() {
            builder.cwd(cwd);
        }

        let mut child = pair
            .slave
            .spawn_command(builder)
            .with_context(|| format!("spawn `{}`", cmd.command))?;

        // The slave handle is not needed once the child owns it.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let killer = child.clone_killer();

        // Blocking reader thread -> async channel.
        let out_tx = tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out_tx
                            .send(AppEvent::PtyOutput(kind, buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Wait for exit on another thread so the loop is notified.
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = tx.send(AppEvent::PtyExited(kind));
        });

        Ok(Self {
            parser: vt100::Parser::new(rows, cols, 2000),
            master: pair.master,
            writer,
            killer,
            rows,
            cols,
        })
    }

    /// Forward raw bytes (translated keystrokes) to the child process.
    pub fn write_input(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Feed child output bytes into the terminal parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Signal the child process to terminate.
    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    /// Resize both the parser and the underlying PTY to match the render area.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if rows == self.rows && cols == self.cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        self.parser.screen_mut().set_size(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.kill();
    }
}
