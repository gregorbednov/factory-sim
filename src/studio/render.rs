use std::{ops::Range, sync::Arc};

use bytemuck::{Pod, Zeroable};
use eframe::{
    egui::{PaintCallbackInfo, Rect},
    egui_wgpu::{Callback, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor},
};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use wgpu::{self as gpu, util::DeviceExt};

use crate::product::Mesh;

const COLOR: gpu::TextureFormat = gpu::TextureFormat::Rgba8UnormSrgb;
// Positive Z face in cube_vertices, reused as a two-sided window.
const WINDOW_FACE: Range<u32> = 24..30;
const DEPTH: gpu::TextureFormat = gpu::TextureFormat::Depth32Float;
const CLEAR: gpu::Color = gpu::Color {
    r: 0.035,
    g: 0.045,
    b: 0.054,
    a: 1.0,
};
pub const SUN: Vec3 = Vec3::new(-0.48, 0.84, 0.25);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsQuality {
    Performance,
    #[default]
    Balanced,
    High,
}

impl GraphicsQuality {
    fn shadow_size(self) -> u32 {
        match self {
            Self::Performance => 1,
            Self::Balanced => 1024,
            Self::High => 1536,
        }
    }

    pub fn radial_segments(self) -> usize {
        match self {
            Self::Performance => 8,
            Self::Balanced => 12,
            Self::High => 20,
        }
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
#[repr(C)]
pub struct Instance {
    pub center: [f32; 4],
    pub axis_x: [f32; 4],
    pub axis_y: [f32; 4],
    pub axis_z: [f32; 4],
    pub color: [f32; 4],
    pub surface: [f32; 4],
}

#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
#[repr(C)]
struct Frame {
    camera: [[f32; 4]; 4],
    light: [[f32; 4]; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    viewport: [f32; 4],
}

pub fn install(state: &RenderState) {
    let required = gpu::TextureFormatFeatureFlags::MULTISAMPLE_X4;
    let supports_msaa = [COLOR, DEPTH].iter().all(|format| {
        state
            .adapter
            .get_texture_format_features(*format)
            .flags
            .contains(required)
    }) && state
        .adapter
        .get_texture_format_features(COLOR)
        .flags
        .contains(gpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    let graphics = Graphics::new(
        &state.device,
        state.target_format,
        if supports_msaa { 4 } else { 1 },
    );
    state.renderer.write().callback_resources.insert(graphics);
}

pub struct MeshBatch {
    pub mesh: Arc<Mesh>,
    pub instances: Range<u32>,
}

pub fn paint(
    rect: Rect,
    camera: Mat4,
    eye: Vec3,
    target: Vec3,
    quality: GraphicsQuality,
    cube_count: u32,
    window_start: u32,
    instances: Vec<Instance>,
    meshes: Vec<MeshBatch>,
) -> eframe::egui::epaint::PaintCallback {
    let sun = SUN.normalize();
    let light_target = Vec3::new(target.x, 0.0, target.z);
    let light = Mat4::orthographic_rh(-13.0, 13.0, -13.0, 13.0, 1.0, 60.0)
        * Mat4::look_at_rh(light_target + sun * 28.0, light_target, Vec3::Y);
    Callback::new_paint_callback(
        rect,
        Draw {
            rect,
            frame: Frame {
                camera: camera.to_cols_array_2d(),
                light: light.to_cols_array_2d(),
                eye: eye
                    .extend(f32::from(quality != GraphicsQuality::Performance))
                    .to_array(),
                sun: sun
                    .extend(f32::from(quality == GraphicsQuality::High))
                    .to_array(),
                viewport: [0.0; 4],
            },
            quality,
            cube_count,
            window_start,
            instances,
            meshes,
        },
    )
}

struct Draw {
    rect: Rect,
    frame: Frame,
    quality: GraphicsQuality,
    cube_count: u32,
    window_start: u32,
    instances: Vec<Instance>,
    meshes: Vec<MeshBatch>,
}

impl CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &gpu::Device,
        queue: &gpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut gpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<gpu::CommandBuffer> {
        let limit = device.limits().max_texture_dimension_2d;
        let size = [
            (self.rect.width() * screen.pixels_per_point)
                .round()
                .max(1.0) as u32,
            (self.rect.height() * screen.pixels_per_point)
                .round()
                .max(1.0) as u32,
        ]
        .map(|extent| {
            let extent = if self.quality == GraphicsQuality::Performance {
                extent.saturating_mul(4).div_ceil(5)
            } else {
                extent
            };
            extent.min(limit)
        });
        resources
            .get_mut::<Graphics>()
            .unwrap()
            .draw(device, queue, encoder, size, self);
        Vec::new()
    }

    fn paint(
        &self,
        _: PaintCallbackInfo,
        pass: &mut gpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let graphics = resources.get::<Graphics>().unwrap();
        if let Some(targets) = &graphics.targets {
            pass.set_pipeline(&graphics.composite);
            pass.set_bind_group(0, &targets.composite, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

struct Targets {
    size: [u32; 2],
    color: gpu::TextureView,
    antialias: Option<gpu::TextureView>,
    depth: gpu::TextureView,
    quality: GraphicsQuality,
    shadow_depth: gpu::TextureView,
    shadow_group: gpu::BindGroup,
    reflected_color: gpu::TextureView,
    reflected_depth: Option<gpu::TextureView>,
    reflection: gpu::BindGroup,
    composite: gpu::BindGroup,
}

struct GpuMesh {
    mesh: Arc<Mesh>,
    vertices: gpu::Buffer,
    count: u32,
}

impl GpuMesh {
    fn new(device: &gpu::Device, mesh: Arc<Mesh>) -> Self {
        let vertices: Vec<_> = mesh
            .triangles
            .iter()
            .flat_map(|triangle| {
                let [a, b, c] = triangle.map(|index| Vec3::from(mesh.positions[index as usize]));
                let face_normal = (b - a).cross(c - a).normalize_or_zero().to_array();
                triangle.map(|index| {
                    let index = index as usize;
                    let normal = mesh
                        .normals
                        .as_ref()
                        .map(|normals| normals[index])
                        .filter(|n| *n != [0.0; 3])
                        .unwrap_or(face_normal);
                    Vertex {
                        position: mesh.positions[index],
                        normal,
                    }
                })
            })
            .collect();
        let count = vertices.len() as u32;
        let vertices = device.create_buffer_init(&gpu::util::BufferInitDescriptor {
            label: Some("scene mesh"),
            contents: bytemuck::cast_slice(&vertices),
            usage: gpu::BufferUsages::VERTEX,
        });
        Self {
            mesh,
            vertices,
            count,
        }
    }
}

struct Graphics {
    frame: gpu::Buffer,
    frame_group: gpu::BindGroup,
    shadow_layout: gpu::BindGroupLayout,
    shadow_sampler: gpu::Sampler,
    texture_layout: gpu::BindGroupLayout,
    sampler: gpu::Sampler,
    cube: gpu::Buffer,
    meshes: Vec<GpuMesh>,
    instances: gpu::Buffer,
    instance_capacity: usize,
    shadow: gpu::RenderPipeline,
    reflection: gpu::RenderPipeline,
    scene: gpu::RenderPipeline,
    scene_msaa: Option<gpu::RenderPipeline>,
    window: gpu::RenderPipeline,
    window_msaa: Option<gpu::RenderPipeline>,
    composite: gpu::RenderPipeline,
    targets: Option<Targets>,
    previous_frame: Option<Frame>,
    previous_instances: Vec<Instance>,
    previous_cube_count: u32,
    previous_window_start: u32,
    previous_mesh_ranges: Vec<Range<u32>>,
}

impl Graphics {
    fn new(device: &gpu::Device, output: gpu::TextureFormat, sample_count: u32) -> Self {
        let uniform_layout = device.create_bind_group_layout(&gpu::BindGroupLayoutDescriptor {
            label: Some("scene uniform layout"),
            entries: &[gpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: gpu::ShaderStages::VERTEX_FRAGMENT,
                ty: gpu::BindingType::Buffer {
                    ty: gpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let frame = device.create_buffer(&gpu::BufferDescriptor {
            label: Some("scene frame"),
            size: size_of::<Frame>() as u64,
            usage: gpu::BufferUsages::UNIFORM | gpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&gpu::BindGroupDescriptor {
            label: Some("scene frame group"),
            layout: &uniform_layout,
            entries: &[gpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            }],
        });
        let shadow_layout = texture_layout(device, true);
        let texture_layout = texture_layout(device, false);
        let shadow_sampler = device.create_sampler(&gpu::SamplerDescriptor {
            label: Some("PCF shadow sampler"),
            compare: Some(gpu::CompareFunction::LessEqual),
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler = device.create_sampler(&gpu::SamplerDescriptor {
            label: Some("scene texture sampler"),
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });

        let module = shader(device, "scene and lights", include_str!("scene.wgsl"));
        const CUBE: [gpu::VertexAttribute; 2] =
            gpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        const INSTANCES: [gpu::VertexAttribute; 6] = gpu::vertex_attr_array![
            2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
        ];
        let buffers = [
            Some(gpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: gpu::VertexStepMode::Vertex,
                attributes: &CUBE,
            }),
            Some(gpu::VertexBufferLayout {
                array_stride: size_of::<Instance>() as u64,
                step_mode: gpu::VertexStepMode::Instance,
                attributes: &INSTANCES,
            }),
        ];
        let shadow = pipeline(
            device,
            &module,
            "vs_shadow",
            None,
            &buffers,
            &[Some(&uniform_layout)],
            true,
            None,
            1,
        );
        let reflection = pipeline(
            device,
            &module,
            "vs_reflection",
            Some(("fs_reflection", COLOR)),
            &buffers,
            &[Some(&uniform_layout), Some(&shadow_layout)],
            true,
            None,
            1,
        );
        let scene_pipeline = |samples| {
            pipeline(
                device,
                &module,
                "vs_scene",
                Some(("fs_scene", COLOR)),
                &buffers,
                &[
                    Some(&uniform_layout),
                    Some(&shadow_layout),
                    Some(&texture_layout),
                ],
                true,
                None,
                samples,
            )
        };
        let scene = scene_pipeline(1);
        let scene_msaa = (sample_count > 1).then(|| scene_pipeline(sample_count));
        let window_pipeline = |samples| {
            pipeline(
                device,
                &module,
                "vs_scene",
                Some(("fs_window", COLOR)),
                &buffers,
                &[Some(&uniform_layout)],
                true,
                Some(gpu::BlendState::ALPHA_BLENDING),
                samples,
            )
        };
        let window = window_pipeline(1);
        let window_msaa = (sample_count > 1).then(|| window_pipeline(sample_count));
        let blit = shader(device, "viewport composite", include_str!("composite.wgsl"));
        let fragment = if output.is_srgb() {
            "fs_linear"
        } else {
            "fs_srgb"
        };
        let composite = pipeline(
            device,
            &blit,
            "vs_main",
            Some((fragment, output)),
            &[],
            &[Some(&texture_layout)],
            false,
            None,
            1,
        );

        let cube = device.create_buffer_init(&gpu::util::BufferInitDescriptor {
            label: Some("unit cube"),
            contents: bytemuck::cast_slice(&cube_vertices()),
            usage: gpu::BufferUsages::VERTEX,
        });
        let instance_capacity = 128;
        let instances = instance_buffer(device, instance_capacity);
        Self {
            frame,
            frame_group,
            shadow_layout,
            shadow_sampler,
            texture_layout,
            sampler,
            cube,
            meshes: Vec::new(),
            instances,
            instance_capacity,
            shadow,
            reflection,
            scene,
            scene_msaa,
            window,
            window_msaa,
            composite,
            targets: None,
            previous_frame: None,
            previous_instances: Vec::new(),
            previous_cube_count: 0,
            previous_window_start: 0,
            previous_mesh_ranges: Vec::new(),
        }
    }

    fn draw(
        &mut self,
        device: &gpu::Device,
        queue: &gpu::Queue,
        encoder: &mut gpu::CommandEncoder,
        size: [u32; 2],
        draw: &Draw,
    ) {
        let resized = self
            .targets
            .as_ref()
            .is_none_or(|targets| targets.size != size || targets.quality != draw.quality);
        let same_meshes = self.meshes.len() == draw.meshes.len()
            && self
                .meshes
                .iter()
                .zip(&draw.meshes)
                .all(|(cached, batch)| Arc::ptr_eq(&cached.mesh, &batch.mesh));
        if !resized
            && same_meshes
            && self.previous_cube_count == draw.cube_count
            && self.previous_window_start == draw.window_start
            && self
                .previous_mesh_ranges
                .iter()
                .eq(draw.meshes.iter().map(|batch| &batch.instances))
            && self.previous_frame == Some(draw.frame)
            && self.previous_instances == draw.instances
        {
            return; // egui can redraw controls over the last completed scene texture.
        }
        if !same_meshes {
            let mut old = std::mem::take(&mut self.meshes);
            self.meshes = draw
                .meshes
                .iter()
                .map(|batch| {
                    old.iter()
                        .position(|cached| Arc::ptr_eq(&cached.mesh, &batch.mesh))
                        .map_or_else(
                            || GpuMesh::new(device, batch.mesh.clone()),
                            |index| old.swap_remove(index),
                        )
                })
                .collect();
        }
        self.previous_cube_count = draw.cube_count;
        self.previous_window_start = draw.window_start;
        self.previous_mesh_ranges.clear();
        self.previous_mesh_ranges
            .extend(draw.meshes.iter().map(|batch| batch.instances.clone()));
        self.previous_frame = Some(draw.frame);
        self.previous_instances.clone_from(&draw.instances);
        let high = draw.quality == GraphicsQuality::High;
        let msaa = self.scene_msaa.as_ref().filter(|_| high);
        if resized {
            let samples = if msaa.is_some() { 4 } else { 1 };
            let reflected = if high {
                size.map(|extent| extent.div_ceil(2))
            } else {
                [1, 1]
            };
            let color = texture(device, size, COLOR, true, 1);
            let reflected_color = texture(device, reflected, COLOR, true, 1);
            let shadow_size = draw
                .quality
                .shadow_size()
                .min(device.limits().max_texture_dimension_2d);
            let shadow_depth = texture(device, [shadow_size; 2], DEPTH, true, 1);
            let shadow_group = texture_group(
                device,
                &self.shadow_layout,
                &shadow_depth,
                &self.shadow_sampler,
            );
            self.targets = Some(Targets {
                size,
                quality: draw.quality,
                depth: texture(device, size, DEPTH, false, samples),
                antialias: msaa.map(|_| texture(device, size, COLOR, false, samples)),
                shadow_depth,
                shadow_group,
                reflected_depth: high.then(|| texture(device, reflected, DEPTH, false, 1)),
                reflection: texture_group(
                    device,
                    &self.texture_layout,
                    &reflected_color,
                    &self.sampler,
                ),
                composite: texture_group(device, &self.texture_layout, &color, &self.sampler),
                color,
                reflected_color,
            });
        }
        let targets = self.targets.as_ref().unwrap();
        if draw.instances.len() > self.instance_capacity {
            self.instance_capacity = draw.instances.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        let mut frame = draw.frame;
        frame.viewport = [
            size[0] as f32,
            size[1] as f32,
            1.0 / size[0] as f32,
            1.0 / size[1] as f32,
        ];
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&draw.instances));

        let count = draw.cube_count;
        let objects = count - 1; // The floor is appended after all shadow and reflection casters.
        if draw.quality != GraphicsQuality::Performance {
            let mut pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
                label: Some("directional shadow map"),
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.shadow_depth,
                    gpu::StoreOp::Store,
                )),
                ..Default::default()
            });
            pass.set_pipeline(&self.shadow);
            self.geometry(&mut pass);
            self.draw_objects(&mut pass, draw, objects);
        }
        if let Some(reflected_depth) = &targets.reflected_depth {
            let mut pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
                label: Some("planar floor reflection"),
                color_attachments: &[Some(color_attachment(
                    &targets.reflected_color,
                    gpu::Color::TRANSPARENT,
                ))],
                depth_stencil_attachment: Some(depth_attachment(
                    reflected_depth,
                    gpu::StoreOp::Discard,
                )),
                ..Default::default()
            });
            pass.set_pipeline(&self.reflection);
            self.geometry(&mut pass);
            pass.set_bind_group(1, &targets.shadow_group, &[]);
            self.draw_objects(&mut pass, draw, objects);
        }
        {
            let mut color = color_attachment(&targets.color, CLEAR);
            if let Some(antialias) = &targets.antialias {
                color.view = antialias;
                color.resolve_target = Some(&targets.color);
                color.ops.store = gpu::StoreOp::Discard;
            }
            let mut pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
                label: Some("viewport color and depth"),
                color_attachments: &[Some(color)],
                depth_stencil_attachment: Some(depth_attachment(
                    &targets.depth,
                    gpu::StoreOp::Discard,
                )),
                ..Default::default()
            });
            pass.set_pipeline(msaa.unwrap_or(&self.scene));
            self.geometry(&mut pass);
            pass.set_bind_group(1, &targets.shadow_group, &[]);
            pass.set_bind_group(2, &targets.reflection, &[]);
            self.draw_objects(&mut pass, draw, count);
            let windows = draw.window_start..draw.instances.len() as u32;
            if !windows.is_empty() {
                let pipeline = self
                    .window_msaa
                    .as_ref()
                    .filter(|_| high)
                    .unwrap_or(&self.window);
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.draw(WINDOW_FACE, windows);
            }
        }
    }

    fn geometry(&self, pass: &mut gpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.set_bind_group(0, &self.frame_group, &[]);
    }

    fn draw_objects(&self, pass: &mut gpu::RenderPass<'_>, draw: &Draw, cubes: u32) {
        pass.set_vertex_buffer(0, self.cube.slice(..));
        pass.draw(0..36, 0..cubes);
        for (mesh, batch) in self.meshes.iter().zip(&draw.meshes) {
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.draw(0..mesh.count, batch.instances.clone());
        }
    }
}

fn shader(device: &gpu::Device, name: &str, wgsl: &str) -> gpu::ShaderModule {
    device.create_shader_module(gpu::ShaderModuleDescriptor {
        label: Some(name),
        source: gpu::ShaderSource::Wgsl(wgsl.into()),
    })
}

fn pipeline(
    device: &gpu::Device,
    module: &gpu::ShaderModule,
    vertex: &str,
    fragment: Option<(&str, gpu::TextureFormat)>,
    buffers: &[Option<gpu::VertexBufferLayout<'_>>],
    layouts: &[Option<&gpu::BindGroupLayout>],
    depth: bool,
    blend: Option<gpu::BlendState>,
    sample_count: u32,
) -> gpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&gpu::PipelineLayoutDescriptor {
        label: Some(vertex),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    let targets = fragment.map(|(_, format)| {
        [Some(gpu::ColorTargetState {
            format,
            blend,
            write_mask: gpu::ColorWrites::ALL,
        })]
    });
    device.create_render_pipeline(&gpu::RenderPipelineDescriptor {
        label: Some(vertex),
        layout: Some(&layout),
        vertex: gpu::VertexState {
            module,
            entry_point: Some(vertex),
            buffers,
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: depth.then(|| gpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(blend.is_none()),
            depth_compare: Some(gpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: gpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        },
        fragment: fragment.map(|(entry, _)| gpu::FragmentState {
            module,
            entry_point: Some(entry),
            targets: targets.as_ref().unwrap(),
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn instance_buffer(device: &gpu::Device, capacity: usize) -> gpu::Buffer {
    device.create_buffer(&gpu::BufferDescriptor {
        label: Some("scene instances"),
        size: (capacity * size_of::<Instance>()) as u64,
        usage: gpu::BufferUsages::VERTEX | gpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn texture(
    device: &gpu::Device,
    [width, height]: [u32; 2],
    format: gpu::TextureFormat,
    sampled: bool,
    sample_count: u32,
) -> gpu::TextureView {
    let mut usage = gpu::TextureUsages::RENDER_ATTACHMENT;
    if sampled {
        usage |= gpu::TextureUsages::TEXTURE_BINDING;
    }
    device
        .create_texture(&gpu::TextureDescriptor {
            label: Some("viewport attachment"),
            size: gpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: gpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn color_attachment(
    view: &gpu::TextureView,
    clear: gpu::Color,
) -> gpu::RenderPassColorAttachment<'_> {
    gpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: gpu::Operations {
            load: gpu::LoadOp::Clear(clear),
            store: gpu::StoreOp::Store,
        },
    }
}

fn depth_attachment(
    view: &gpu::TextureView,
    store: gpu::StoreOp,
) -> gpu::RenderPassDepthStencilAttachment<'_> {
    gpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(gpu::Operations {
            load: gpu::LoadOp::Clear(1.0),
            store,
        }),
        stencil_ops: None,
    }
}

fn texture_layout(device: &gpu::Device, depth: bool) -> gpu::BindGroupLayout {
    let sample_type = if depth {
        gpu::TextureSampleType::Depth
    } else {
        gpu::TextureSampleType::Float { filterable: true }
    };
    let sampler = if depth {
        gpu::SamplerBindingType::Comparison
    } else {
        gpu::SamplerBindingType::Filtering
    };
    device.create_bind_group_layout(&gpu::BindGroupLayoutDescriptor {
        label: Some("sampled viewport texture"),
        entries: &[
            gpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: gpu::ShaderStages::FRAGMENT,
                ty: gpu::BindingType::Texture {
                    sample_type,
                    view_dimension: gpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            gpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: gpu::ShaderStages::FRAGMENT,
                ty: gpu::BindingType::Sampler(sampler),
                count: None,
            },
        ],
    })
}

fn texture_group(
    device: &gpu::Device,
    layout: &gpu::BindGroupLayout,
    view: &gpu::TextureView,
    sampler: &gpu::Sampler,
) -> gpu::BindGroup {
    device.create_bind_group(&gpu::BindGroupDescriptor {
        label: Some("viewport sampled view"),
        layout,
        entries: &[
            gpu::BindGroupEntry {
                binding: 0,
                resource: gpu::BindingResource::TextureView(view),
            },
            gpu::BindGroupEntry {
                binding: 1,
                resource: gpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn cube_vertices() -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(36);
    for normal in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ] {
        let tangent = if normal.y == 0.0 { Vec3::Y } else { Vec3::X };
        let bitangent = normal.cross(tangent);
        let corners = [
            (normal - tangent - bitangent) * 0.5,
            (normal + tangent - bitangent) * 0.5,
            (normal + tangent + bitangent) * 0.5,
            (normal - tangent + bitangent) * 0.5,
        ];
        vertices.extend([0, 1, 2, 0, 2, 3].map(|index| Vertex {
            position: corners[index].to_array(),
            normal: normal.to_array(),
        }));
    }
    vertices
}
