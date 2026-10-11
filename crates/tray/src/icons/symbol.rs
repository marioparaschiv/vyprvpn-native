//! The menu bar shows Safari's compass from SF Symbols. Apple licenses the symbols for use on its
//! platforms only, so AppKit draws them at runtime rather than the app shipping them.

#![allow(unsafe_code)]

use std::{ptr, slice};

use objc2::AllocAnyThread;
use objc2_app_kit::{NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tray_icon::Icon;

use super::{IconError, IconKind};

/// tray-icon shows the icon at most 22 pt tall, drawn here at 2x for Retina screens.
const CANVAS_HEIGHT: f64 = 22.;
const SCALE: f64 = 2.;
/// About the height of the system's own menu bar symbols.
const SYMBOL_HEIGHT: f64 = 16.;

/// The compass outlined while disconnected or busy, filled once connected. tray-icon draws it as a
/// template, so it follows the menu bar's colour.
pub fn icon(kind: IconKind) -> Result<Icon, IconError> {
    let name = match kind {
        IconKind::Disconnected | IconKind::Busy => "safari",
        IconKind::Connected => "safari.fill",
    };
    let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        None,
    )
    .ok_or(IconError::MissingSymbol(name))?;

    render(&symbol)
}

fn render(symbol: &NSImage) -> Result<Icon, IconError> {
    let size = symbol.size();
    let symbol_width = size.width * SYMBOL_HEIGHT / size.height;
    let width = (symbol_width * SCALE).ceil() as usize;
    let height = (CANVAS_HEIGHT * SCALE) as usize;

    // SAFETY: null planes make AppKit allocate and own the pixel buffer.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }
    .ok_or(IconError::NoBitmap)?;
    // Sized in points, so drawing below is in points at `SCALE` pixels each.
    bitmap.setSize(NSSize::new(width as f64 / SCALE, CANVAS_HEIGHT));
    let context =
        NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap).ok_or(IconError::NoBitmap)?;

    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    symbol.drawInRect(NSRect::new(
        NSPoint::new(0., (CANVAS_HEIGHT - SYMBOL_HEIGHT) / 2.),
        NSSize::new(symbol_width, SYMBOL_HEIGHT),
    ));
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();

    let row = bitmap.bytesPerRow() as usize;
    // SAFETY: the bitmap owns `row * height` bytes and outlives this borrow.
    let pixels = unsafe { slice::from_raw_parts(bitmap.bitmapData(), row * height) };
    // A template only reads alpha, and AppKit's bitmap is premultiplied, so keep alpha alone.
    let rgba = pixels
        .chunks(row)
        .flat_map(|line| {
            line[..width * 4]
                .chunks(4)
                .flat_map(|pixel| [0, 0, 0, pixel[3]])
        })
        .collect();

    Ok(Icon::from_rgba(rgba, width as u32, height as u32)?)
}
