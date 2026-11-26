struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return VertexOut(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), uv);
}

@fragment
fn fs_linear(i: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(scene, linear_sampler, i.uv);
}

@fragment
fn fs_srgb(i: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSample(scene, linear_sampler, i.uv).rgb;
    let rgb = select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
    return vec4<f32>(rgb, 1.0);
}
