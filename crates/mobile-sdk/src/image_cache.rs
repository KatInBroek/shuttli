//! Session-key encrypted temporary PNG objects. No text or key reaches disk.
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroize;

#[derive(Debug)]
pub struct ImageObject {
    pub(crate) path: PathBuf,
    pub(crate) nonce: [u8; 24],
    pub(crate) size: usize,
}

impl Drop for ImageObject {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Debug)]
pub enum CachedBody {
    Memory(Arc<[u8]>),
    EncryptedImage(Arc<ImageObject>),
}

impl CachedBody {
    pub(crate) fn size(&self) -> usize {
        match self {
            Self::Memory(bytes) => bytes.len(),
            Self::EncryptedImage(object) => object.size,
        }
    }
}

pub struct ImageCache {
    key: [u8; 32],
    dir: PathBuf,
}

impl Drop for ImageCache {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl ImageCache {
    pub fn new(base: &Path) -> Result<Self, String> {
        let dir = base.join("shuttli-images-v1");
        if dir.exists()
            && fs::symlink_metadata(&dir)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("image cache directory is a symlink".into());
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file()
                && entry.file_name().to_string_lossy().starts_with("img-")
                && entry.file_name().to_string_lossy().ends_with(".bin")
            {
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        let mut key = [0; 32];
        getrandom::getrandom(&mut key).map_err(|e| e.to_string())?;
        Ok(Self { key, dir })
    }

    pub fn put(&self, body: &[u8]) -> Result<CachedBody, String> {
        if body.is_empty() || body.len() > 8 * 1024 * 1024 {
            return Err("invalid image size".into());
        }
        let mut nonce = [0; 24];
        getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("img-{}.bin", hex::encode(nonce)));
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), body)
            .map_err(|_| "image encryption failed")?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        if let Err(error) = file.write_all(&ciphertext) {
            let _ = fs::remove_file(&path);
            return Err(error.to_string());
        }
        Ok(CachedBody::EncryptedImage(Arc::new(ImageObject {
            path,
            nonce,
            size: body.len(),
        })))
    }

    pub fn get(&self, object: &ImageObject) -> Option<Arc<[u8]>> {
        if !self.contains(object) {
            return None;
        }
        let ciphertext = fs::read(&object.path).ok()?;
        if ciphertext.len() != object.size + 16 || ciphertext.len() > 8 * 1024 * 1024 + 16 {
            return None;
        }
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let body = match cipher.decrypt(XNonce::from_slice(&object.nonce), ciphertext.as_slice()) {
            Ok(body) => body,
            Err(_) => {
                let _ = fs::remove_file(&object.path);
                return None;
            }
        };
        Some(body.into())
    }

    pub fn contains(&self, object: &ImageObject) -> bool {
        object.path.parent() == Some(self.dir.as_path())
            && fs::symlink_metadata(&object.path).is_ok_and(|metadata| {
                metadata.file_type().is_file() && metadata.len() == object.size as u64 + 16
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_bounded_scoped_and_removed_when_last_reference_is_dropped() {
        let mut nonce = [0; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let base = std::env::temp_dir().join(format!("shuttli-cache-{}", hex::encode(nonce)));
        let cache = ImageCache::new(&base).unwrap();
        assert!(cache.put(&[]).is_err());
        assert!(cache.put(&vec![0; 8 * 1024 * 1024 + 1]).is_err());
        let body = cache.put(b"image data").unwrap();
        assert_eq!(body.size(), 10);
        let CachedBody::EncryptedImage(object) = &body else {
            panic!("must use encrypted object")
        };
        let path = object.path.clone();
        let clone = body.clone();
        assert_eq!(&*cache.get(object).unwrap(), b"image data");
        let foreign = ImageObject {
            path: base.join("elsewhere.bin"),
            nonce: object.nonce,
            size: object.size,
        };
        assert!(!cache.contains(&foreign));
        assert!(cache.get(&foreign).is_none());
        drop(body);
        assert!(path.exists());
        drop(clone);
        assert!(!path.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cache_refuses_symlink_directories_and_keeps_unrelated_files() {
        let mut nonce = [0; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let base = std::env::temp_dir().join(format!("shuttli-cache-link-{}", hex::encode(nonce)));
        std::fs::create_dir_all(base.join("target")).unwrap();
        std::os::unix::fs::symlink(base.join("target"), base.join("shuttli-images-v1")).unwrap();
        assert!(ImageCache::new(&base).is_err());
        std::fs::remove_file(base.join("shuttli-images-v1")).unwrap();
        let cache = ImageCache::new(&base).unwrap();
        let keep = cache.dir.join("unrelated.bin");
        std::fs::write(&keep, b"unrelated").unwrap();
        drop(cache);
        let _cache = ImageCache::new(&base).unwrap();
        assert!(keep.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
