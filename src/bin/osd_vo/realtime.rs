//! Top CPU priority for the popup while it shows (`[osd] realtime`).
//!
//! The popup is what the user is watching while they dictate, so a busy
//! machine must not make it stutter. While a popup shows, the popup's own
//! threads (the thread that draws it and the threads that read the
//! daemon's audio and files, see [`register_thread`]) run `SCHED_RR` at
//! priority 1, the lowest real-time priority: any normal task waits for
//! them, and the compositor (KWin runs `SCHED_RR` 1 too) shares the CPU
//! with them. When the popup hides, they go back to `SCHED_OTHER`. Threads
//! they start meanwhile begin as normal threads (`SCHED_RESET_ON_FORK`).
//! Threads the GPU driver starts are left alone: some spin while they wait
//! for the GPU.
//!
//! A real-time thread that never sleeps would hold its CPU, and many
//! desktops turn off the kernel's real-time throttling. `RLIMIT_RTTIME`
//! bounds that: when a raised thread runs 100 ms on end without blocking,
//! the kernel sends `SIGXCPU`, and the handler puts every raised thread
//! back to `SCHED_OTHER` for the rest of that popup. The hard limit (1 s)
//! kills the process only if the handler could not run. A popup frame
//! takes a few milliseconds and then waits for the compositor, so the
//! limit is not near in normal use.
//!
//! Setting `SCHED_RR` needs `RLIMIT_RTPRIO` of at least 1 (the `audio` or
//! `realtime` group on most distributions). Without it the threads get
//! the lowest nice value `RLIMIT_NICE` allows, down to -10, and without
//! that they keep their priority.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, Once};

/// The real-time priority: the lowest, the same as KWin's.
const RT_PRIORITY: i32 = 1;
/// The nice value used when `SCHED_RR` is not allowed.
const BOOST_NICE: i32 = -10;
/// The policy a raised thread goes back to. Only a privileged thread may
/// clear `SCHED_RESET_ON_FORK` once set, so it stays; on a normal thread it
/// only means its new threads start at nice 0.
const NORMAL: libc::c_int = libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK;
/// Microseconds a real-time thread may run without blocking before
/// `SIGXCPU`.
const RTTIME_SOFT_US: u64 = 100_000;
/// After this many, the kernel kills the process.
const RTTIME_HARD_US: u64 = 1_000_000;

/// Threads raised to `SCHED_RR`, for the `SIGXCPU` handler, which may
/// only use plain loads and system calls. 0 is an empty slot.
static RAISED: [AtomicI32; 8] = [const { AtomicI32::new(0) }; 8];
/// Set by the `SIGXCPU` handler; read and cleared when the popup hides.
static DEMOTED: AtomicBool = AtomicBool::new(false);
/// Threads besides the caller of [`raise`] that are raised with it.
static REGISTERED: Mutex<Vec<libc::pid_t>> = Mutex::new(Vec::new());
static SETUP: Once = Once::new();

/// How a thread was raised.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Raised {
    Realtime,
    Nice(i32),
}

/// The raised threads; dropping it puts them back.
pub(crate) struct Boost {
    tids: Vec<(libc::pid_t, Raised)>,
}

/// Marks the calling thread as one of the popup's own, raised with the
/// drawing thread while a popup shows.
pub(crate) fn register_thread() {
    unblock_sigxcpu();
    let tid = unsafe { libc::gettid() };
    REGISTERED.lock().unwrap().push(tid);
}

/// Raises the calling thread and the registered threads while a popup
/// shows. Returns `None` when `enabled` is false or nothing could be
/// raised.
pub(crate) fn raise(enabled: bool) -> Option<Boost> {
    if !enabled {
        return None;
    }
    SETUP.call_once(|| {
        limit_rttime();
        install_sigxcpu_handler();
    });
    unblock_sigxcpu();
    let mut wanted = vec![unsafe { libc::gettid() }];
    wanted.extend(REGISTERED.lock().unwrap().iter().copied());
    let mut tids = Vec::new();
    let mut failed = None;
    for tid in wanted {
        match raise_thread(tid) {
            Ok(how) => tids.push((tid, how)),
            Err(e) => failed = Some(e),
        }
    }
    let realtime = tids.iter().filter(|(_, how)| *how == Raised::Realtime);
    for (slot, (tid, _)) in RAISED.iter().zip(realtime) {
        slot.store(*tid, Ordering::Relaxed);
    }
    match (tids.first(), failed) {
        (None, Some(e)) => {
            tracing::debug!("popup priority unchanged: {e}");
            None
        }
        (None, None) => None,
        (Some((_, how)), _) => {
            tracing::debug!(threads = tids.len(), ?how, "popup priority raised");
            Some(Boost { tids })
        }
    }
}

impl Drop for Boost {
    fn drop(&mut self) {
        for slot in &RAISED {
            slot.store(0, Ordering::Relaxed);
        }
        for &(tid, how) in &self.tids {
            // A thread that ended since is gone: ESRCH, nothing to do.
            match how {
                Raised::Realtime => {
                    set_scheduler(tid, NORMAL, 0);
                }
                Raised::Nice(old) => unsafe {
                    libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, old);
                },
            }
        }
        if DEMOTED.swap(false, Ordering::Relaxed) {
            tracing::warn!(
                "a popup thread ran {} ms on end at real-time priority; the popup ran at normal priority from then on",
                RTTIME_SOFT_US / 1000
            );
        }
    }
}

