//! The process's memory (plan M5 D138): from `/proc/self/status` on Linux,
//! from the process's memory counters on Windows.

/// Peak and current resident memory, MiB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Memory {
    pub(crate) peak: f64,
    pub(crate) resident: f64,
}

/// Memory as `/proc/self/status` text reports it (`VmHWM`, `VmRSS`).
#[cfg(any(not(windows), test))]
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

/// Memory from byte counts, as Windows reports it.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn from_bytes(peak: usize, resident: usize) -> Memory {
    const MIB: f64 = 1024.0 * 1024.0;
    Memory {
        peak: peak as f64 / MIB,
        resident: resident as f64 / MIB,
    }
}

/// The process's memory now; `None` where the system does not report it.
#[cfg(not(windows))]
pub(crate) fn current() -> Option<Memory> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .as_deref()
        .and_then(parse_status)
}

/// The process's memory now: its peak and current working set.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn current() -> Option<Memory> {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };

    let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that is always
    // valid and needs no closing; `counters` is a live, writable
    // `PROCESS_MEMORY_COUNTERS` and `size` is its size in bytes, so the call
    // writes only within it.
    let done = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) };
    (done != 0).then(|| from_bytes(counters.PeakWorkingSetSize, counters.WorkingSetSize))
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
    fn test_from_bytes_converts_to_mib() {
        assert_eq!(
            from_bytes(3 * 1024 * 1024, 512 * 1024),
            Memory {
                peak: 3.0,
                resident: 0.5
            }
        );
    }

    #[test]
    fn test_current_memory_on_linux_and_windows() {
        if cfg!(any(target_os = "linux", windows)) {
            assert!(current().is_some_and(|m| m.peak >= m.resident && m.resident > 0.0));
        }
    }
}
