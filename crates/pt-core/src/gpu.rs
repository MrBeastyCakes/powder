//! GPU device setup shared by the app, the benchmark and the tests.

use crate::MaterialTable;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("no GPU adapter found: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("could not open the GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
}

/// A device and queue, plus the adapter they came from.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// The backends the app runs on: Vulkan (Android, Linux, Windows) and
    /// Metal (iOS, macOS). Simulation shaders need storage buffers in compute
    /// and fragment stages, which the GL backend cannot promise on phones.
    pub fn backends() -> wgpu::Backends {
        wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12
    }

    pub fn instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: Gpu::backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    }

    /// Opens a device with no window, for tests and the benchmark.
    pub fn headless() -> Result<Gpu, GpuError> {
        let instance = Gpu::instance();
        pollster::block_on(Gpu::open(instance, None))
    }

    /// Opens a device that can draw to `surface` when one is given.
    pub async fn open(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Gpu, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: surface,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type);

        // Ask for everything the adapter offers: the 3D world needs storage
        // buffers far larger than the WebGPU defaults.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("powder-toy"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await?;
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    /// Blocks until all submitted work has finished.
    pub fn wait(&self) {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU device lost while waiting");
    }

    /// Copies a buffer back to the CPU. Slow; for tests and the inspect tool.
    pub fn read_buffer<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer, offset: u64, count: usize) -> Vec<T> {
        let size = (count * std::mem::size_of::<T>()) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.wait();
        let data = {
            let view = slice.get_mapped_range().expect("read mapped readback buffer");
            bytemuck::cast_slice::<u8, T>(&view).to_vec()
        };
        staging.unmap();
        data
    }
}

/// The uniform buffer holding the compiled material table.
pub fn material_buffer(device: &wgpu::Device, table: &MaterialTable) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("materials"),
        contents: table.gpu_bytes(),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Bind group layout entry helpers, so each simulation's layouts read as a list.
pub mod layout {
    pub fn uniform(binding: u32, visibility: wgpu::ShaderStages, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        }
    }

    pub fn storage(binding: u32, visibility: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }
    }
}

/// Builds a compute pipeline for one entry point of `module`.
pub fn compute_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// Size of one slot in a per-tick uniform array; dynamic offsets must be
/// aligned to this on every GPU wgpu supports.
pub const UNIFORM_SLOT: u64 = 256;
