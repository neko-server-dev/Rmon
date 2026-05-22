use std::ffi::{c_void, OsStr};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::time::Instant;

use super::common::{self, NET_HISTORY_SIZE};

const PROCESSOR_PERF_INFO: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Filetime {
    low: u32,
    high: u32,
}

impl Filetime {
    fn to_u64(self) -> u64 {
        ((self.high as u64) << 32) | self.low as u64
    }
}

#[repr(C)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}

#[repr(C)]
struct SystemProcessorPerformanceInformation {
    idle_time: i64,
    kernel_time: i64,
    user_time: i64,
    dpc_time: i64,
    interrupt_time: i64,
    interrupt_count: u32,
}

#[repr(C)]
#[repr(C)]
struct MibIfTable2 {
    num_entries: u32,
}

#[repr(C)]
struct MibIfRow2 {
    interface_luid: u64,
    interface_index: u32,
    interface_guid: [u8; 16],
    alias: [u16; 257],
    description: [u16; 257],
    physical_address_length: u32,
    physical_address: [u8; 32],
    permanent_physical_address: [u8; 32],
    media_type: u32,
    access_type: u32,
    direction_type: u32,
    interface_and_oper_status_flags: u32,
    oper_status: u32,
    admin_status: u32,
    media_connect_state: u32,
    network_guid: [u8; 16],
    connection_type: u32,
    transmit_link_speed: u64,
    receive_link_speed: u64,
    in_octets: u64,
    in_ucast_pkts: u64,
    in_nucast_pkts: u64,
    in_discards: u64,
    in_errors: u64,
    in_unknown_protos: u64,
    in_ucast_octets: u64,
    in_mcast_octets: u64,
    in_bcast_octets: u64,
    out_octets: u64,
    out_ucast_pkts: u64,
    out_nucast_pkts: u64,
    out_discards: u64,
    out_errors: u64,
    out_ucast_octets: u64,
    out_mcast_octets: u64,
    out_bcast_octets: u64,
    out_qlen: u64,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemInfo(info: *mut SystemInfo);
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    fn GetDiskFreeSpaceExW(
        root: *const u16,
        free_avail: *mut u64,
        total: *mut u64,
        total_free: *mut u64,
    ) -> i32;
    fn GetSystemTimes(
        idle: *mut Filetime,
        kernel: *mut Filetime,
        user: *mut Filetime,
    ) -> i32;
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        info: *mut c_void,
        length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetIfTable2(table: *mut *mut MibIfTable2) -> u32;
    fn FreeMibTable(table: *mut c_void) -> u32;
}

#[repr(C)]
struct SystemInfo {
    arch: u16,
    reserved: u16,
    page_size: u32,
    min_app_addr: *mut c_void,
    max_app_addr: *mut c_void,
    active_processor_mask: usize,
    num_processors: u32,
    processor_type: u32,
    allocation_granularity: u32,
    processor_level: u16,
    processor_revision: u16,
}

