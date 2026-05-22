use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::time::Instant;

use super::common::{self, NET_HISTORY_SIZE};

#[repr(C)]
struct Statvfs {
    bsize: u64,
    frsize: u64,
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    favail: u64,
    fsid: u64,
    flag: u64,
    namemax: u64,
}

#[link(name = "c")]
unsafe extern "C" {
    fn statvfs(path: *const i8, buf: *mut Statvfs) -> i32;
}

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    idle: u64,
    total: u64,
}

pub struct Monitor {
    cpu_prev: Vec<CpuTimes>,
    net: NetState,
}

struct NetState {
    last_recv: u64,
    last_sent: u64,
    last_time: Instant,
    recv_rate: f64,
    sent_rate: f64,
    primed: bool,
    recv_history: Vec<f64>,
    sent_history: Vec<f64>,
}

impl Monitor {
    pub fn new() -> Self {
        let count = logical_cpu_count().max(1);
        Self {
            cpu_prev: vec![CpuTimes::default(); count],
            net: NetState::default(),
        }
    }

    pub fn cpu_percents(&mut self) -> Vec<f64> {
        let current = read_cpu_times();
        let mut out = Vec::with_capacity(current.len());
        for (i, cur) in current.iter().enumerate() {
            let prev = self.cpu_prev.get(i).copied().unwrap_or_default();
            let idle_d = cur.idle.saturating_sub(prev.idle);
            let total_d = cur.total.saturating_sub(prev.total);
            let pct = if total_d > 0 {
                (1.0 - idle_d as f64 / total_d as f64) * 100.0
            } else {
                0.0
            };
            out.push(pct.max(0.0).min(100.0));
        }
        self.cpu_prev = current;
        out
    }

