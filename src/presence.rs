//! Whether the user is at the computer, from their keyboard, mouse and
//! touchpad input.
//!
//! A thread reads every keyboard and pointer device under `/dev/input`
//! (evdev, no grab, so the compositor and the hotkey listener still get
//! every event). This works the same on every compositor and on X11, sees
//! the keys typed at the lock screen, and needs only the `input` group the
//! hotkey listener already needs. The alternatives were checked on KDE
//! Plasma 6 Wayland: `org.freedesktop.ScreenSaver.GetSessionIdleTime` answers
//! "not supported on this platform", and logind's `IdleHint` stays `no`
//! even while the screen is locked.
//!
//! The thread blocks in `poll()` while nothing happens, so the first input
//! after a pause is seen at once. After an input it sleeps a moment before
//! reading again, so a moving mouse costs a wakeup every
//! [`INPUT_SAMPLE`], not one per event.

use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// After an input, wait this long before reading the devices again.
const INPUT_SAMPLE: Duration = Duration::from_secs(2);

/// Watchers hear about an input only when the one before it is at least this
/// old: they care about the user coming back, not about every keystroke.
const RETURN_GAP: Duration = Duration::from_secs(5);

/// The longest the thread blocks, so it notices a suspend soon after resume
/// even with no input.
const POLL_TIMEOUT_MS: i32 = 60_000;

/// What last showed that the user is here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// The daemon started.
    Start,
    /// Keyboard, mouse or touchpad input from a real device.
    Input,
    /// Input from a virtual (uinput) device, such as a remote-desktop
    /// server's keyboard and mouse.
    RemoteInput,
    /// The computer woke from suspend.
    Resume,
    /// A recording started.
    Recording,
}

impl Activity {
    pub fn describe(self) -> &'static str {
        match self {
            Activity::Start => "daemon start",
            Activity::Input => "keyboard or mouse input",
            Activity::RemoteInput => "remote-desktop input",
            Activity::Resume => "resume from suspend",
            Activity::Recording => "a recording",
        }
    }
}

struct State {
    last: Instant,
    kind: Activity,
    /// The same, leaving out [`Activity::RemoteInput`].
    last_local: Instant,
    kind_local: Activity,
}

/// The time of the user's last activity, and who to tell when they return.
pub struct Presence {
    state: Mutex<State>,
    watchers: Mutex<Vec<Sender<()>>>,
}

impl Presence {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            state: Mutex::new(State {
                last: now,
                kind: Activity::Start,
                last_local: now,
                kind_local: Activity::Start,
            }),
            watchers: Mutex::new(Vec::new()),
        }
    }

    /// The user did something at `now`. Watchers are told when it ends a
    /// pause of at least [`RETURN_GAP`] (local input after local quiet, or
    /// remote input after any quiet), or when it is not input.
    pub fn mark_at(&self, kind: Activity, now: Instant) {
        let returned = {
            let Ok(mut st) = self.state.lock() else {
                return;
            };
            let gap = now.saturating_duration_since(st.last);
            let local_gap = now.saturating_duration_since(st.last_local);
            if now > st.last {
                st.last = now;
            }
            st.kind = kind;
            if kind != Activity::RemoteInput {
                if now > st.last_local {
                    st.last_local = now;
                }
                st.kind_local = kind;
            }
            match kind {
                Activity::Input => local_gap >= RETURN_GAP,
                Activity::RemoteInput => gap >= RETURN_GAP,
                _ => true,
            }
        };
        if returned {
            if let Ok(mut watchers) = self.watchers.lock() {
                watchers.retain(|tx| tx.send(()).is_ok());
            }
        }
    }

    pub fn mark(&self, kind: Activity) {
        self.mark_at(kind, Instant::now());
    }

    /// Set the last activity, also back in time.
    #[cfg(test)]
    pub(crate) fn set_last(&self, last: Instant, kind: Activity) {
        let mut st = self.state.lock().unwrap();
        st.last = last;
        st.kind = kind;
        st.last_local = last;
        st.kind_local = kind;
    }

    /// When the user was last active, and what showed it.
    pub fn last(&self) -> (Instant, Activity) {
        self.state
            .lock()
            .map(|st| (st.last, st.kind))
            .unwrap_or((Instant::now(), Activity::Start))
    }

    /// When the user was last active on this machine itself (no remote
    /// input), and what showed it.
    pub fn last_local(&self) -> (Instant, Activity) {
        self.state
            .lock()
            .map(|st| (st.last_local, st.kind_local))
            .unwrap_or((Instant::now(), Activity::Start))
    }

    /// Send `()` to `tx` when the user returns after a pause. Dropped once
    /// its receiver is gone.
    pub fn watch(&self, tx: Sender<()>) {
        if let Ok(mut watchers) = self.watchers.lock() {
            watchers.push(tx);
        }
    }
}

