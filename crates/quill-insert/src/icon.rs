//! App icons: what Quill shows beside the apps text was dictated into.
//!
//! Read from the installed app (its bundle on macOS, its executable on
//! Windows), never from its windows or what it shows.

/// Edge length asked for: 64 points on macOS (128 pixels, for Retina) and
/// 64 pixels on Windows, plenty for icons shown at up to 32 points.
pub const SIZE: u32 = 64;

/// The icon of the app with this identifier ([`crate::FrontApp::id`]) as a
/// PNG, if it can be found. On Windows that needs the app to have been in
/// front since Quill started, when its location was seen.
pub fn png(app_id: &str) -> Option<Vec<u8>> {
    imp::png(app_id)
}

// AppKit is an Objective-C API: the two calls below that take raw pointers
// or untyped dictionaries are unsafe.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use objc2::AllocAnyThread;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    pub fn png(app_id: &str) -> Option<Vec<u8>> {
        let workspace = NSWorkspace::sharedWorkspace();
        let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(app_id))?;
        let path = url.path()?;
        let image = workspace.iconForFile(&path);
        let size = f64::from(super::SIZE);
        image.setSize(NSSize::new(size, size));
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size));
        // SAFETY: `rect` is a valid NSRect for the call to read and adjust;
        // no context or hints.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        let properties = NSDictionary::<NSString, AnyObject>::new();
        // SAFETY: an empty properties dictionary of the type AppKit expects.
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        Some(data.to_vec())
    }
}

// The shell and GDI are C/COM APIs: the calls below are FFI.
#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;

    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
    };
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };
    use windows::core::HSTRING;

    pub fn png(app_id: &str) -> Option<Vec<u8>> {
        let path = crate::app::path_of(app_id)?;
        // SAFETY: sets up COM on this thread for the shell calls; undone
        // below only if this call did it.
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let icon = icon(&path);
        if init.is_ok() {
            // SAFETY: balances the CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        icon
    }

    fn icon(path: &str) -> Option<Vec<u8>> {
        let edge = i32::try_from(super::SIZE).unwrap_or(64);
        // SAFETY: a path string and no bind context; COM is set up.
        let factory: IShellItemImageFactory =
            unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }.ok()?;
        // SAFETY: asks for the icon at our size (or bigger); we own the
        // bitmap it returns and delete it below.
        let bitmap = unsafe {
            factory.GetImage(
                SIZE { cx: edge, cy: edge },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
        }
        .ok()?;
        let pixels = read(bitmap);
        // SAFETY: the bitmap is ours and isn't used again.
        let _ = unsafe { DeleteObject(bitmap.into()) };
        let (width, height, rgba) = pixels?;
        encode(width, height, &rgba)
    }

    /// The bitmap's pixels as straight (not premultiplied) RGBA, top row first.
    fn read(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
        let mut info = BITMAP::default();
        // SAFETY: GDI writes one BITMAP describing `bitmap` into `info`.
        let written = unsafe {
            GetObjectW(
                bitmap.into(),
                i32::try_from(size_of::<BITMAP>()).ok()?,
                Some((&raw mut info).cast::<c_void>()),
            )
        };
        if written == 0 {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight.abs());
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).ok()?,
                biWidth: width,
                // Negative: rows top to bottom.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let (w, h) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
        let mut bgra = vec![0_u8; usize::try_from(w * h * 4).ok()?];
        // SAFETY: the screen's device context, released below.
        let dc = unsafe { GetDC(None) };
        // SAFETY: `bgra` holds `height` rows of `width` 32-bit pixels, as
        // `header` describes.
        let lines = unsafe {
            GetDIBits(
                dc,
                bitmap,
                0,
                h,
                Some(bgra.as_mut_ptr().cast::<c_void>()),
                &raw mut header,
                DIB_RGB_COLORS,
            )
        };
        // SAFETY: the device context from GetDC above.
        unsafe { ReleaseDC(None, dc) };
        if lines == 0 {
            return None;
        }
        Some((w, h, to_rgba(bgra)))
    }

    fn encode(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
        Some(out)
    }

    /// The shell's bitmaps are BGRA, usually with premultiplied alpha, and
    /// with no alpha at all for some old icons.
    fn to_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
        let (px, _) = pixels.as_chunks_mut::<4>();
        let has_alpha = px.iter().any(|p| p[3] != 0);
        // Premultiplied colour never exceeds its alpha.
        let premultiplied = has_alpha
            && px
                .iter()
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
        for p in px.iter_mut() {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            } else if premultiplied && p[3] > 0 && p[3] < 255 {
                let alpha = u32::from(p[3]);
                for c in &mut p[..3] {
                    *c = u8::try_from(u32::from(*c) * 255 / alpha).unwrap_or(255);
                }
            }
        }
        pixels
    }

    #[cfg(test)]
    mod tests {
        use super::to_rgba;

        #[test]
        fn shell_pixels_become_straight_rgba() {
            // Half-transparent red, premultiplied BGRA: (0, 0, 64, 128).
            assert_eq!(
                to_rgba(vec![0, 0, 64, 128, 0, 0, 0, 0]),
                [127, 0, 0, 128, 0, 0, 0, 0]
            );
            // Colour above its alpha: straight alpha already, left alone.
            assert_eq!(to_rgba(vec![10, 20, 200, 100]), [200, 20, 10, 100]);
            // No alpha anywhere: an old icon, made opaque.
            assert_eq!(to_rgba(vec![1, 2, 3, 0]), [3, 2, 1, 255]);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn png(_app_id: &str) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn finds_an_apps_icon_by_bundle_id() {
        // Finder is on every Mac.
        let png = super::png("com.apple.finder").expect("Finder's icon");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(super::png("com.example.not-installed").is_none());
    }
}
