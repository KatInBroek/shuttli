//! Portable, bounded content validation and canonical digest for every platform.
use sha2::{Digest, Sha256};
use shuttli_model::sync::Format;
use std::io::Cursor;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: usize = 4 * 1024 * 1024;

pub fn canonical_digest(format: Format, data: &[u8]) -> Result<[u8; 32], &'static str> {
    if data.is_empty() || data.len() > MAX_BYTES {
        return Err("invalid content size");
    }
    let mut hash = Sha256::new();
    match format {
        Format::Text => {
            if data.len() > 1024 * 1024 {
                return Err("text exceeds 1 MiB");
            }
            let text = std::str::from_utf8(data).map_err(|_| "invalid UTF-8")?;
            if text.contains('\0') {
                return Err("NUL in text");
            }
            hash.update(b"text.v1\0");
            hash.update(text.replace("\r\n", "\n").as_bytes());
        }
        Format::Png => {
            let (w, h, pixels) = rgba(data)?;
            hash.update(b"png.rgba8.v1\0");
            hash.update(w.to_be_bytes());
            hash.update(h.to_be_bytes());
            hash.update(pixels);
        }
    }
    Ok(hash.finalize().into())
}

pub fn rgba(data: &[u8]) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(data),
        png::Limits {
            bytes: MAX_PIXELS * 8,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "invalid PNG header")?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_PIXELS as u64 || w > 16384 || h > 16384 {
        return Err("PNG dimensions exceed limit");
    }
    if info.animation_control.is_some() {
        return Err("animated PNG unsupported");
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut raw)
        .map_err(|_| "invalid PNG data")?;
    reader.finish().map_err(|_| "invalid PNG trailer")?;
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    let channels = info.color_type.samples();
    for p in raw[..info.buffer_size()].chunks_exact(channels) {
        match info.color_type {
            png::ColorType::Rgba => out.extend_from_slice(p),
            png::ColorType::Rgb => out.extend_from_slice(&[p[0], p[1], p[2], 255]),
            png::ColorType::Grayscale => out.extend_from_slice(&[p[0], p[0], p[0], 255]),
            png::ColorType::GrayscaleAlpha => out.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
            _ => return Err("unsupported PNG representation"),
        }
    }
    Ok((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_normalization_and_png_bounds() {
        assert_eq!(
            canonical_digest(Format::Text, b"a\r\nb").unwrap(),
            canonical_digest(Format::Text, b"a\nb").unwrap()
        );
        assert!(canonical_digest(Format::Text, b"x\0").is_err());
        let mut bytes = Vec::new();
        let encoder = png::Encoder::new(&mut bytes, 100000, 100000);
        let _ = encoder.write_header().unwrap();
        assert!(canonical_digest(Format::Png, &bytes).is_err());
    }
}
