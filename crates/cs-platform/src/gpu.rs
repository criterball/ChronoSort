//! Graphics cards and their memory (VRAM), used to suggest AI models that fit
//! this PC. Read-only and local: no driver SDKs, no network.
//!
//! * Windows: the display-adapter class key in the registry
//!   (`HardwareInformation.qwMemorySize`, falling back to `MemorySize`).
//! * Linux: AMD/Intel via sysfs `mem_info_vram_total`; NVIDIA via `nvidia-smi` if installed.
//! * macOS: nothing yet. Apple Silicon shares main memory with the GPU, so RAM decides.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gpu {
    pub name: String,
    /// Dedicated video memory in bytes, when known.
    pub vram_bytes: Option<u64>,
    /// Built into the processor (shares system memory; not useful for big models).
    pub integrated: bool,
}

/// All graphics adapters found, best (most dedicated memory) first.
pub fn gpus() -> Vec<Gpu> {
    let mut v = imp::gpus();
    v.retain(|g| !g.name.trim().is_empty() && !is_virtual(&g.name));
    v.sort_by(|a, b| b.useful_vram().cmp(&a.useful_vram()).then(a.name.cmp(&b.name)));
    v.dedup_by(|a, b| a.name == b.name && a.vram_bytes == b.vram_bytes);
    v
}

impl Gpu {
    /// VRAM that counts for AI models: 0 for integrated graphics.
    pub fn useful_vram(&self) -> u64 {
        if self.integrated { 0 } else { self.vram_bytes.unwrap_or(0) }
    }
}

/// Remote-desktop, Hyper-V and "basic display" adapters are not real GPUs.
fn is_virtual(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["basic display", "basic render", "remote display", "hyper-v", "virtual", "parsec", "displaylink", "citrix"]
        .iter()
        .any(|k| n.contains(k))
}

/// Integrated graphics by name. Intel Arc cards are discrete; Intel UHD/Iris/HD are not.
/// AMD APUs report as "Radeon Graphics" / "Radeon(TM) Graphics" / "Radeon Vega".
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) fn looks_integrated(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("intel") {
        return !n.contains("arc");
    }
    n.contains("radeon(tm) graphics")
        || n.ends_with("radeon graphics")
        || n.contains("radeon vega")
        || n.contains("adreno")
        || n.contains("mali")
}

#[cfg(windows)]
mod imp {
    use super::{Gpu, looks_integrated};
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_QWORD, RRF_RT_REG_SZ, RegGetValueW,
    };

    const CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Raw value bytes, or None if missing / wrong type.
    fn read(subkey: &str, value: &str, flags: u32) -> Option<Vec<u8>> {
        let (k, v) = (wide(subkey), wide(value));
        let mut len: u32 = 0;
        // SAFETY: valid NUL-terminated strings; null data pointer asks for the size only.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                k.as_ptr(),
                v.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc != 0 || len == 0 || len > 4096 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: buf has `len` writable bytes, and len reports its size.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                k.as_ptr(),
                v.as_ptr(),
                flags,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (rc == 0).then(|| {
            buf.truncate(len as usize);
            buf
        })
    }

    fn read_string(subkey: &str, value: &str) -> Option<String> {
        let b = read(subkey, value, RRF_RT_REG_SZ)?;
        let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&c| c != 0).collect();
        Some(String::from_utf16_lossy(&w))
    }

    fn read_number(subkey: &str, value: &str) -> Option<u64> {
        let b = read(subkey, value, RRF_RT_REG_QWORD | RRF_RT_REG_DWORD | RRF_RT_REG_BINARY)?;
        match b.len() {
            8 => Some(u64::from_le_bytes(b.try_into().ok()?)),
            4 => Some(u32::from_le_bytes(b.try_into().ok()?) as u64),
            _ => None,
        }
    }

    pub fn gpus() -> Vec<Gpu> {
        let mut out = Vec::new();
        // Adapter keys are 0000, 0001, …; there can be gaps, so try a fixed range.
        for i in 0..64 {
            let key = format!(r"{CLASS}\{i:04}");
            let Some(name) = read_string(&key, "DriverDesc") else { continue };
            // qwMemorySize is exact above 4 GB; MemorySize is a 32-bit fallback.
            let vram = read_number(&key, "HardwareInformation.qwMemorySize")
                .or_else(|| read_number(&key, "HardwareInformation.MemorySize"))
                .filter(|&b| b > 0);
            let integrated = looks_integrated(&name);
            out.push(Gpu { name, vram_bytes: vram, integrated });
        }
        out
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Gpu, looks_integrated};
    use std::fs;

    pub fn gpus() -> Vec<Gpu> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir("/sys/class/drm") {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.starts_with("card") || name.contains('-') {
                    continue;
                }
                let dev = e.path().join("device");
                let vendor = fs::read_to_string(dev.join("vendor")).unwrap_or_default();
                let vram = fs::read_to_string(dev.join("mem_info_vram_total")).ok().and_then(|s| s.trim().parse().ok());
                let label = match vendor.trim() {
                    "0x1002" => "AMD Radeon",
                    "0x8086" => "Intel graphics",
                    "0x10de" => continue, // reported by nvidia-smi below
                    _ => "Graphics adapter",
                };
                // Small VRAM on AMD means an APU's carve-out, not a real card.
                let integrated = vendor.trim() == "0x8086" || vram.is_none_or(|v: u64| v < (1 << 30));
                out.push(Gpu {
                    name: label.into(),
                    vram_bytes: vram,
                    integrated: integrated || looks_integrated(label),
                });
            }
        }
        // NVIDIA: only if the driver's tool is installed; short, local, read-only.
        if let Ok(o) = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
            .output()
            && o.status.success()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some((n, mib)) = line.rsplit_once(',') {
                    let vram = mib.trim().parse::<u64>().ok().map(|m| m << 20);
                    out.push(Gpu { name: n.trim().into(), vram_bytes: vram, integrated: false });
                }
            }
        }
        out
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn gpus() -> Vec<super::Gpu> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrated_by_name() {
        assert!(looks_integrated("Intel(R) UHD Graphics 750"));
        assert!(looks_integrated("Intel(R) Iris(R) Xe Graphics"));
        assert!(!looks_integrated("Intel(R) Arc(TM) A770 Graphics"));
        assert!(looks_integrated("AMD Radeon(TM) Graphics"));
        assert!(!looks_integrated("AMD Radeon RX 7800 XT"));
        assert!(!looks_integrated("NVIDIA GeForce RTX 3060"));
    }

    #[test]
    fn virtual_adapters_are_dropped() {
        assert!(is_virtual("Microsoft Basic Display Adapter"));
        assert!(is_virtual("Microsoft Remote Display Adapter"));
        assert!(!is_virtual("NVIDIA GeForce RTX 3060"));
    }

    #[test]
    fn useful_vram_ignores_integrated() {
        let igpu = Gpu { name: "Intel(R) UHD Graphics".into(), vram_bytes: Some(2 << 30), integrated: true };
        let dgpu = Gpu { name: "NVIDIA".into(), vram_bytes: Some(12 << 30), integrated: false };
        assert_eq!(igpu.useful_vram(), 0);
        assert_eq!(dgpu.useful_vram(), 12 << 30);
    }

    /// Smoke test on the real machine: never panics, names are non-empty.
    #[test]
    fn detect_does_not_panic() {
        for g in gpus() {
            assert!(!g.name.is_empty());
        }
    }
}
