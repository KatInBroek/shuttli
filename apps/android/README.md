# Android app

The Android application uses Jetpack Compose and the same Rust mobile SDK as iOS. Its network listener runs only while the app is in the foreground and binds to the official Tailscale VPN IPv4 address. Importing the clipboard, sending a draft, and copying fetched history are three separate actions. A newly authenticated device starts with phone sending disabled and phone receiving enabled. Per-device choices and the device identity are stored locally; clipboard history is session-only. Text history stays in memory; image history uses temporary encrypted objects under the app cache directory, with a process-only key and cleanup on clear or restart.

## Build and test

Install JDK 17, Android SDK 36, build-tools 36.0.0, Android NDK 27.0.12077973, Rust, and the `aarch64-linux-android` and `x86_64-linux-android` targets. Then run from the repository root:

```bash
bash tools/mobile/build_android.sh
```

The script compiles both ABI libraries from the locked Rust workspace, generates UniFFI Kotlin bindings from the same binary, runs JVM tests, and builds an installable debug APK, an unsigned optimized release APK, and an unsigned release AAB. It writes checksums and the source commit to `target/mobile-android/manifest.json`. The Gradle wrapper is pinned to 8.13 with a distribution checksum. Build outputs, native libraries, identity files, and signing keys are excluded from Git.

On an Android emulator or authorized test device, run `cd apps/android && ./gradlew connectedDebugAndroidTest`. This covers explicit text/image import, non-sending on import, background draft clearing, persisted history settings, and image clipboard URI readback after history clear. The debug APK is at `apps/android/app/build/outputs/apk/debug/app-debug.apk`. CI uploads it along with the unsigned release APK, AAB, and test APK as `android-packages` artifacts. Release outputs need an external signing key before installation or distribution; never commit one.

With one running Android emulator, `bash tools/mobile/verify_android_emulator.sh` also installs a separate paste probe package and verifies that another app can read the exported image bytes through the system clipboard URI grant. This is repeated in CI. The probe is a test fixture and is excluded from the production app and package job.

## Real-device acceptance

A physical Android phone with the official Tailscale app and a permitted Linux desktop is still required to validate VPN address binding, discovery, authenticated traffic, desktop OS clipboard readback, multi-computer history catch-up, and cross-app image paste. Emulator tests and unsigned packages are build evidence, not a claim that these device-specific paths passed. See [Android acceptance](../../docs/mobile/android.md).
