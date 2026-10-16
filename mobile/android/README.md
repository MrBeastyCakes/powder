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

## Signing key for CI builds

CI signs every APK with one key so each build installs over the last one and
keeps its saves. The key lives in two repository secrets. To set them up once,
on a machine with a JDK and the GitHub CLI:

```sh
keytool -genkeypair -keystore powder.keystore -alias powder -keyalg RSA \
  -keysize 2048 -validity 10000 -dname "CN=Powder Toy Next"   # asks for a password
gh secret set ANDROID_KEYSTORE_PASSWORD -R MrBeastyCakes/powder  # paste the same password
base64 < powder.keystore | tr -d "\n" | gh secret set ANDROID_KEYSTORE_BASE64 -R MrBeastyCakes/powder
```

Keep `powder.keystore` somewhere safe and out of the repository. Without the
secrets, CI makes a throwaway key per build (and warns), so each APK must
replace the installed app. The first build with the new key also needs one
uninstall, since its key differs from the earlier throwaway ones.

## Startup diagnostics

If an installed APK closes on launch, capture both the native and Android
crash logs. Building an APK or passing headless GPU tests does not verify
NativeActivity startup on a phone.

```sh
adb logcat -c
adb shell am force-stop app.powdertoy.next
adb shell am start -W -n app.powdertoy.next/android.app.NativeActivity
adb logcat -d -v threadtime > powder-startup.log
adb logcat -b crash -d -v threadtime > powder-crash.log
```

The `powder-toy` tag reports startup stages, Rust panic details, and
`startup: first frame presented` after the first presentation on each new
surface. Keep the full logs: linker failures, Java exceptions, native
signals, and GPU driver errors can use other tags.

[cargo-apk]: https://crates.io/crates/cargo-apk
