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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_payload_uses_shared_canonical_digest() {
        let a = payload(Format::Text, b"a\r\nb\rc".to_vec()).unwrap();
        let b = payload(Format::Text, b"a\nb\rc".to_vec()).unwrap();
        assert_eq!(a.meta.digest, b.meta.digest);
        assert_eq!(&*a.data, b"a\r\nb\rc");
        assert!(payload(Format::Text, b"x\0".to_vec()).is_err());
    }
}
