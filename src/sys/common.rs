pub const UPDATE_INTERVAL_MS: u64 = 1000;
pub const NET_HISTORY_SIZE: usize = 300;

pub fn human_bytes(b: u64) -> String {
    const UNIT: f64 = 1024.0;
    if b < UNIT as u64 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut i: isize = -1;
    while v >= UNIT && i < (units.len() - 1) as isize {
        v /= UNIT;
        i += 1;
    }
    format!("{v:.2} {}", units[i as usize])
}

pub fn clamp_percent(p: f64) -> u8 {
    if p < 0.0 {
        0
    } else if p > 100.0 {
        100
    } else {
        (p + 0.5) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn human_bytes_64gib_total() {
        let sixty_four_gib = 64 * 1024 * 1024 * 1024;
        assert_eq!(human_bytes(sixty_four_gib), "64.00 GiB");
    }
}

pub fn push_bounded(buf: &mut Vec<f64>, v: f64, max: usize) {
    buf.push(v);
    if buf.len() > max {
        let drop = buf.len() - max;
        buf.drain(0..drop);
    }
}
