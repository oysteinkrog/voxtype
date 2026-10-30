//! Gets the popup ready again when the display comes or comes back.
//!
//! The popup connects and makes its GPU device and renderer at startup, so
//! the first dictation draws on the GPU at once. `voxtype.service` can
//! start before the compositor, and the compositor can restart under a
//! running daemon. In both cases that warm-up fails or is lost, and the
//! next dictation would make it all while it shows.
//!
//! [`watch`] waits in inotify for the compositor's socket to be created
//! (a start or a restart), and [`Retry`] tries a failed warm-up again
//! after 1 s, then twice as long each time up to 60 s, in case the socket
//! was there but the compositor was not ready.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use inotify::{Inotify, WatchMask};

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LONGEST_RETRY: Duration = Duration::from_secs(60);

/// The compositor's socket, as libwayland finds it: `$WAYLAND_DISPLAY`
/// (default `wayland-0`) in `$XDG_RUNTIME_DIR`, or the path itself when it
/// is absolute.
fn socket_path(
    wayland_display: Option<OsString>,
    runtime_dir: Option<OsString>,
) -> Option<PathBuf> {
    let name = PathBuf::from(wayland_display.unwrap_or_else(|| "wayland-0".into()));
    if name.is_absolute() {
        return Some(name);
    }
    Some(PathBuf::from(runtime_dir?).join(name))
}

/// Calls `on_new` each time the compositor's socket is created. The thread
/// sleeps in inotify in between. Returns false when it cannot watch; the
/// retries still run then.
pub(crate) fn watch(on_new: impl Fn() + Send + 'static) -> bool {
    let socket = socket_path(
        std::env::var_os("WAYLAND_DISPLAY"),
        std::env::var_os("XDG_RUNTIME_DIR"),
    );
    socket.is_some_and(|s| watch_socket(s, on_new))
}

fn watch_socket(socket: PathBuf, on_new: impl Fn() + Send + 'static) -> bool {
    let (Some(dir), Some(name)) = (socket.parent(), socket.file_name()) else {
        return false;
    };
    let (dir, name) = (dir.to_path_buf(), name.to_os_string());
    let mut inotify = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!("cannot watch for the display: {e}");
            return false;
        }
    };
    if let Err(e) = inotify
        .watches()
        .add(&dir, WatchMask::CREATE | WatchMask::MOVED_TO)
    {
        tracing::debug!("cannot watch {}: {e}", dir.display());
        return false;
    }
    std::thread::Builder::new()
        .name("voxtype-osd-disp".into())
        .spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                let events = match inotify.read_events_blocking(&mut buffer) {
                    Ok(events) => events,
                    Err(e) => {
                        tracing::debug!("stopped watching for the display: {e}");
                        return;
                    }
                };
                if events.into_iter().any(|e| e.name == Some(name.as_os_str())) {
                    on_new();
                }
            }
        })
        .is_ok()
}

/// When to try a failed warm-up again.
#[derive(Debug)]
pub(crate) struct Retry {
    next: Option<Instant>,
    delay: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            next: None,
            delay: FIRST_RETRY,
        }
    }
}

impl Retry {
    /// The warm-up failed: try again after the current delay, and wait
    /// twice as long after the next failure. Returns whether this is the
    /// first failure in a row.
    pub(crate) fn failed(&mut self) -> bool {
        let first = self.delay == FIRST_RETRY;
        self.next = Some(Instant::now() + self.delay);
        self.delay = (self.delay * 2).min(LONGEST_RETRY);
        first
    }

    /// The warm-up started: no retry.
    pub(crate) fn succeeded(&mut self) {
        *self = Retry::default();
    }

    /// Whether a retry is due now.
    pub(crate) fn due(&self) -> bool {
        self.next.is_some_and(|t| Instant::now() >= t)
    }

    /// How long until the retry is due, or `None` when none is pending.
    pub(crate) fn wait(&self) -> Option<Duration> {
        self.next
            .map(|t| t.saturating_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn retry_backs_off_to_a_minute_and_resets() {
        let mut r = Retry::default();
        assert!(!r.due() && r.wait().is_none());
        assert!(r.failed());
        assert!(r.wait().unwrap() <= FIRST_RETRY);
        assert!(!r.failed());
        for _ in 0..10 {
            r.failed();
        }
        assert_eq!(r.delay, LONGEST_RETRY);
        r.succeeded();
        assert!(r.wait().is_none());
        assert_eq!(r.delay, FIRST_RETRY);
    }

    #[test]
    fn socket_path_follows_libwayland() {
        let run = Some(OsString::from("/run/user/1000"));
        assert_eq!(
            socket_path(None, run.clone()),
            Some(PathBuf::from("/run/user/1000/wayland-0"))
        );
        assert_eq!(
            socket_path(Some("wayland-1".into()), run),
            Some(PathBuf::from("/run/user/1000/wayland-1"))
        );
        assert_eq!(
            socket_path(Some("/tmp/w".into()), None),
            Some(PathBuf::from("/tmp/w"))
        );
        assert_eq!(socket_path(Some("wayland-1".into()), None), None);
    }

    #[test]
    fn watch_sees_the_socket_appear() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        assert!(watch_socket(dir.path().join("wayland-test"), move || {
            let _ = tx.send(());
        }));
        std::fs::write(dir.path().join("other"), "").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        let _socket =
            std::os::unix::net::UnixListener::bind(dir.path().join("wayland-test")).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
