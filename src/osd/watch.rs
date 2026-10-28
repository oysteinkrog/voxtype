//! Wakes an OSD popup as soon as the daemon changes one of its runtime files,
//! so new words show without waiting for the next poll.
//!
//! It watches the runtime folder, not the files: the daemon replaces the
//! preview and status files by rename, and removes them when a dictation
//! ends. When inotify is not available, or the watch fails, the popups keep
//! polling as before.

use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use inotify::{EventMask, Inotify, WatchMask};

/// The runtime files the popups show.
const NAMES: [&str; 6] = [
    "state",
    "preview",
    "preview-status",
    "preview.json",
    "osd_suppressed",
    super::look_step::FILE,
];

/// An inotify watch on the runtime folder.
pub struct RuntimeWatch {
    dir: PathBuf,
    inotify: Option<Inotify>,
    watching: bool,
    buffer: [u8; 4096],
}

impl RuntimeWatch {
    pub fn new(dir: PathBuf) -> RuntimeWatch {
        let inotify = Inotify::init()
            .map_err(|e| tracing::warn!("inotify unavailable, polling the runtime files: {e}"))
            .ok();
        let mut watch = RuntimeWatch {
            dir,
            inotify,
            watching: false,
            buffer: [0; 4096],
        };
        watch.ensure();
        watch
    }

    /// Adds the watch if it is missing. The folder appears when the daemon
    /// starts and goes when it stops, so this is retried on each call.
    fn ensure(&mut self) {
        let Some(inotify) = self.inotify.as_mut() else {
            return;
        };
        if self.watching {
            return;
        }
        let mask = WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::CREATE
            | WatchMask::DELETE
            | WatchMask::DELETE_SELF
            | WatchMask::MOVE_SELF;
        self.watching = inotify.watches().add(&self.dir, mask).is_ok();
    }

    /// Whether one of the runtime files changed since the last call. Never
    /// blocks. Always false without a watch.
    pub fn changed(&mut self) -> bool {
        self.ensure();
        let Some(inotify) = self.inotify.as_mut() else {
            return false;
        };
        let mut hit = false;
        loop {
            match inotify.read_events(&mut self.buffer) {
                Ok(events) => {
                    for event in events {
                        if event.mask.intersects(
                            EventMask::IGNORED | EventMask::DELETE_SELF | EventMask::MOVE_SELF,
                        ) {
                            // The folder went away; add the watch again later.
                            self.watching = false;
                            hit = true;
                        } else if event
                            .name
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| NAMES.contains(&n))
                        {
                            hit = true;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    tracing::warn!("inotify read failed, polling the runtime files: {e}");
                    self.inotify = None;
                    self.watching = false;
                    break;
                }
            }
        }
        hit
    }

    /// Waits up to `timeout` for a change to one of the runtime files, and
    /// says whether one came. Without a watch it sleeps for `timeout`.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        self.ensure();
        let Some(inotify) = self.inotify.as_ref().filter(|_| self.watching) else {
            std::thread::sleep(timeout);
            return false;
        };
        let mut fd = libc::pollfd {
            fd: inotify.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut fd, 1, ms) };
        if ready < 0 {
            std::thread::sleep(timeout);
            return false;
        }
        self.changed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A runtime folder of its own, inside a temp folder that is removed at the end.
    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("voxtype");
        std::fs::create_dir(&dir).unwrap();
        (root, dir)
    }

    #[test]
    fn a_renamed_preview_wakes_the_wait_at_once() {
        let (_root, dir) = scratch();
        let mut watch = RuntimeWatch::new(dir.clone());
        assert!(!watch.changed());
        let writer = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                std::fs::write(dir.join("preview.tmp"), "hello").unwrap();
                std::fs::rename(dir.join("preview.tmp"), dir.join("preview")).unwrap();
            })
        };
        let start = Instant::now();
        let mut woke = false;
        while !woke && start.elapsed() < Duration::from_secs(5) {
            woke = watch.wait(Duration::from_secs(5));
        }
        writer.join().unwrap();
        assert!(woke);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn other_files_do_not_count() {
        let (_root, dir) = scratch();
        let mut watch = RuntimeWatch::new(dir.clone());
        std::fs::write(dir.join("hotkey-last"), "1").unwrap();
        std::fs::write(dir.join("preview.tmp"), "x").unwrap();
        assert!(!watch.changed());
        std::fs::write(dir.join("state"), "recording").unwrap();
        assert!(watch.changed());
    }

    #[test]
    fn the_watch_comes_back_when_the_folder_is_made_again() {
        let (_root, dir) = scratch();
        let mut watch = RuntimeWatch::new(dir.clone());
        std::fs::remove_dir_all(&dir).unwrap();
        watch.changed();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!watch.changed());
        std::fs::write(dir.join("preview"), "again").unwrap();
        assert!(watch.changed());
    }

    #[test]
    fn a_missing_folder_falls_back_to_sleeping() {
        let (_root, dir) = scratch();
        let mut watch = RuntimeWatch::new(dir.join("never-made"));
        let start = Instant::now();
        assert!(!watch.wait(Duration::from_millis(30)));
        assert!(start.elapsed() >= Duration::from_millis(30));
    }
}
