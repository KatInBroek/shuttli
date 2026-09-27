//! Canonical comparison and bounded PNG decoding, shared by OS and wire input.
use sha2::{Digest, Sha256};
use shuttli_model::sync::{Format, Metadata};
use shuttli_ports::sync::{Payload, Result};
use std::{io::Cursor, sync::Arc};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: usize = 4 * 1024 * 1024;
pub fn payload(format: Format, data: Vec<u8>) -> Result<Payload> {
    if data.len() > MAX_BYTES {
        return Err("content exceeds 8 MiB".into());
    }
    let mut hash = Sha256::new();
    match format {
        Format::Text => {
            if data.len() > 1024 * 1024 {
                return Err("text exceeds 1 MiB".into());
            }
            let text = std::str::from_utf8(&data).map_err(|_| "invalid UTF-8")?;
            if text.contains('\0') {
                return Err("NUL in text".into());
            }
            hash.update(b"text.v1\0");
            hash.update(text.replace("\r\n", "\n").as_bytes());
        }
        Format::Png => {
            let (w, h, pixels) = rgba(&data)?;
            hash.update(b"png.rgba8.v1\0");
            hash.update(w.to_be_bytes());
            hash.update(h.to_be_bytes());
            hash.update(pixels);
        }
    }
    let meta = Metadata {
        format,
        size: data.len() as u64,
        digest: hash.finalize().into(),
    };
    Ok(Payload {
        meta,
        data: Arc::from(data),
    })
}
pub fn rgba(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
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
        return Err("PNG dimensions exceed limit".into());
    }
    if info.animation_control.is_some() {
        return Err("animated PNG unsupported".into());
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
            _ => return Err("unsupported PNG representation".into()),
        }
    }
    Ok((w, h, out))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_comparison_preserves_wire_and_rejects_nul() {
        let a = payload(Format::Text, b"a\r\nb\rc".to_vec()).unwrap();
        let b = payload(Format::Text, b"a\nb\rc".to_vec()).unwrap();
        assert_eq!(a.meta.digest, b.meta.digest);
        assert_eq!(&*a.data, b"a\r\nb\rc");
        assert!(payload(Format::Text, b"x\0".to_vec()).is_err());
    }
    #[test]
    fn bomb_header_rejected_before_allocation() {
        let mut b = Vec::new();
        {
            let e = png::Encoder::new(&mut b, 100000, 100000);
            let _ = e.write_header().unwrap();
        }
        assert!(payload(Format::Png, b).is_err());
    }
}