/// A presence file older than this is not trusted.
const FILE_FRESH_SECS: f64 = 60.0;

/// One reading of an external presence file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileReading {
    pub state: String,
    /// How long the file says the state has held (from `"since"`).
    pub since_age: Option<Duration>,
    pub reason: String,
}

/// Parse a presence file: JSON with `"state"` and `"updated"` (Unix
/// seconds), optionally `"since"` (Unix seconds) and `"reason"`. None when it
/// is not that, or `updated` is not within the last [`FILE_FRESH_SECS`].
pub fn parse_presence(text: &str, now_epoch: f64) -> Option<FileReading> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let state = v["state"].as_str()?.trim();
    let updated = v["updated"].as_f64()?;
    let age = now_epoch - updated;
    if state.is_empty() || !(-5.0..FILE_FRESH_SECS).contains(&age) {
        return None;
    }
    Some(FileReading {
        state: state.to_string(),
        since_age: v["since"]
            .as_f64()
            .map(|since| Duration::from_secs_f64((now_epoch - since).max(0.0))),
        reason: v["reason"]
            .as_str()
            .unwrap_or("no reason given")
            .to_string(),
    })
}

/// Read and parse a presence file. None when it is missing, unreadable,
/// malformed or stale.
pub fn read_presence_file(path: &Path) -> Option<FileReading> {
    let text = std::fs::read_to_string(path).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    parse_presence(&text, now)
}

/// The daemon's presence tracker. The first call starts the input thread;
/// the daemon's start counts as the first activity.
pub fn global() -> Arc<Presence> {
    static PRESENCE: OnceLock<Arc<Presence>> = OnceLock::new();
    PRESENCE
        .get_or_init(|| {
            let presence = Arc::new(Presence::new(Instant::now()));
            let monitor = Arc::clone(&presence);
            let spawned = std::thread::Builder::new()
                .name("voxtype-presence".into())
                .spawn(move || watch_input(&monitor));
            if let Err(e) = spawned {
                tracing::warn!("Cannot watch for keyboard and mouse input: {}", e);
            }
            presence
        })
        .clone()
}

/// Counts as a keyboard: has letters and Enter, like the hotkey listener's.
/// Counts as a pointer: a mouse (relative X with a left button), or a
/// touchpad, touchscreen or tablet (absolute X with touch, pen or button).
/// Switches, power buttons and accelerometers are none of these.
fn is_user_input(device: &evdev::Device) -> bool {
    use evdev::{AbsoluteAxisCode, KeyCode as Key, RelativeAxisCode};
    if device
        .name()
        .is_some_and(crate::hotkey::evdev_listener::is_injection_keyboard)
    {
        // voxtype's own typed output, not the user.
        return false;
    }
    let keys = device.supported_keys();
    let has_key = |k| keys.is_some_and(|keys| keys.contains(k));
    let keyboard = has_key(Key::KEY_A) && has_key(Key::KEY_Z) && has_key(Key::KEY_ENTER);
    let mouse = device
        .supported_relative_axes()
        .is_some_and(|a| a.contains(RelativeAxisCode::REL_X))
        && has_key(Key::BTN_LEFT);
    let touch = device
        .supported_absolute_axes()
        .is_some_and(|a| a.contains(AbsoluteAxisCode::ABS_X))
        && (has_key(Key::BTN_TOUCH) || has_key(Key::BTN_TOOL_PEN) || has_key(Key::BTN_LEFT));
    keyboard || mouse || touch
}

