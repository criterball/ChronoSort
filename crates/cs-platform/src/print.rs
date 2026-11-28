//! Printing finished pages with the standard Windows Print dialog (printer, copies, preferences), then GDI:
//! each page (an RGB picture of the whole sheet) is stretched onto the printer's physical page. Nothing is
//! started or downloaded. Not available on other systems yet (they get an error message).

/// One page: width, height and RGB bytes (row by row, no padding) of the whole sheet.
pub struct PagePixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgb: &'a [u8],
}

/// Show the Print dialog and print. `Ok(false)` = the user cancelled. Blocking (the dialog is modal).
#[cfg(windows)]
pub fn print_pages(pages: &[PagePixels<'_>], title: &str, owner: isize) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use windows_sys::Win32::UI::Controls::Dialogs::*;

    if pages.is_empty() {
        return Ok(false);
    }
    // SAFETY: PRINTDLGW is plain data; zeroed is its documented starting state (lStructSize set below).
    let mut pd: PRINTDLGW = unsafe { std::mem::zeroed() };
    pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
    pd.hwndOwner = owner as _;
    pd.Flags = PD_RETURNDC | PD_NOSELECTION | PD_NOPAGENUMS | PD_USEDEVMODECOPIESANDCOLLATE;
    pd.nCopies = 1;
    // SAFETY: `pd` is a valid, initialised PRINTDLGW that outlives the call.
    let ok = unsafe { PrintDlgW(&mut pd) };
    let free = |pd: &PRINTDLGW| {
        // SAFETY: the handles come from PrintDlgW (or are null, which GlobalFree / DeleteDC accept).
        unsafe {
            if !pd.hDevMode.is_null() {
                GlobalFree(pd.hDevMode);
            }
            if !pd.hDevNames.is_null() {
                GlobalFree(pd.hDevNames);
            }
            if !pd.hDC.is_null() {
                DeleteDC(pd.hDC);
            }
        }
    };
    if ok == 0 {
        // SAFETY: no arguments.
        let code = unsafe { CommDlgExtendedError() };
        free(&pd);
        return if code == 0 { Ok(false) } else { Err(format!("The Print dialog could not open (code {code}).")) };
    }
    let dc = pd.hDC;
    let result = (|| -> Result<(), String> {
        // SAFETY: `dc` is the printer DC PrintDlgW returned; GetDeviceCaps only reads it.
        let (phys_w, phys_h, off_x, off_y) = unsafe {
            (
                GetDeviceCaps(dc, PHYSICALWIDTH as i32),
                GetDeviceCaps(dc, PHYSICALHEIGHT as i32),
                GetDeviceCaps(dc, PHYSICALOFFSETX as i32),
                GetDeviceCaps(dc, PHYSICALOFFSETY as i32),
            )
        };
        if phys_w <= 0 || phys_h <= 0 {
            return Err("The printer did not report its paper size.".into());
        }
        let wtitle: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: DOCINFOW is plain data; lpszDocName points at a live NUL-terminated buffer.
        let mut di: DOCINFOW = unsafe { std::mem::zeroed() };
        di.cbSize = std::mem::size_of::<DOCINFOW>() as i32;
        di.lpszDocName = wtitle.as_ptr();
        // SAFETY: valid DC and DOCINFOW.
        if unsafe { StartDocW(dc, &di) } <= 0 {
            return Err("The printer did not start the job.".into());
        }
        for p in pages {
            // Turn a landscape sheet when the paper is portrait (and the other way round).
            let turn = (p.width > p.height) != (phys_w > phys_h);
            let (w, h, bgr) = dib_rows(p, turn);
            // SAFETY: plain data, fully initialised below.
            let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32); // top-down rows
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 24;
            bmi.bmiHeader.biCompression = BI_RGB;
            // SAFETY: `dc` is valid; the bitmap buffer matches the header (24-bit rows padded to 4 bytes).
            let drawn = unsafe {
                if StartPage(dc) <= 0 {
                    0
                } else {
                    SetStretchBltMode(dc, HALFTONE as _);
                    SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
                    let r = StretchDIBits(
                        dc,
                        -off_x,
                        -off_y,
                        phys_w,
                        phys_h,
                        0,
                        0,
                        w as i32,
                        h as i32,
                        bgr.as_ptr().cast(),
                        &bmi,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                    EndPage(dc);
                    r
                }
            };
            if drawn == 0 {
                // SAFETY: valid DC with a started document.
                unsafe { AbortDoc(dc) };
                return Err("The printer did not accept a page.".into());
            }
        }
        // SAFETY: valid DC with a started document.
        unsafe { EndDoc(dc) };
        Ok(())
    })();
    free(&pd);
    result.map(|()| true)
}

#[cfg(not(windows))]
pub fn print_pages(_pages: &[PagePixels<'_>], _title: &str, _owner: isize) -> Result<bool, String> {
    Err("Printing is not available on this system yet. Use “Save as PDF” and print the PDF.".into())
}

/// The page as 24-bit BGR rows padded to 4 bytes (what a DIB wants), turned 90° clockwise when asked.
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_rows(p: &PagePixels<'_>, turn: bool) -> (u32, u32, Vec<u8>) {
    let (sw, sh) = (p.width as usize, p.height as usize);
    let (w, h) = if turn { (sh, sw) } else { (sw, sh) };
    let stride = (w * 3).div_ceil(4) * 4;
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            // Turned clockwise: the new (x, y) comes from the source (y, sh - 1 - x).
            let (sx, sy) = if turn { (y, sh - 1 - x) } else { (x, y) };
            let s = (sy * sw + sx) * 3;
            let d = y * stride + x * 3;
            out[d] = p.rgb[s + 2];
            out[d + 1] = p.rgb[s + 1];
            out[d + 2] = p.rgb[s];
        }
    }
    (w as u32, h as u32, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_bgr_padded_and_turned_clockwise() {
        // 2 × 1: red, green.
        let rgb = [255, 0, 0, 0, 255, 0];
        let p = PagePixels { width: 2, height: 1, rgb: &rgb };
        let (w, h, b) = dib_rows(&p, false);
        assert_eq!((w, h, b.len()), (2, 1, 8));
        assert_eq!(&b[..6], &[0, 0, 255, 0, 255, 0]);
        // Turned: 1 × 2, red on top (the left pixel goes to the top).
        let (w, h, b) = dib_rows(&p, true);
        assert_eq!((w, h), (1, 2));
        assert_eq!(&b[0..3], &[0, 0, 255]);
        assert_eq!(&b[4..7], &[0, 255, 0]);
    }
}
