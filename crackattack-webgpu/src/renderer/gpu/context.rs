//! GPU device, queue, surface, and viewport configuration.

pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    /// True if the surface supports `COPY_SRC`, so screenshots are possible. The usage is
    /// negotiated: requiring it would make configuration fail on backends without it.
    pub capture_supported: bool,
    pub viewport_x: u32,
    pub viewport_y: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[cfg(feature = "native")]
impl GpuContext {
    /// Creates the GPU context, independent of the windowing platform.
    ///
    /// The surface is built by `create_surface`, supplied by the binary's `PlatformShell`
    /// layer, and the pixel size is passed explicitly. No window type is needed here.
    pub fn new(
        create_surface: impl FnOnce(&wgpu::Instance) -> Result<wgpu::Surface<'static>, Box<dyn std::error::Error>>,
        pixel_size: (u32, u32),
    ) -> Self {
        use pollster::FutureExt;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // GPU init failures exit cleanly with status 1 (fail-fast) instead of panicking.
        // Most likely cause: no adapter (headless, CI, missing driver).
        let fatal = |msg: &str| -> ! {
            crate::platform::diagnostics::fatal(&format!(
                "The game could not start its graphics ({msg}). It needs a graphics card and driver that \
                 support Metal (macOS), DirectX 12 or Vulkan (Windows), or Vulkan (Linux)."))
        };

        let surface = create_surface(&instance)
            .unwrap_or_else(|e| fatal(&format!("surface: {e}")));

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .block_on()
            .unwrap_or_else(|| fatal("no compatible graphics card found; DirectX 11 is not supported"));

        log::info!("Using adapter: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("crackattack_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .block_on()
            .unwrap_or_else(|e| fatal(&format!("device: {e}")));

        // Device loss (GPU switch on dual-GPU Macs, eGPU unplug, TDR, sleep/wake): without
        // a callback the default wgpu handler panics. Log a clear message and exit instead.
        // Recovery (recreating device and surface) is not attempted.
        device.set_device_lost_callback(Box::new(|reason, msg| {
            log::error!("GPU device lost ({reason:?}): {msg}");
            eprintln!("Fatal error: GPU device lost ({reason:?}).");
            std::process::exit(1);
        }));

        let (size_w, size_h) = pixel_size;
        let surface_caps = surface.get_capabilities(&adapter);

        let capture_supported = surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        if !capture_supported {
            log::warn!("Surface without COPY_SRC: screenshots unavailable on this backend");
        }

        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or_else(|| fatal("no sRGB surface format"));

        let alpha_mode = if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        };

        log::info!("Surface format: {:?} (sRGB: {})", surface_format, surface_format.is_srgb());
        log::info!("Alpha mode: {:?}", alpha_mode);
        log::info!("Present mode: Fifo (vsync)");

        let config = wgpu::SurfaceConfiguration {
            usage: if capture_supported {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format: surface_format,
            width: size_w.max(1),
            height: size_h.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let viewport_width = config.width;
        let viewport_height = config.height;

        Self {
            device, queue, surface, config, capture_supported,
            viewport_x: 0, viewport_y: 0,
            viewport_width, viewport_height,
        }
    }
}
