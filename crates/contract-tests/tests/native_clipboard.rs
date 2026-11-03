//! Runs only in an explicitly isolated X11 session; never against a user's clipboard.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Xvfb and xclip; run explicitly in CI"]
fn production_clipboard_checks_generation_and_independent_os_readback() {
    use shuttli_adapters::{clipboard, content::payload};
    use shuttli_core::sync::SyncCore;
    use shuttli_model::sync::*;
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    fn external_copy(bytes: &[u8]) {
        let mut child = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        assert!(child.wait().unwrap().success());
    }
    fn wait_value(
        clip: &mut dyn shuttli_ports::sync::Clipboard,
        bytes: &[u8],
    ) -> shuttli_ports::sync::ClipboardValue {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(value) = clip.read() {
                if value
                    .payload
                    .as_ref()
                    .is_some_and(|p| p.data.as_ref() == bytes)
                {
                    return value;
                }
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let mut clip = clipboard::open().unwrap();
    external_copy(b"synthetic original");
    let first = wait_value(clip.as_mut(), b"synthetic original");
    let body = payload(Format::Text, b"synthetic application write".to_vec()).unwrap();
    let core = SyncCore::new([1; 32], [1; 16], Settings::default());
    let stale = core.authorize_local_copy(first.stamp, body.meta.clone());
    external_copy(b"synthetic external replacement");
    let changed = wait_value(clip.as_mut(), b"synthetic external replacement");
    assert_ne!(first.stamp, changed.stamp);
    assert!(
        clip.write(&body, stale).is_err(),
        "stale capture must not overwrite a new OS copy"
    );
    clip.write(
        &body,
        core.authorize_local_copy(changed.stamp, body.meta.clone()),
    )
    .unwrap();
    let external = Command::new("xclip")
        .args(["-selection", "clipboard", "-out"])
        .output()
        .unwrap();
    assert!(external.status.success());
    assert_eq!(
        external.stdout,
        body.data.as_ref(),
        "independent OS client must see the write"
    );
    external_copy(b"synthetic after readback");
    wait_value(clip.as_mut(), b"synthetic after readback");
    external_copy(b"");
    let deadline = Instant::now() + Duration::from_secs(3);
    let empty = loop {
        if let Ok(value) = clip.read() {
            if value.payload.is_none() {
                break value;
            }
        }
        assert!(
            Instant::now() < deadline,
            "empty text must be a healthy clipboard revision"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let replacement = payload(Format::Text, b"incoming after empty clipboard".to_vec()).unwrap();
    clip.write(
        &replacement,
        core.authorize_local_copy(empty.stamp, replacement.meta.clone()),
    )
    .unwrap();
    let external = Command::new("xclip")
        .args(["-selection", "clipboard", "-out"])
        .output()
        .unwrap();
    assert_eq!(external.stdout, replacement.data.as_ref());

    // Exercise the separate ICCCM INCR path with incompressible multi-megabyte
    // images, repeatedly starting from the previously broken empty-text state.
    for cycle in 0..5u32 {
        external_copy(b"");
        let deadline = Instant::now() + Duration::from_secs(3);
        let baseline = loop {
            if let Ok(value) = clip.read() {
                if value.payload.is_none() {
                    break value.stamp;
                }
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut seed = 0x1234abcd_u32.wrapping_add(cycle);
        let pixels: Vec<u8> = (0..1024 * 768 * 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 1024, 768);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        assert!(
            encoded.len() > 60 * 1024,
            "must exercise INCR rather than the small selection path"
        );
        let image = payload(Format::Png, encoded).unwrap();
        clip.write(
            &image,
            core.authorize_local_copy(baseline, image.meta.clone()),
        )
        .unwrap();
        let external = Command::new("xclip")
            .args(["-selection", "clipboard", "-target", "image/png", "-out"])
            .output()
            .unwrap();
        assert!(external.status.success());
        assert_eq!(external.stdout, image.data.as_ref());
        // Optional GTK acceptance driver, supplied explicitly by the operator.
        // It receives expected bytes on stdin and must compare an independent
        // clipboard read. Keep GTK optional for the ordinary Xvfb CI job.
        if cycle == 0 {
            if let Some(driver) = std::env::var_os("SHUTTLI_GDK_READBACK_DRIVER") {
                let mut reader = Command::new("python3")
                    .arg(driver)
                    .env("SHUTTLI_TEST_SESSION_PID", std::process::id().to_string())
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                reader.stdin.take().unwrap().write_all(&image.data).unwrap();
                assert!(reader.wait().unwrap().success());
            }
        }
    }
}
