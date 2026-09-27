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
}
