//! Headless GPU compute probe.
//!
//! Requests a Vulkan adapter without a surface, runs a compute shader that
//! doubles 1024 `u32` values, reads the result back, and checks it.

use std::process::ExitCode;

use wgpu::util::DeviceExt;

const N: u32 = 1024;
const WORKGROUP_SIZE: u32 = 64;

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> input: array<u32>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i < arrayLength(&input)) {
        output[i] = input[i] * 2u;
    }
}
"#;

fn main() -> ExitCode {
    match pollster::block_on(run()) {
        Ok(()) => {
            println!("gpu-probe ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gpu-probe failed: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|e| format!("no adapter found: {e}"))?;

    let info = adapter.get_info();
    println!("adapter: {}", info.name);
    println!("backend: {:?}", info.backend);
    println!("driver: {} ({})", info.driver, info.driver_info);

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|e| format!("request_device failed: {e}"))?;

    let input: Vec<u32> = (0..N).collect();
    let size = u64::from(N) * std::mem::size_of::<u32>() as u64;

    let input_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("input"),
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("double"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("double"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("double"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output_buf.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("probe"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("double"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(N.div_ceil(WORKGROUP_SIZE), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output_buf, 0, &readback_buf, 0, size);
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    readback_buf.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("poll failed: {e}"))?;
    rx.recv()
        .map_err(|e| format!("map callback dropped: {e}"))?
        .map_err(|e| format!("map failed: {e}"))?;

    {
        let view = readback_buf
            .get_mapped_range(..)
            .map_err(|e| format!("get_mapped_range failed: {e}"))?;
        let out: &[u32] = bytemuck::cast_slice(&view);
        if out.len() != N as usize {
            return Err(format!("expected {N} results, got {}", out.len()));
        }
        for (i, &v) in out.iter().enumerate() {
            let want = 2 * i as u32;
            if v != want {
                return Err(format!("mismatch at {i}: got {v}, want {want}"));
            }
        }
    }
    readback_buf.unmap();
    Ok(())
}