/// Whether an event device is a uinput device (a remote-desktop server's,
/// for example) rather than real hardware. Bluetooth LE input goes through
/// uhid and stays real.
fn is_virtual(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    std::fs::canonicalize(Path::new("/sys/class/input").join(name).join("device"))
        .is_ok_and(|p| p.to_string_lossy().contains("/devices/virtual/input/"))
}

/// The keyboards and pointers under `dir`, opened non-blocking.
fn open_devices(dir: &Path) -> Vec<(PathBuf, evdev::Device, bool)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"))
        {
            continue;
        }
        let Ok(device) = evdev::Device::open(&path) else {
            continue;
        };
        if !is_user_input(&device) {
            continue;
        }
        let fd = device.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags != -1 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        let remote = is_virtual(&path);
        devices.push((path, device, remote));
    }
    devices
}

/// Read everything waiting on `fd`. Some(true) if there was any, None if the
/// device is gone.
fn drain(fd: RawFd) -> Option<bool> {
    let mut buf = [0u8; 24 * 64];
    let mut got = false;
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            got = true;
            continue;
        }
        if n == 0 {
            return None;
        }
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EAGAIN) | Some(libc::EINTR) => Some(got),
            _ => None,
        };
    }
}

fn clock_secs(clock: libc::clockid_t) -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// Notices a suspend: the boot clock runs on through it, the monotonic clock
/// does not.
struct SleepClock {
    offset: f64,
}

impl SleepClock {
    fn new() -> Self {
        Self {
            offset: Self::offset_now(),
        }
    }

    fn offset_now() -> f64 {
        clock_secs(libc::CLOCK_BOOTTIME) - clock_secs(libc::CLOCK_MONOTONIC)
    }

    /// Whether the computer slept since the last call.
    fn slept(&mut self) -> bool {
        let now = Self::offset_now();
        let slept = now - self.offset > 5.0;
        self.offset = now;
        slept
    }
}