fn raise_thread(tid: libc::pid_t) -> Result<Raised, std::io::Error> {
    // Only normal threads: one that is real-time, batch or idle already
    // keeps what it has, and hiding does not touch it.
    let policy = unsafe { libc::sched_getscheduler(tid) };
    if policy < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if policy & !libc::SCHED_RESET_ON_FORK != libc::SCHED_OTHER {
        return Err(std::io::Error::other("not a normal thread"));
    }
    if set_scheduler(tid, libc::SCHED_RR | libc::SCHED_RESET_ON_FORK, RT_PRIORITY) {
        return Ok(Raised::Realtime);
    }
    let rr = std::io::Error::last_os_error();
    let lowest = lowest_nice();
    if lowest >= 0 {
        return Err(rr);
    }
    // getpriority returns -1 both for nice -1 and for an error.
    clear_errno();
    let old = unsafe { libc::getpriority(libc::PRIO_PROCESS, tid as libc::id_t) };
    if old == -1 && errno() != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let nice = BOOST_NICE.max(lowest);
    if nice >= old {
        return Err(rr);
    }
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Raised::Nice(old))
}

fn set_scheduler(tid: libc::pid_t, policy: libc::c_int, priority: i32) -> bool {
    let param = libc::sched_param {
        sched_priority: priority,
    };
    unsafe { libc::sched_setscheduler(tid, policy, &param) == 0 }
}

/// The lowest nice value `RLIMIT_NICE` lets an unprivileged process set.
fn lowest_nice() -> i32 {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NICE, &mut lim) } != 0 {
        return 0;
    }
    // The limit is 20 - nice: 31 allows nice -11.
    20 - lim.rlim_cur.min(40) as i32
}

/// Lowers `RLIMIT_RTTIME` to the limits above, keeping a lower one.
fn limit_rttime() {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut lim) } != 0 {
        return;
    }
    let new = libc::rlimit {
        rlim_cur: lim.rlim_cur.min(RTTIME_SOFT_US),
        rlim_max: lim.rlim_max.min(RTTIME_HARD_US),
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &new) } != 0 {
        tracing::debug!(
            "cannot limit real-time CPU time: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// `SIGXCPU` may arrive on any thread that does not block it, so the
/// handler puts back every raised thread, not only its own.
fn install_sigxcpu_handler() {
    extern "C" fn on_sigxcpu(_: libc::c_int) {
        let param = libc::sched_param { sched_priority: 0 };
        for slot in &RAISED {
            let tid = slot.load(Ordering::Relaxed);
            if tid != 0 {
                // A plain system call: safe in a signal handler.
                unsafe { libc::sched_setscheduler(tid, NORMAL, &param) };
            }
        }
        DEMOTED.store(true, Ordering::Relaxed);
    }
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigxcpu as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut());
    }
}

/// A thread that blocks `SIGXCPU` (inherited from whoever started us)
/// could not run the handler, and the hard limit would kill the process.
fn unblock_sigxcpu() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGXCPU);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(tid: libc::pid_t) -> libc::c_int {
        unsafe { libc::sched_getscheduler(tid) & !libc::SCHED_RESET_ON_FORK }
    }

    fn nice(tid: libc::pid_t) -> i32 {
        unsafe { libc::getpriority(libc::PRIO_PROCESS, tid as libc::id_t) }
    }

    #[test]
    fn off_raises_nothing() {
        assert!(raise(false).is_none());
    }

    /// One test for raise, hide and SIGXCPU: they share the process-wide
    /// slots and limits.
    #[test]
    fn raises_while_shown_and_puts_back() {
        std::thread::spawn(|| {
            let me = unsafe { libc::gettid() };
            let before = (policy(me), nice(me));
            let boost = raise(true);
            let how = boost
                .as_ref()
                .and_then(|b| b.tids.iter().find(|(t, _)| *t == me).map(|(_, h)| *h));
            match how {
                Some(Raised::Realtime) => assert_eq!(policy(me), libc::SCHED_RR),
                Some(Raised::Nice(_)) => assert!(nice(me) < before.1),
                // No RLIMIT_RTPRIO or RLIMIT_NICE here: nothing to check.
                None => {}
            }
            // A thread the raised one starts begins as a normal thread.
            let child = std::thread::spawn(|| policy(unsafe { libc::gettid() }))
                .join()
                .unwrap();
            assert_eq!(child, libc::SCHED_OTHER);
            if how == Some(Raised::Realtime) {
                // What the kernel does at the soft RLIMIT_RTTIME limit.
                unsafe { libc::raise(libc::SIGXCPU) };
                assert_eq!(policy(me), libc::SCHED_OTHER);
            }
            drop(boost);
            assert_eq!((policy(me), nice(me)), before);
            assert!(!DEMOTED.load(Ordering::Relaxed));
            let mut lim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut lim) };
            assert!(lim.rlim_cur <= RTTIME_SOFT_US && lim.rlim_max <= RTTIME_HARD_US);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn lowest_nice_reads_the_limit() {
        let n = lowest_nice();
        assert!((-20..=20).contains(&n));
    }
}
