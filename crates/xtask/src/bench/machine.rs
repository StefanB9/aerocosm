//! The machine a benchmark runs on (plan M5 D141): its CPU, logical cores
//! and memory, from `/proc` on Linux and from the CPU and the system on
//! Windows.

use super::record::Machine;

/// This machine as the system describes it; what it does not report is
/// unknown.
pub(crate) fn this_machine(os: &str) -> Machine {
    #[cfg(windows)]
    {
        Machine {
            cpu: cpu_name().unwrap_or_else(|| "unknown".to_owned()),
            cores: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
            memory_mib: total_memory().map_or(0, |bytes| bytes / (1024 * 1024)),
            os: os.to_owned(),
        }
    }
    #[cfg(not(windows))]
    {
        let read = |path| std::fs::read_to_string(path).unwrap_or_default();
        Machine::from_proc(&read("/proc/cpuinfo"), &read("/proc/meminfo"), os)
    }
}

/// The CPU's name as CPUID leaves `0x8000_0002` to `0x8000_0004` give it (the
/// "model name" of Linux's `/proc/cpuinfo`).
#[cfg(windows)]
fn cpu_name() -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;

        if __cpuid(0x8000_0000).eax < 0x8000_0004 {
            return None;
        }
        let registers = [0x8000_0002, 0x8000_0003, 0x8000_0004].map(|leaf| {
            let r = __cpuid(leaf);
            [r.eax, r.ebx, r.ecx, r.edx]
        });
        brand_string(&registers)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

/// The text of the CPUID brand-string registers up to the first NUL,
/// without surrounding spaces; `None` if empty.
#[cfg(any(windows, test))]
fn brand_string(registers: &[[u32; 4]; 3]) -> Option<String> {
    let mut bytes = Vec::with_capacity(48);
    bytes.extend(registers.iter().flatten().flat_map(|r| r.to_le_bytes()));
    let text = bytes.split(|&b| b == 0).next().unwrap_or_default();
    let name = String::from_utf8_lossy(text).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// The physical memory installed, bytes.
#[cfg(windows)]
#[allow(unsafe_code)]
fn total_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(size_of::<MEMORYSTATUSEX>()).ok()?,
        ..MEMORYSTATUSEX::default()
    };
    // SAFETY: `status` is a live, writable `MEMORYSTATUSEX` whose `dwLength`
    // is its size, as the call requires; it writes only within it.
    let done = unsafe { GlobalMemoryStatusEx(&raw mut status) };
    (done != 0).then_some(status.ullTotalPhys)
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};

    use super::*;

    /// CPUID leaves `0x8000_0002..=0x8000_0004` as `text`, padded with NULs.
    fn registers(text: &str) -> Result<[[u32; 4]; 3]> {
        let mut bytes = [0_u8; 48];
        let Some(head) = bytes.get_mut(..text.len()) else {
            bail!("{text} is longer than 48 bytes");
        };
        head.copy_from_slice(text.as_bytes());
        let mut registers = [[0_u32; 4]; 3];
        let (chunks, _) = bytes.as_chunks::<4>();
        for (register, chunk) in registers.iter_mut().flatten().zip(chunks) {
            *register = u32::from_le_bytes(*chunk);
        }
        Ok(registers)
    }

    #[test]
    fn test_brand_string_trims_padding() -> Result<()> {
        assert_eq!(
            brand_string(&registers("  AMD Ryzen 9 7950X 16-Core Processor  ")?),
            Some("AMD Ryzen 9 7950X 16-Core Processor".to_owned())
        );
        assert_eq!(brand_string(&registers("")?), None);
        assert_eq!(brand_string(&registers("   ")?), None);
        Ok(())
    }

    #[test]
    fn test_this_machine_reports_cores_and_memory() {
        if cfg!(any(target_os = "linux", windows)) {
            let machine = this_machine("test");
            assert!(machine.cores > 0 && machine.memory_mib > 0, "{machine:?}");
            assert_ne!(machine.cpu, "unknown");
            assert_eq!(machine.os, "test");
        }
    }
}
