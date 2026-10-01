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
