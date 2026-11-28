//! The desktop wallpaper: read the current one (so it can be put back) and set a
//! picture file with a style. Windows only for now; elsewhere it says so.

use std::io;
use std::path::{Path, PathBuf};

/// How the picture fills the screen (the same choices as Windows Settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallStyle {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

impl WallStyle {
    pub fn parse(s: &str) -> Option<WallStyle> {
        Some(match s {
            "fill" => WallStyle::Fill,
            "fit" => WallStyle::Fit,
            "stretch" => WallStyle::Stretch,
            "center" => WallStyle::Center,
            "tile" => WallStyle::Tile,
            "span" => WallStyle::Span,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            WallStyle::Fill => "fill",
            WallStyle::Fit => "fit",
            WallStyle::Stretch => "stretch",
            WallStyle::Center => "center",
            WallStyle::Tile => "tile",
            WallStyle::Span => "span",
        }
    }
    /// Registry values (WallpaperStyle, TileWallpaper) under HKCU\Control Panel\Desktop.
    pub fn registry(self) -> (&'static str, &'static str) {
        match self {
            WallStyle::Fill => ("10", "0"),
            WallStyle::Fit => ("6", "0"),
            WallStyle::Stretch => ("2", "0"),
            WallStyle::Center => ("0", "0"),
            WallStyle::Tile => ("0", "1"),
            WallStyle::Span => ("22", "0"),
        }
    }
    pub fn from_registry(style: &str, tile: &str) -> WallStyle {
        match (style.trim(), tile.trim()) {
            (_, "1") => WallStyle::Tile,
            ("10", _) => WallStyle::Fill,
            ("6", _) => WallStyle::Fit,
            ("2", _) => WallStyle::Stretch,
            ("22", _) => WallStyle::Span,
            _ => WallStyle::Center,
        }
    }
}

/// The current wallpaper file (empty path = a plain colour) and its style.
pub fn current_wallpaper() -> io::Result<(PathBuf, WallStyle)> {
    imp::current()
}

/// Make `file` (a JPEG/PNG/BMP on this PC) the desktop wallpaper.
pub fn set_wallpaper(file: &Path, style: WallStyle) -> io::Result<()> {
    imp::set(file, style)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegGetValueW,
        RegOpenKeyExW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
    };

    const KEY: &str = "Control Panel\\Desktop";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn reg_sz(value: &str) -> Option<String> {
        let (k, v) = (wide(KEY), wide(value));
        let mut buf = [0u16; 64];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: NUL-terminated names; buf/len describe a valid buffer.
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
    }

    fn set_reg_sz(value: &str, data: &str) -> io::Result<()> {
        let k = wide(KEY);
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: valid key name; h receives an open key that is closed below.
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_SET_VALUE | KEY_QUERY_VALUE, &mut h) };
        if r != 0 {
            return Err(io::Error::from_raw_os_error(r as i32));
        }
        let (v, d) = (wide(value), wide(data));
        // SAFETY: h is open; d is NUL-terminated UTF-16 and its byte length is passed.
        let r = unsafe { RegSetValueExW(h, v.as_ptr(), 0, REG_SZ, d.as_ptr().cast(), (d.len() * 2) as u32) };
        // SAFETY: h was opened above.
        unsafe { RegCloseKey(h) };
        if r != 0 { Err(io::Error::from_raw_os_error(r as i32)) } else { Ok(()) }
    }

    pub fn current() -> io::Result<(PathBuf, WallStyle)> {
        let mut buf = [0u16; 1024];
        // SAFETY: buf is a writable buffer of the given length (in characters).
        let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr().cast(), 0) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let path = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        let style = WallStyle::from_registry(
            &reg_sz("WallpaperStyle").unwrap_or_default(),
            &reg_sz("TileWallpaper").unwrap_or_default(),
        );
        Ok((path, style))
    }

    pub fn set(file: &Path, style: WallStyle) -> io::Result<()> {
        let (s, t) = style.registry();
        set_reg_sz("WallpaperStyle", s)?;
        set_reg_sz("TileWallpaper", t)?;
        let abs = if file.as_os_str().is_empty() { PathBuf::new() } else { std::path::absolute(file)? };
        let w = wide(&abs.to_string_lossy());
        // SAFETY: w is a NUL-terminated path that lives for the call.
        let ok = unsafe {
            SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, w.as_ptr() as *mut _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
        };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    fn not_yet() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "Setting the wallpaper is not available on this system yet.")
    }
    pub fn current() -> io::Result<(PathBuf, WallStyle)> {
        Err(not_yet())
    }
    pub fn set(_: &Path, _: WallStyle) -> io::Result<()> {
        Err(not_yet())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_round_trip() {
        for s in
            [WallStyle::Fill, WallStyle::Fit, WallStyle::Stretch, WallStyle::Center, WallStyle::Tile, WallStyle::Span]
        {
            let (a, b) = s.registry();
            assert_eq!(WallStyle::from_registry(a, b), s);
            assert_eq!(WallStyle::parse(s.name()), Some(s));
        }
        assert_eq!(WallStyle::parse("nope"), None);
    }

    /// Reads the current wallpaper only (never changes it in tests).
    #[cfg(windows)]
    #[test]
    fn reads_the_current_wallpaper() {
        assert!(current_wallpaper().is_ok());
    }
}

/// Run by hand only (`cargo test -p cs-platform -- --ignored wallpaper_same`): sets the
/// wallpaper to exactly what it is now (no visible change) and reads it back.
#[cfg(all(test, windows))]
#[test]
#[ignore]
fn wallpaper_same_again() {
    let (p, s) = current_wallpaper().unwrap();
    set_wallpaper(&p, s).unwrap();
    assert_eq!(current_wallpaper().unwrap(), (p, s));
}