    pub fn memory(&self) -> Option<(f64, u64, u64)> {
        let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
        let mut total = 0u64;
        let mut available = 0u64;
        for line in meminfo.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let val: u64 = parts.next()?.parse().ok()?;
            match key {
                "MemTotal:" => total = val * 1024,
                "MemAvailable:" => available = val * 1024,
                _ => {}
            }
        }
        if total == 0 {
            return None;
        }
        let used = total.saturating_sub(available);
        let pct = used as f64 / total as f64 * 100.0;
        Some((pct, used, total))
    }

    pub fn swap(&self) -> Option<(f64, u64, u64)> {
        let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
        let mut total = 0u64;
        let mut free = 0u64;
        for line in meminfo.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let val: u64 = parts.next()?.parse().ok()?;
            match key {
                "SwapTotal:" => total = val * 1024,
                "SwapFree:" => free = val * 1024,
                _ => {}
            }
        }
        if total == 0 {
            return None;
        }
        let used = total.saturating_sub(free);
        Some((used as f64 / total as f64 * 100.0, used, total))
    }

    pub fn mountpoints(&self) -> Vec<String> {
        let content = fs::read_to_string("/proc/mounts")
            .or_else(|_| fs::read_to_string("/etc/mtab"))
            .unwrap_or_default();
        let mut seen = HashSet::new();
        let mut mounts = Vec::new();
        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let _device = parts.next();
            let mount = parts.next().unwrap_or("");
            let fstype = parts.next().unwrap_or("");
            if mount.is_empty() || should_skip_mount(mount, fstype) {
                continue;
            }
            if seen.insert(mount.to_string()) {
                mounts.push(mount.to_string());
            }
        }
        mounts.sort();
        mounts
    }

    pub fn disk_usage(&self, mount: &str) -> Option<(f64, u64, u64)> {
        let path = CString::new(mount).ok()?;
        let mut st: Statvfs = unsafe { std::mem::zeroed() };
        if unsafe { statvfs(path.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let total = st.blocks.saturating_mul(st.frsize);
        let avail = st.bavail.saturating_mul(st.frsize);
        if total == 0 {
            return None;
        }
        let used = total.saturating_sub(avail);
        Some((used as f64 / total as f64 * 100.0, used, total))
    }

    pub fn sample_network(&mut self) {
        let (recv, sent) = match read_net_bytes() {
            Some(v) => v,
            None => return,
        };
        let now = Instant::now();
        if self.net.primed {
            let dt = now.duration_since(self.net.last_time).as_secs_f64();
            if dt >= 0.2 {
                self.net.recv_rate = (recv.saturating_sub(self.net.last_recv)) as f64 / dt;
                self.net.sent_rate = (sent.saturating_sub(self.net.last_sent)) as f64 / dt;
                common::push_bounded(&mut self.net.recv_history, self.net.recv_rate, NET_HISTORY_SIZE);
                common::push_bounded(&mut self.net.sent_history, self.net.sent_rate, NET_HISTORY_SIZE);
            }
        }
        self.net.last_recv = recv;
        self.net.last_sent = sent;
        self.net.last_time = now;
        self.net.primed = true;
    }

    pub fn network(&self) -> (&[f64], &[f64], f64, f64, u64, u64) {
        (
            &self.net.recv_history,
            &self.net.sent_history,
            self.net.recv_rate,
            self.net.sent_rate,
            self.net.last_recv,
            self.net.last_sent,
        )
    }
}

impl Default for NetState {
    fn default() -> Self {
        Self {
            last_recv: 0,
            last_sent: 0,
            last_time: Instant::now(),
            recv_rate: 0.0,
            sent_rate: 0.0,
            primed: false,
            recv_history: Vec::new(),
            sent_history: Vec::new(),
        }
    }
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

fn logical_cpu_count() -> usize {
    fs::read_to_string("/proc/stat")
        .map(|s| s.lines().filter(|l| l.starts_with("cpu") && !l.starts_with("cpu ")).count())
        .unwrap_or(1)
}

fn read_cpu_times() -> Vec<CpuTimes> {
    let Ok(content) = fs::read_to_string("/proc/stat") else {
        return vec![CpuTimes::default()];
    };
    let mut cpus = Vec::new();
    for line in content.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let label = line.split_whitespace().next().unwrap_or("");
        if label == "cpu" {
            continue;
        }
        let nums: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() < 4 {
            continue;
        }
        let user = nums[0];
        let nice = nums.get(1).copied().unwrap_or(0);
        let system = nums[2];
        let idle = nums[3];
        let iowait = nums.get(4).copied().unwrap_or(0);
        let irq = nums.get(5).copied().unwrap_or(0);
        let softirq = nums.get(6).copied().unwrap_or(0);
        let steal = nums.get(7).copied().unwrap_or(0);
        let guest = nums.get(8).copied().unwrap_or(0);
        let guest_nice = nums.get(9).copied().unwrap_or(0);
        let idle_all = idle + iowait;
        let total = user + nice + system + idle + iowait + irq + softirq + steal + guest + guest_nice;
        cpus.push(CpuTimes {
            idle: idle_all,
            total,
        });
    }
    if cpus.is_empty() {
        cpus.push(CpuTimes::default());
    }
    cpus
}

fn read_net_bytes() -> Option<(u64, u64)> {
    let file = fs::File::open("/proc/net/dev").ok()?;
    let reader = BufReader::new(file);
    let mut recv = 0u64;
    let mut sent = 0u64;
    for (i, line) in reader.lines().enumerate() {
        let line = line.ok()?;
        if i < 2 {
            continue;
        }
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let iface = parts.next()?.trim_end_matches(':');
        if iface == "lo" {
            continue;
        }
        let r: u64 = parts.next()?.parse().ok()?;
        let _packets = parts.next()?;
        let _errs = parts.next()?;
        let _drop = parts.next()?;
        let _fifo = parts.next()?;
        let _frame = parts.next()?;
        let _compressed = parts.next()?;
        let _multicast = parts.next()?;
        let s: u64 = parts.next()?.parse().ok()?;
        recv = recv.saturating_add(r);
        sent = sent.saturating_add(s);
    }
    Some((recv, sent))
}

fn should_skip_mount(mount: &str, fstype: &str) -> bool {
    let fstype = fstype.to_lowercase();
    const SKIP_FS: &[&str] = &[
        "squashfs", "tmpfs", "devtmpfs", "overlay", "proc", "sysfs", "cgroup", "cgroup2",
        "devpts", "mqueue", "pstore", "fuse.gvfsd-fuse",
    ];
    if SKIP_FS.contains(&fstype.as_str()) {
        return true;
    }
    mount.starts_with("/proc")
        || mount.starts_with("/sys")
        || mount.starts_with("/dev")
        || mount.starts_with("/run")
        || mount.starts_with("/snap")
        || mount.starts_with("/var/lib/docker")
}