fn watch_input(presence: &Presence) {
    let dir = Path::new("/dev/input");
    let mut inotify = inotify::Inotify::init().ok().filter(|i| {
        i.watches()
            .add(dir, inotify::WatchMask::CREATE | inotify::WatchMask::ATTRIB)
            .is_ok()
    });
    if let Some(ref i) = inotify {
        let fd = i.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags != -1 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
    }
    let mut devices = open_devices(dir);
    if devices.is_empty() {
        tracing::warn!(
            "No keyboard or mouse readable under /dev/input (is the user in the 'input' group?). \
             The warm live worker then only counts recordings as activity."
        );
    } else {
        tracing::info!(
            "Watching {} keyboard and pointer devices for presence",
            devices.len()
        );
    }
    let mut sleep_clock = SleepClock::new();
    let mut inotify_buf = [0u8; 1024];
    loop {
        let mut fds: Vec<libc::pollfd> = devices
            .iter()
            .map(|(_, d, _)| libc::pollfd {
                fd: d.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        if let Some(ref i) = inotify {
            fds.push(libc::pollfd {
                fd: i.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let ready =
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, POLL_TIMEOUT_MS) };
        let mut rescan = false;
        if sleep_clock.slept() {
            tracing::info!("Resumed from suspend");
            presence.mark(Activity::Resume);
            rescan = true;
        }
        if ready < 0 {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let mut input = false;
        let mut remote_input = false;
        let mut gone = Vec::new();
        for (i, (path, device, remote)) in devices.iter().enumerate() {
            let revents = fds[i].revents;
            if revents == 0 {
                continue;
            }
            if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                gone.push(path.clone());
                continue;
            }
            match drain(device.as_raw_fd()) {
                Some(got) if *remote => remote_input |= got,
                Some(got) => input |= got,
                None => gone.push(path.clone()),
            }
        }
        if !gone.is_empty() {
            devices.retain(|(p, _, _)| !gone.contains(p));
        }
        if let Some(ref mut i) = inotify {
            if fds.last().is_some_and(|f| f.revents != 0) {
                while i
                    .read_events(&mut inotify_buf)
                    .is_ok_and(|mut e| e.next().is_some())
                {}
                rescan = true;
            }
        }
        if rescan {
            // udev sets the group a moment after the node appears.
            std::thread::sleep(Duration::from_millis(500));
            for (path, device, remote) in open_devices(dir) {
                if !devices.iter().any(|(p, _, _)| *p == path) {
                    tracing::debug!("Presence: watching {:?}", path);
                    devices.push((path, device, remote));
                }
            }
        }
        if remote_input {
            presence.mark(Activity::RemoteInput);
        }
        if input {
            presence.mark(Activity::Input);
        }
        if input || remote_input {
            std::thread::sleep(INPUT_SAMPLE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Lists the devices this machine's presence watch reads.
    /// `cargo test --lib presence::tests::list_devices -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn list_devices() {
        for (path, device, remote) in open_devices(Path::new("/dev/input")) {
            println!("{:?} {:?} remote={}", path, device.name(), remote);
        }
    }

    #[test]
    fn a_fresh_presence_file_is_read() {
        let text = r#"{"state": "remote", "since": 1000.0, "updated": 1990.0, "reason": "RDP client connected"}"#;
        let r = parse_presence(text, 2000.0).unwrap();
        assert_eq!(r.state, "remote");
        assert_eq!(r.since_age, Some(Duration::from_secs(1000)));
        assert_eq!(r.reason, "RDP client connected");
    }

    #[test]
    fn a_stale_or_broken_presence_file_is_not_trusted() {
        let text = r#"{"state": "here", "updated": 1000.0}"#;
        assert!(parse_presence(text, 1059.0).is_some());
        assert!(parse_presence(text, 1061.0).is_none(), "stale");
        assert!(parse_presence(text, 900.0).is_none(), "from the future");
        assert!(parse_presence(r#"{"state": "here"}"#, 1000.0).is_none());
        assert!(parse_presence("here", 1000.0).is_none());
        assert!(read_presence_file(Path::new("/nonexistent/presence")).is_none());
    }

    #[test]
    fn remote_input_does_not_move_the_local_clock() {
        let t0 = Instant::now();
        let p = Presence::new(t0);
        let (tx, rx) = mpsc::channel();
        p.watch(tx);
        p.mark_at(Activity::RemoteInput, t0 + Duration::from_secs(60));
        assert!(rx.try_recv().is_ok(), "a return over remote desktop");
        assert_eq!(p.last().0, t0 + Duration::from_secs(60));
        assert_eq!(p.last_local(), (t0, Activity::Start));
        // Local input after local quiet is a return, though remote input was recent.
        p.mark_at(Activity::Input, t0 + Duration::from_secs(61));
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            p.last_local(),
            (t0 + Duration::from_secs(61), Activity::Input)
        );
    }

    #[test]
    fn a_return_after_a_pause_tells_the_watchers() {
        let t0 = Instant::now();
        let p = Presence::new(t0);
        let (tx, rx) = mpsc::channel();
        p.watch(tx);
        p.mark_at(Activity::Input, t0 + Duration::from_secs(1));
        assert!(rx.try_recv().is_err(), "typing on is not a return");
        p.mark_at(Activity::Input, t0 + Duration::from_secs(60));
        assert!(rx.try_recv().is_ok());
        assert_eq!(p.last(), (t0 + Duration::from_secs(60), Activity::Input));
    }

    #[test]
    fn resume_and_recording_always_tell_the_watchers() {
        let t0 = Instant::now();
        let p = Presence::new(t0);
        let (tx, rx) = mpsc::channel();
        p.watch(tx);
        p.mark_at(Activity::Resume, t0);
        assert!(rx.try_recv().is_ok());
        p.mark_at(Activity::Recording, t0);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn a_dropped_watcher_is_forgotten() {
        let p = Presence::new(Instant::now());
        let (tx, rx) = mpsc::channel();
        p.watch(tx);
        drop(rx);
        p.mark(Activity::Resume);
        assert!(p.watchers.lock().unwrap().is_empty());
    }

    #[test]
    fn an_older_mark_does_not_move_the_clock_back() {
        let t0 = Instant::now();
        let p = Presence::new(t0 + Duration::from_secs(10));
        p.mark_at(Activity::Input, t0);
        assert_eq!(p.last().0, t0 + Duration::from_secs(10));
    }
}
