//! Uygulama adı ve ikonu, kabuk öğesi üzerinden (bölüm 5.3).
//!
//! Tek yol: `SHCreateItemFromParsingName` → AUMID için `shell:AppsFolder\<AUMID>`,
//! yoksa exe yolu → `IShellItemImageFactory::GetImage` → PNG → base64 data URL.
//! Çağıran iş parçacığında COM başlatılmış olmalı.

use std::io::Cursor;

use base64::Engine;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::Storage::EnhancedStorage::PKEY_FileDescription;
use windows::Win32::UI::Shell::{
    IShellItem, IShellItem2, IShellItemImageFactory, SHCreateItemFromParsingName,
    SIGDN_NORMALDISPLAY, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, HICON, ICONINFO};
use windows::core::{Interface, PCWSTR};

use crate::win::{take_pwstr, wide};

/// İkon kenar uzunluğu (fiziksel piksel). Büyütülmüş ikon 72 mantıksal px × ölçek.
const ICON_PX: i32 = 128;

pub struct ShellInfo {
    pub name: Option<String>,
    pub icon: Option<String>,
    /// AUMID kabukta (Başlat menüsü uygulamaları) kayıtlı mı.
    pub aumid_resolves: bool,
}

pub fn shell_info(aumid: Option<&str>, exe: Option<&str>) -> ShellInfo {
    let from_aumid = aumid.and_then(|a| parse(&format!("shell:AppsFolder\\{a}")));
    let aumid_resolves = from_aumid.is_some();
    let (item, is_exe) = match from_aumid {
        Some(item) => (Some(item), false),
        None => (exe.and_then(parse), true),
    };
    let Some(item) = item else {
        return ShellInfo {
            name: None,
            icon: None,
            aumid_resolves,
        };
    };
    let mut icon = icon_image(&item);
    // Başlat kutucuğu logosu: bazı uygulamalar (Firefox) AUMID'lerine renkli
    // zemine konacak beyaz bir silüet tanımlar (VisualElementsManifest); kabuk
    // onu verir ve dock'ta renksiz görünür. Exe'nin kendi ikonu renkliyse o.
    if !is_exe
        && icon.as_ref().is_some_and(is_white_glyph)
        && let Some(own) = exe.and_then(parse).as_ref().and_then(icon_image)
        && !is_white_glyph(&own)
    {
        icon = Some(own);
    }
    ShellInfo {
        name: display_name(&item, is_exe),
        icon: icon.as_ref().and_then(png_data_url),
        aumid_resolves,
    }
}

/// Opak piksellerinin neredeyse hepsi beyaz / açık gri (renksiz silüet).
fn is_white_glyph(image: &image::RgbaImage) -> bool {
    let (mut opaque, mut light) = (0u32, 0u32);
    for p in image.pixels() {
        if p[3] > 128 {
            opaque += 1;
            if p[0].min(p[1]).min(p[2]) > 200 {
                light += 1;
            }
        }
    }
    opaque > 0 && light * 100 >= opaque * 95
}

fn parse(path: &str) -> Option<IShellItem> {
    let w = wide(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok() }
}

