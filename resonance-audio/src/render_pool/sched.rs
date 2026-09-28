//! Thread scheduling for render workers: read the rendering thread's own
//! policy and give a worker the same one (realtime-multithreading.md
//! §4.3).
//!
//! A worker must run at the priority of the thread it helps. A worker at
//! normal priority preempted mid-job stalls the join and the whole block
//! xruns, which is worse than rendering serially — so a worker that cannot
//! match a realtime caller reports it and the pool stops using workers.
//!
//! Only `pthread_setschedparam` is tried: it works whenever
//! `RLIMIT_RTPRIO` allows (the usual `audio`-group setup). PipeWire's own
//! RTKit path is not reachable through the `pipewire` crate, and RTKit
//! over D-Bus is not implemented (realtime-multithreading.md §9).

/// A thread's scheduling class, as far as the pool cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sched {
    /// Time-sharing (`SCHED_OTHER` and friends).
    Normal,
    /// `SCHED_FIFO` at this priority.
    Fifo(i32),
    /// `SCHED_RR` at this priority.
    RoundRobin(i32),
}

impl Sched {
    pub fn is_realtime(self) -> bool {
        !matches!(self, Self::Normal)
    }

    /// Packed for an atomic: `0` = not yet known, then `1 + variant`
    /// in the low byte and the priority above it.
    pub(crate) fn pack(self) -> u64 {
        match self {
            Self::Normal => 1,
            Self::Fifo(p) => 2 | ((p as u32 as u64) << 8),
            Self::RoundRobin(p) => 3 | ((p as u32 as u64) << 8),
        }
    }

    pub(crate) fn unpack(bits: u64) -> Option<Self> {
        let prio = (bits >> 8) as u32 as i32;
        match bits & 0xff {
            1 => Some(Self::Normal),
            2 => Some(Self::Fifo(prio)),
            3 => Some(Self::RoundRobin(prio)),
            _ => None,
        }
    }
}

/// The calling thread's scheduling class. One syscall; allocation-free.
#[cfg(target_os = "linux")]
pub fn current() -> Sched {
    let mut policy: libc::c_int = 0;
    // SAFETY: plain out-parameters for the calling thread.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::pthread_getschedparam(libc::pthread_self(), &mut policy, &mut param) };
    if rc != 0 {
        return Sched::Normal;
    }
    match policy & !libc::SCHED_RESET_ON_FORK {
        libc::SCHED_FIFO => Sched::Fifo(param.sched_priority),
        libc::SCHED_RR => Sched::RoundRobin(param.sched_priority),
        _ => Sched::Normal,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn current() -> Sched {
    Sched::Normal
}

/// Give the calling thread `sched`. `Normal` always succeeds (a worker
/// is created at normal priority). Returns the OS error on failure.
#[cfg(target_os = "linux")]
pub fn apply(sched: Sched) -> Result<(), std::io::Error> {
    let (policy, prio) = match sched {
        Sched::Normal => return Ok(()),
        Sched::Fifo(p) => (libc::SCHED_FIFO, p),
        Sched::RoundRobin(p) => (libc::SCHED_RR, p),
    };
    // SAFETY: plain in-parameter for the calling thread.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = prio;
    // Reset-on-fork, like PipeWire's own RT threads: nothing a plugin
    // forks inherits realtime.
    let rc = unsafe {
        libc::pthread_setschedparam(
            libc::pthread_self(),
            policy | libc::SCHED_RESET_ON_FORK,
            &param,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc))
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply(sched: Sched) -> Result<(), std::io::Error> {
    match sched {
        Sched::Normal => Ok(()),
        _ => Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
    }
}

/// Physical cores (SMT siblings counted once), from sysfs topology;
/// falls back to the logical CPU count. Allocates; engine side.
pub fn physical_cores() -> usize {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    #[cfg(target_os = "linux")]
    {
        let mut cores = std::collections::BTreeSet::new();
        for cpu in 0..logical.max(1) {
            let base = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
            let read = |f: &str| std::fs::read_to_string(format!("{base}/{f}")).ok();
            if let (Some(pkg), Some(core)) = (read("physical_package_id"), read("core_id")) {
                cores.insert((pkg.trim().to_owned(), core.trim().to_owned()));
            }
        }
        if !cores.is_empty() {
            return cores.len();
        }
    }
    logical
}
