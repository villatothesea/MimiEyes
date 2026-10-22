//! MimiEyes (VENDORED.md #10): the window and taskbar icon wear the user's
//! accent colour. The "Just Eyes" logo (`assets/brand/mimieyes-icon.svg`: a
//! rounded tile, four eyes, their highlights) is plain geometry, so it is
//! drawn here pixel by pixel (4×4 supersampled), encoded as PNG and handed
//! to Windows as the window's big and small icons whenever the accent
//! changes. Pinned shortcuts and Explorer keep the exe's built-in green.

/// The logo's eye colour; its tile and highlights wear the accent.
const INK: [u8; 3] = [0x11, 0x1a, 0x14];
/// The logo's 512 canvas: the tile's corner, eyes (x, y, r), highlights.
const CORNER: f32 = 115.0;
const EYES: [(f32, f32, f32); 4] = [
    (66.0, 236.0, 22.0),
    (446.0, 236.0, 22.0),
    (176.0, 270.0, 80.0),
    (336.0, 270.0, 80.0),
];
const SHINES: [(f32, f32, f32); 4] = [
    (150.0, 244.0, 20.0),
    (310.0, 244.0, 20.0),
    (196.0, 292.0, 8.0),
    (356.0, 292.0, 8.0),
];

fn in_tile(x: f32, y: f32) -> bool {
    let (cx, cy) = (x.clamp(CORNER, 512.0 - CORNER), y.clamp(CORNER, 512.0 - CORNER));
    (0.0..=512.0).contains(&x)
        && (0.0..=512.0).contains(&y)
        && (x - cx).powi(2) + (y - cy).powi(2) <= CORNER * CORNER
}

fn in_any(set: &[(f32, f32, f32)], x: f32, y: f32) -> bool {
    set.iter()
        .any(|(cx, cy, r)| (x - cx).powi(2) + (y - cy).powi(2) <= r * r)
}

/// The logo, `size` px square, as straight RGBA.
pub fn rgba(size: u32, accent: [u8; 3]) -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let unit = 512.0 / (size * SS) as f32;
    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut hits) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = ((px * SS + sx) as f32 + 0.5) * unit;
                    let y = ((py * SS + sy) as f32 + 0.5) * unit;
                    if !in_tile(x, y) {
                        continue;
                    }
                    let c = if in_any(&EYES, x, y) && !in_any(&SHINES, x, y) {
                        INK
                    } else {
                        accent
                    };
                    r += c[0] as u32;
                    g += c[1] as u32;
                    b += c[2] as u32;
                    hits += 1;
                }
            }
            if hits == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[
                    (r / hits) as u8,
                    (g / hits) as u8,
                    (b / hits) as u8,
                    (hits * 255 / (SS * SS)) as u8,
                ]);
            }
        }
    }
    out
}

/// The logo as PNG bytes.
pub fn png(size: u32, accent: [u8; 3]) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(size, size, rgba(size, accent))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}

fn rgb_of(accent: gpui::Hsla) -> [u8; 3] {
    let rgba = gpui::Rgba::from(accent);
    [rgba.r, rgba.g, rgba.b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Put the accent-coloured logo on the window (and so its taskbar button);
/// a no-op while the accent is the one already applied. Call per frame.
pub fn sync(window: &gpui::Window, accent: gpui::Hsla) {
    // Test windows have no native handle (asking for one panics).
    #[cfg(all(windows, not(test)))]
    windows::sync(window, rgb_of(accent));
    #[cfg(not(all(windows, not(test))))]
    let _ = (window, rgb_of(accent));
}

#[cfg(all(windows, not(test)))]
mod windows {
    use std::cell::Cell;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, DestroyIcon, HICON, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR,
        SendMessageW, WM_SETICON,
    };

    thread_local! {
        /// (window, accent) last applied, and the icons it made.
        static APPLIED: Cell<Option<(isize, [u8; 3])>> = const { Cell::new(None) };
        static ICONS: Cell<(HICON, HICON)> = const { Cell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
    }

    fn icon(size: u32, accent: [u8; 3]) -> HICON {
        let Some(png) = super::png(size, accent) else {
            return std::ptr::null_mut();
        };
        // SAFETY: `png` is a complete PNG icon image (Vista+ accepts PNG
        // resources); the returned handle is ours to destroy.
        unsafe {
            CreateIconFromResourceEx(
                png.as_ptr(),
                png.len() as u32,
                1,
                0x0003_0000,
                size as i32,
                size as i32,
                LR_DEFAULTCOLOR,
            )
        }
    }

    pub(super) fn sync(window: &gpui::Window, accent: [u8; 3]) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get();
        if APPLIED.with(|a| a.get()) == Some((hwnd, accent)) {
            return;
        }
        APPLIED.with(|a| a.set(Some((hwnd, accent))));
        let (big, small) = (icon(64, accent), icon(32, accent));
        if big.is_null() || small.is_null() {
            return;
        }
        // SAFETY: a live top-level window of this thread; the icons stay
        // alive until replaced (the old ones are destroyed after the swap).
        unsafe {
            SendMessageW(hwnd as HWND, WM_SETICON, ICON_BIG as usize, big as isize);
            SendMessageW(hwnd as HWND, WM_SETICON, ICON_SMALL as usize, small as isize);
        }
        let (old_big, old_small) = ICONS.with(|i| i.replace((big, small)));
        for old in [old_big, old_small] {
            if !old.is_null() {
                // SAFETY: made by `icon`, no longer set on any window.
                unsafe { DestroyIcon(old) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_wears_the_accent_with_ink_eyes_and_clear_corners() {
        let accent = [0x20, 0x80, 0xf0];
        let px = rgba(64, accent);
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        // A corner is outside the rounded tile.
        assert_eq!(at(0, 0)[3], 0);
        // The tile's top middle is the accent, opaque.
        assert_eq!(at(32, 4), &[0x20, 0x80, 0xf0, 0xff]);
        // A big eye's middle (512: 176,300 → 64: 22,37) is ink.
        assert_eq!(&at(22, 37)[..3], &INK);
        assert!(png(32, accent).is_some_and(|png| png.starts_with(b"\x89PNG")));
    }
}
