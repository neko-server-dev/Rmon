pub mod common;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Monitor;
#[cfg(target_os = "windows")]
pub use windows::Monitor;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub struct Monitor;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl Monitor {
    pub fn new() -> Self {
        Self
    }
    pub fn cpu_percents(&mut self) -> Vec<f64> {
        vec![]
    }
    pub fn memory(&self) -> Option<(f64, u64, u64)> {
        None
    }
    pub fn swap(&self) -> Option<(f64, u64, u64)> {
        None
    }
    pub fn mountpoints(&self) -> Vec<String> {
        vec![]
    }
    pub fn disk_usage(&self, _mount: &str) -> Option<(f64, u64, u64)> {
        None
    }
    pub fn sample_network(&mut self) {}
    pub fn network(&self) -> (&[f64], &[f64], f64, f64, u64, u64) {
        (&[], &[], 0.0, 0.0, 0, 0)
    }
}
