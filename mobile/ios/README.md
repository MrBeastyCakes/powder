# iOS build

`crates/pt-app` builds a static library (`libpt_app.a`) whose
`powder_toy_main()` starts the app; winit hands the main thread to UIKit and
wgpu draws through Metal. The Xcode project is generated from `project.yml`
with [XcodeGen], and its build phase compiles the Rust library first.

## One-time setup (on a Mac)

```sh
brew install xcodegen
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

## Build and run

```sh
cd mobile/ios
xcodegen generate
open PowderToy.xcodeproj
```

Pick your phone or a simulator and press Run. For a phone, set your team under
Signing & Capabilities first. The Rust build runs from the "Build Rust" phase
(`build-rust.sh`), in release mode for both Debug and Release configurations
because the simulation is unplayably slow without optimizations.

## Status

The iOS target type-checks in CI (`cargo check -p pt-app --target
aarch64-apple-ios`). Linking and running need Xcode, so the first real build
will happen on a Mac.

[XcodeGen]: https://github.com/yonaskolb/XcodeGen
