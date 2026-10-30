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
    /// Keyboard, mouse or touchpad input.
    Input,
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
            Activity::Resume => "resume from suspend",
            Activity::Recording => "a recording",
        }
    }
}

struct State {
    last: Instant,
    kind: Activity,
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
            }),
            watchers: Mutex::new(Vec::new()),
        }
    }

    /// The user did something at `now`. Watchers are told when it ends a
    /// pause of at least [`RETURN_GAP`], or when it is not plain input.
    pub fn mark_at(&self, kind: Activity, now: Instant) {
        let returned = {
            let Ok(mut st) = self.state.lock() else {
                return;
            };
            let gap = now.saturating_duration_since(st.last);
            if now > st.last {
                st.last = now;
            }
            st.kind = kind;
            kind != Activity::Input || gap >= RETURN_GAP
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
    }

    /// When the user was last active, and what showed it.
    pub fn last(&self) -> (Instant, Activity) {
        self.state
            .lock()
            .map(|st| (st.last, st.kind))
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

/// The keyboards and pointers under `dir`, opened non-blocking.
fn open_devices(dir: &Path) -> Vec<(PathBuf, evdev::Device)> {
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
        devices.push((path, device));
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
            .map(|(_, d)| libc::pollfd {
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
        let mut gone = Vec::new();
        for (i, (path, device)) in devices.iter().enumerate() {
            let revents = fds[i].revents;
            if revents == 0 {
                continue;
            }
            if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                gone.push(path.clone());
                continue;
            }
            match drain(device.as_raw_fd()) {
                Some(got) => input |= got,
                None => gone.push(path.clone()),
            }
        }
        if !gone.is_empty() {
            devices.retain(|(p, _)| !gone.contains(p));
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
            for (path, device) in open_devices(dir) {
                if !devices.iter().any(|(p, _)| *p == path) {
                    tracing::debug!("Presence: watching {:?}", path);
                    devices.push((path, device));
                }
            }
        }
        if input {
            presence.mark(Activity::Input);
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
        for (path, device) in open_devices(Path::new("/dev/input")) {
            println!("{:?} {:?}", path, device.name());
        }
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
