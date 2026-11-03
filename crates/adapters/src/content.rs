//! Desktop payload adapter over the platform-neutral content contract.
use shuttli_model::sync::{Format, Metadata};
use shuttli_ports::sync::{Payload, Result};
use std::sync::Arc;

pub use shuttli_content::MAX_BYTES;

pub fn payload(format: Format, data: Vec<u8>) -> Result<Payload> {
    let digest = shuttli_content::canonical_digest(format, &data).map_err(str::to_owned)?;
    Ok(Payload {
        meta: Metadata {
            format,
            size: data.len() as u64,
            digest,
        },
        data: Arc::from(data),
    })
}

/// An OS clipboard may validly contain an empty string. It has no transferable
/// body, but still supplies a healthy revision/baseline for incoming writes.
/// Keep wire payload validation strict, including empty/invalid PNG rejection.
pub fn capture_payload(format: Format, data: Vec<u8>) -> Result<Option<Payload>> {
    if format == Format::Text && data.is_empty() {
        Ok(None)
    } else {
        payload(format, data).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_os_text_keeps_the_backend_available_without_weakening_wire_validation() {
        assert!(capture_payload(Format::Text, Vec::new()).unwrap().is_none());
        assert!(payload(Format::Text, Vec::new()).is_err());
        assert!(capture_payload(Format::Png, Vec::new()).is_err());
        assert!(capture_payload(Format::Text, vec![0xff]).is_err());
        let value = capture_payload(Format::Text, b"new content".to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(value.data.as_ref(), b"new content");
    }
    #[test]
    fn desktop_payload_uses_shared_canonical_digest() {
        let a = payload(Format::Text, b"a\r\nb\rc".to_vec()).unwrap();
        let b = payload(Format::Text, b"a\nb\rc".to_vec()).unwrap();
        assert_eq!(a.meta.digest, b.meta.digest);
        assert_eq!(&*a.data, b"a\r\nb\rc");
        assert!(payload(Format::Text, b"x\0".to_vec()).is_err());
    }
}