fn display_name(item: &IShellItem, is_exe: bool) -> Option<String> {
    unsafe {
        if is_exe {
            // Exe için "chrome.exe" yerine sürüm bilgisindeki açıklama ("Google Chrome").
            if let Ok(item2) = item.cast::<IShellItem2>()
                && let Ok(p) = item2.GetString(&PKEY_FileDescription)
            {
                let s = take_pwstr(p);
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
        let s = take_pwstr(item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?);
        let s = if is_exe {
            s.strip_suffix(".exe").unwrap_or(&s).to_string()
        } else {
            s
        };
        (!s.is_empty()).then_some(s)
    }
}

fn icon_image(item: &IShellItem) -> Option<image::RgbaImage> {
    unsafe {
        let factory = item.cast::<IShellItemImageFactory>().ok()?;
        let bitmap = factory
            .GetImage(
                SIZE {
                    cx: ICON_PX,
                    cy: ICON_PX,
                },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let rgba = bitmap_to_rgba(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let (w, h, pixels) = rgba?;
        let mut image = image::RgbaImage::from_raw(w, h, pixels)?;
        // SIIGBF_BIGGERSIZEOK daha büyüğünü (çoğu zaman 256) döndürebilir; en
        // büyük gösterim 72 px (40 × 1.8), 128 yeter ve bellekte 4 kat küçük (7.4).
        if w > ICON_PX as u32 || h > ICON_PX as u32 {
            image = image::imageops::resize(
                &image,
                ICON_PX as u32,
                ICON_PX as u32,
                image::imageops::FilterType::CatmullRom,
            );
        }
        Some(image)
    }
}

fn png_data_url(image: &image::RgbaImage) -> Option<String> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    Some(format!("data:image/png;base64,{b64}"))
}

/// Başka bir sürecin ikonu (tepsi, 8.6) → PNG data URL. Alfa kanalı olmayan eski
/// ikonlarda saydamlık maskeden alınır.
pub fn hicon_data_url(icon: HICON) -> Option<String> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let result = (|| {
            if info.hbmColor.is_invalid() {
                return monochrome_icon(info.hbmMask);
            }
            let (w, h, mut px) = bitmap_bgra(info.hbmColor)?;
            if px.as_chunks::<4>().0.iter().all(|p| p[3] == 0)
                && let Some((mw, _, mask)) = bitmap_bgra(info.hbmMask)
                && mw == w
                && mask.len() >= px.len()
            {
                for (p, m) in px
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(mask.as_chunks::<4>().0)
                {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            fix_alpha_and_swizzle(&mut px);
            png_data_url(&image::RgbaImage::from_raw(w, h, px)?)
        })();
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        result
    }
}

/// Renk bitmap'i olmayan (tek renkli) ikon: maske iki katı yüksekliktedir, üst
/// yarı AND, alt yarı XOR. AND=0 → ikonun kendi pikseli (XOR siyah / beyaz),
/// AND=1 → saydam (XOR=1 ters çevirme, saydam sayılır).
unsafe fn monochrome_icon(mask: HBITMAP) -> Option<String> {
    let (w, h2, px) = unsafe { bitmap_bgra(mask) }?;
    let h = h2 / 2;
    if h == 0 {
        return None;
    }
    let row = w as usize * 4;
    let mut out = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let and = px[y * row + x * 4] != 0;
            let xor = px[(y + h as usize) * row + x * 4] != 0;
            let c = if xor { 255 } else { 0 };
            out.extend_from_slice(&[c, c, c, if and { 0 } else { 255 }]);
        }
    }
    png_data_url(&image::RgbaImage::from_raw(w, h, out)?)
}

/// HBITMAP → üstten alta RGBA piksel dizisi.
unsafe fn bitmap_to_rgba(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, mut buf) = unsafe { bitmap_bgra(bitmap) }?;
    fix_alpha_and_swizzle(&mut buf);
    Some((w, h, buf))
}

/// HBITMAP → üstten alta BGRA (olduğu gibi).
unsafe fn bitmap_bgra(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut _),
        ) == 0
        {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negatif = üstten alta
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        Some((w as u32, h as u32, buf))
    }
}

/// BGRA → RGBA. Alfa kanalı boşsa (eski 24 bit ikonlar) opak yapar; önceden
/// çarpılmış (premultiplied) alfa ise düz alfaya çevirir.
fn fix_alpha_and_swizzle(buf: &mut [u8]) {
    let all_transparent = buf.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    let premultiplied = !all_transparent
        && buf
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    for p in buf.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        if all_transparent {
            p[3] = 255;
        } else if premultiplied && p[3] > 0 && p[3] < 255 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_white_glyph;
    use image::{Rgba, RgbaImage};

    #[test]
    fn detects_white_tile_logos() {
        // Saydam zemin üstünde beyaz silüet (Firefox'un kutucuk logosu gibi).
        let mut glyph = RgbaImage::new(8, 8);
        for x in 2..6 {
            glyph.put_pixel(x, 4, Rgba([250, 250, 250, 255]));
        }
        assert!(is_white_glyph(&glyph));
        // Siyah daire içinde beyaz X (X'in logosu): renksiz ama silüet değil.
        let mut x_logo = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]));
        x_logo.put_pixel(3, 3, Rgba([255, 255, 255, 255]));
        assert!(!is_white_glyph(&x_logo));
        // Renkli ikon.
        assert!(!is_white_glyph(&RgbaImage::from_pixel(
            8,
            8,
            Rgba([255, 120, 0, 255])
        )));
        // Tamamen saydam.
        assert!(!is_white_glyph(&RgbaImage::new(8, 8)));
    }
}
