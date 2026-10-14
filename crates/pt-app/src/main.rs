//! Desktop and iOS entry point. Android loads the library and starts in
//! `android_main` instead, so this binary is empty there.

#[cfg(not(target_os = "android"))]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let event_loop = winit::event_loop::EventLoop::new().expect("create event loop");
    pt_app::run(event_loop);
}

#[cfg(target_os = "android")]
fn main() {}