pub struct Monitor {
    net: NetState,
    cpu_count: usize,
    cpu_prev: Vec<(i64, i64)>,
    sys_times_prev: Option<(u64, u64)>,
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
        let mut info: SystemInfo = unsafe { zeroed() };
        unsafe { GetSystemInfo(&mut info) };
        let count = info.num_processors.max(1) as usize;
        Self {
            net: NetState {
                last_time: Instant::now(),
                ..Default::default()
            },
            cpu_count: count,
            cpu_prev: vec![(0, 0); count],
            sys_times_prev: None,
        }
    }

    pub fn cpu_percents(&mut self) -> Vec<f64> {
        if let Some(per_cpu) = query_per_cpu_performance() {
            if self.cpu_prev.len() != per_cpu.len() {
                self.cpu_prev = vec![(0, 0); per_cpu.len()];
            }
            let mut out = Vec::with_capacity(per_cpu.len());
            for (i, (idle, total)) in per_cpu.iter().enumerate() {
                let (p_idle, p_total) = self.cpu_prev[i];
                let idle_d = idle.saturating_sub(p_idle);
                let total_d = total.saturating_sub(p_total);
                let pct = if total_d > 0 {
                    (1.0 - idle_d as f64 / total_d as f64) * 100.0
                } else {
                    0.0
                };
                out.push(pct.max(0.0).min(100.0));
                self.cpu_prev[i] = (*idle, *total);
            }
            return out;
        }

        // fallback: aggregate GetSystemTimes
        let mut idle = Filetime::default();
        let mut kernel = Filetime::default();
        let mut user = Filetime::default();
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return vec![0.0; self.cpu_count.max(1)];
        }
        let idle_t = idle.to_u64();
        let kernel_t = kernel.to_u64();
        let user_t = user.to_u64();
        let total = kernel_t + user_t;
        let pct = if let Some((pi, pt)) = self.sys_times_prev {
            let idle_d = idle_t.saturating_sub(pi);
            let total_d = total.saturating_sub(pt);
            if total_d > 0 {
                (1.0 - idle_d as f64 / total_d as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        self.sys_times_prev = Some((idle_t, total));
        vec![pct.max(0.0).min(100.0); self.cpu_count.max(1)]
    }

    pub fn memory(&self) -> Option<(f64, u64, u64)> {
        let mut st: MemoryStatusEx = unsafe { zeroed() };
        st.length = size_of::<MemoryStatusEx>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut st) } == 0 {
            return None;
        }
        let used = st.total_phys.saturating_sub(st.avail_phys);
        let pct = used as f64 / st.total_phys as f64 * 100.0;
        Some((pct, used, st.total_phys))
    }

    pub fn swap(&self) -> Option<(f64, u64, u64)> {
        let mut st: MemoryStatusEx = unsafe { zeroed() };
        st.length = size_of::<MemoryStatusEx>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut st) } == 0 {
            return None;
        }
        if st.total_page_file == 0 {
            return None;
        }
        let used = st.total_page_file.saturating_sub(st.avail_page_file);
        let pct = used as f64 / st.total_page_file as f64 * 100.0;
        Some((pct, used, st.total_page_file))
    }

    pub fn mountpoints(&self) -> Vec<String> {
        let mut mounts = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            let mut wide: Vec<u16> = OsStr::new(&root).encode_wide().chain(std::iter::once(0)).collect();
            let mut free_avail = 0u64;
            let mut total = 0u64;
            let mut total_free = 0u64;
            if unsafe {
                GetDiskFreeSpaceExW(
                    wide.as_mut_ptr(),
                    &mut free_avail,
                    &mut total,
                    &mut total_free,
                )
            } != 0
            {
                mounts.push(format!("{}:", letter as char));
            }
        }
        mounts
    }

    pub fn disk_usage(&self, mount: &str) -> Option<(f64, u64, u64)> {
        let path = if mount.ends_with('\\') {
            mount.to_string()
        } else {
            format!("{mount}\\")
        };
        let mut wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(std::iter::once(0)).collect();
        let mut free_avail = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        if unsafe {
            GetDiskFreeSpaceExW(
                wide.as_mut_ptr(),
                &mut free_avail,
                &mut total,
                &mut total_free,
            )
        } == 0
        {
            return None;
        }
        if total == 0 {
            return None;
        }
        let used = total.saturating_sub(free_avail);
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

fn query_per_cpu_performance() -> Option<Vec<(i64, i64)>> {
    let mut info: SystemInfo = unsafe { zeroed() };
    unsafe { GetSystemInfo(&mut info) };
    let n = info.num_processors as usize;
    let mut buf: Vec<SystemProcessorPerformanceInformation> =
        (0..n).map(|_| unsafe { zeroed() }).collect();
    let mut ret_len = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(
            PROCESSOR_PERF_INFO,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * size_of::<SystemProcessorPerformanceInformation>()) as u32,
            &mut ret_len,
        )
    };
    if status != 0 {
        return None;
    }
    let count = (ret_len as usize) / size_of::<SystemProcessorPerformanceInformation>();
    buf.truncate(count.min(n));
    Some(
        buf.iter()
            .map(|p| {
                let idle = p.idle_time;
                let total = p.kernel_time.saturating_add(p.user_time);
                (idle, total)
            })
            .collect(),
    )
}

fn read_net_bytes() -> Option<(u64, u64)> {
    let mut table: *mut MibIfTable2 = ptr::null_mut();
    if unsafe { GetIfTable2(&mut table) } != 0 {
        return None;
    }
    if table.is_null() {
        return None;
    }
    let num_entries = unsafe { (*table).num_entries } as usize;
    let row_size = size_of::<MibIfRow2>();
    let rows_base = unsafe { (table as *const u8).add(8) };
    let mut recv = 0u64;
    let mut sent = 0u64;
    for i in 0..num_entries {
        let row = unsafe { &*(rows_base.add(i * row_size) as *const MibIfRow2) };
        let alias_end = row.alias.iter().position(|&c| c == 0).unwrap_or(row.alias.len());
        let alias = String::from_utf16_lossy(&row.alias[..alias_end]);
        if !alias.to_lowercase().contains("loopback") {
            recv = recv.saturating_add(row.in_octets);
            sent = sent.saturating_add(row.out_octets);
        }
    }
    unsafe { FreeMibTable(table as *mut c_void) };
    Some((recv, sent))
}
