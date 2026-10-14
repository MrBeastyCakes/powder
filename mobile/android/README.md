# Android build

The app is a Rust `NativeActivity`: `crates/pt-app` builds a `cdylib` whose
entry point is `android_main`, and [cargo-apk] packages it with the manifest
settings in `crates/pt-app/Cargo.toml` (`[package.metadata.android]`).
Vulkan is required (`android.hardware.vulkan.level`), which covers every phone
from Android 9 (API 28) on that the benchmark targets.

## One-time setup

1. Install the Android SDK and NDK (Android Studio's SDK Manager, or
   `sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"`).
2. Point the tools at them:
   ```sh
   export ANDROID_HOME=$HOME/Android/Sdk
   export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/27.2.12479018
   ```
3. `rustup target add aarch64-linux-android`
4. `cargo install cargo-apk`

## Build and run on a phone

With the phone plugged in and USB debugging on:

```sh
cargo apk run -p pt-app --release
```

The APK lands in `target/release/apk/powder-toy-next.apk`; `cargo apk build
-p pt-app --release` builds without installing. Logs show up under the
`powder-toy` tag: `adb logcat -s powder-toy`.

## Status

The Android target type-checks in CI (`cargo check -p pt-app --target
aarch64-linux-android`). Building the APK needs the NDK, which the cloud
container that wrote this code could not download, so the first real APK
build will happen on a machine with the SDK installed.

[cargo-apk]: https://crates.io/crates/cargo-apk
