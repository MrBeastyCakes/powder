//! `pt-bench`: runs the Phase 0 benchmark and can save screenshots.
//!
//! ```text
//! cargo run --release -p pt-bench                    # default sizes
//! cargo run --release -p pt-bench -- --ticks 60 --2d 1080x2400 --3d 128,256
//! cargo run --release -p pt-bench -- --screenshots out/
//! ```

use std::path::{Path, PathBuf};

use pt_core::gpu::Gpu;
use pt_core::MaterialTable;
use pt_sim2d::{Overlay, Renderer2d, Sim2d};
use pt_sim3d::{OrbitCamera, Renderer3d, Sim3d, View3d};

struct Args {
    ticks: u32,
    sizes_2d: Vec<(u32, u32)>,
    sizes_3d: Vec<u32>,
    screenshots: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        ticks: 120,
        // Common phone screens: 1080p-class Android, iPhone Pro, iPhone Pro Max.
        sizes_2d: vec![(1080, 2400), (1179, 2556), (1290, 2796)],
        sizes_3d: vec![64, 128, 192, 256],
        screenshots: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--ticks" => args.ticks = value()?.parse().map_err(|e| format!("--ticks: {e}"))?,
            "--2d" => {
                args.sizes_2d = value()?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let (w, h) = s.split_once('x').ok_or(format!("--2d wants WIDTHxHEIGHT, got {s}"))?;
                        Ok((w.parse().map_err(|e| format!("{e}"))?, h.parse().map_err(|e| format!("{e}"))?))
                    })
                    .collect::<Result<_, String>>()?
            }
            "--3d" => {
                args.sizes_3d = value()?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().map_err(|e| format!("--3d: {e}")))
                    .collect::<Result<_, _>>()?
            }
            "--screenshots" => args.screenshots = Some(PathBuf::from(value()?)),
            "-h" | "--help" => {
                println!("pt-bench [--ticks N] [--2d WxH,...] [--3d N,...] [--screenshots DIR]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let gpu = Gpu::headless().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let table = MaterialTable::core();

    if let Some(dir) = &args.screenshots {
        std::fs::create_dir_all(dir).expect("create screenshot folder");
        screenshots(&gpu, &table, dir);
        return;
    }

    let mut results = Vec::new();
    for &(w, h) in &args.sizes_2d {
        let r = pt_bench::bench_2d(&gpu, &table, w, h, args.ticks);
        eprintln!("{}: {:.2} ms per tick", r.label, r.ms_per_tick);
        results.push(r);
    }
    for &n in &args.sizes_3d {
        match pt_bench::bench_3d(&gpu, &table, n, (args.ticks / 4).max(4)) {
            Some(r) => {
                eprintln!("{}: {:.2} ms per tick", r.label, r.ms_per_tick);
                results.push(r);
            }
            None => eprintln!("3D {n}^3: does not fit in this GPU's buffer limits"),
        }
    }
    println!("{}", pt_bench::table(&gpu.adapter_name(), &results));
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn target(gpu: &Gpu, w: u32, h: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn save(gpu: &Gpu, texture: &wgpu::Texture, path: &Path) {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback"),
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let padded: Vec<u8> = gpu.read_buffer(&buffer, 0, (row * h) as usize);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&padded[start..start + (w * 4) as usize]);
    }
    let file = std::fs::File::create(path).expect("create screenshot file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut wr| wr.write_image_data(&pixels))
        .expect("write png");
    println!("wrote {}", path.display());
}

fn screenshots(gpu: &Gpu, table: &MaterialTable, dir: &Path) {
    // 2D: a phone-shaped world, one cell per pixel.
    let (w, h) = (540, 1200);
    let scene = pt_sim2d::scenes::demo(table, w, h, 3);
    let mut sim = Sim2d::new(gpu, table.clone(), w, h, 3);
    sim.upload(gpu, &scene.cells, &scene.temps);
    let renderer = Renderer2d::new(gpu, &sim, FORMAT);
    let tex = target(gpu, w, h);
    let view = tex.create_view(&Default::default());
    for (ticks, name) in [(0, "2d-start"), (300, "2d-later"), (900, "2d-much-later")] {
        sim.run(gpu, ticks);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.draw(&gpu.queue, &mut encoder, &view, &sim, [0, 0], Overlay::None);
        gpu.queue.submit([encoder.finish()]);
        save(gpu, &tex, &dir.join(format!("{name}.png")));
    }
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&gpu.queue, &mut encoder, &view, &sim, [0, 0], Overlay::Temperature);
    gpu.queue.submit([encoder.finish()]);
    save(gpu, &tex, &dir.join("2d-temperature.png"));

    // 3D: a 128-cubed world seen from the default camera.
    let dims = [128, 128, 128];
    let scene = pt_sim3d::scenes::demo(table, dims, 3);
    let mut sim = Sim3d::new(gpu, table.clone(), dims, 3);
    sim.upload(gpu, &scene.cells, &scene.temps);
    let renderer = Renderer3d::new(gpu, &sim, FORMAT);
    let (w, h) = (720, 1280);
    let tex = target(gpu, w, h);
    let view = tex.create_view(&Default::default());
    let camera = OrbitCamera::looking_at(dims);
    for (ticks, name) in [(0, "3d-start"), (200, "3d-later")] {
        sim.run(gpu, ticks);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.draw(
            &gpu.queue,
            &mut encoder,
            &view,
            [w, h],
            &sim,
            &camera,
            View3d {
                show_plane: name == "3d-start",
                brush_radius: 4.0,
                temperature: false,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        save(gpu, &tex, &dir.join(format!("{name}.png")));
    }
}
