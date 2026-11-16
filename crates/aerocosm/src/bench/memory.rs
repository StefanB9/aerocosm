//! The process's memory, from `/proc/self/status` on Linux (plan M5 D138).

/// Peak and current resident memory, MiB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Memory {
    pub(crate) peak: f64,
    pub(crate) resident: f64,
}

/// Memory as `/proc/self/status` text reports it (`VmHWM`, `VmRSS`).
pub(crate) fn parse_status(text: &str) -> Option<Memory> {
    let field = |key: &str| -> Option<f64> {
        let line = text.lines().find_map(|line| line.strip_prefix(key))?;
        let kib: u32 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
        Some(f64::from(kib) / 1024.0)
    };
    Some(Memory {
        peak: field("VmHWM:")?,
        resident: field("VmRSS:")?,
    })
}

/// The process's memory now; `None` where the system does not report it.
pub(crate) fn current() -> Option<Memory> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .as_deref()
        .and_then(parse_status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status_reads_peak_and_resident() {
        let text = "Name:\taerocosm\nVmPeak:\t 9000 kB\nVmHWM:\t  2048 kB\nVmRSS:\t  1024 kB\n";
        assert_eq!(
            parse_status(text),
            Some(Memory {
                peak: 2.0,
                resident: 1.0
            })
        );
        assert_eq!(parse_status("VmRSS:\t1024 kB\n"), None);
        assert_eq!(parse_status("VmHWM:\tlots kB\nVmRSS:\t1 kB\n"), None);
    }

    #[test]
    fn test_current_memory_on_linux() {
        if cfg!(target_os = "linux") {
            assert!(current().is_some_and(|m| m.peak >= m.resident && m.resident > 0.0));
        }
    }
}
