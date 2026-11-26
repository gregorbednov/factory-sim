struct Frame {
    camera: mat4x4<f32>,
    light: mat4x4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    viewport: vec4<f32>,
}

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) center: vec4<f32>,
    @location(3) axis_x: vec4<f32>,
    @location(4) axis_y: vec4<f32>,
    @location(5) axis_z: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) surface: vec4<f32>,
}

struct Fragment {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) local: vec3<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) surface: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;
@group(2) @binding(0) var reflection: texture_2d<f32>;
@group(2) @binding(1) var reflection_sampler: sampler;

fn world(v: Vertex) -> vec3<f32> {
    return v.center.xyz + v.axis_x.xyz * v.position.x
        + v.axis_y.xyz * v.position.y + v.axis_z.xyz * v.position.z;
}

fn surface_vertex(v: Vertex, point: vec3<f32>) -> Fragment {
    let x = v.axis_x.xyz;
    let y = v.axis_y.xyz;
    let z = v.axis_z.xyz;
    let normal = normalize(x * (v.normal.x / dot(x, x))
        + y * (v.normal.y / dot(y, y)) + z * (v.normal.z / dot(z, z)));
    let position = frame.camera * vec4<f32>(point, 1.0);
    let local = v.position * vec3<f32>(length(v.axis_x.xyz), length(v.axis_y.xyz), length(v.axis_z.xyz));
    return Fragment(position, world(v), normal, local, v.color, v.surface);
}

@vertex
fn vs_scene(v: Vertex) -> Fragment {
    return surface_vertex(v, world(v));
}

@vertex
fn vs_reflection(v: Vertex) -> Fragment {
    return surface_vertex(v, world(v) * vec3<f32>(1.0, -1.0, 1.0));
}

@vertex
fn vs_shadow(v: Vertex) -> @builtin(position) vec4<f32> {
    return frame.light * vec4<f32>(world(v), 1.0);
}

fn shadow_at(point: vec3<f32>, normal: vec3<f32>) -> f32 {
    let ndc = frame.light * vec4<f32>(point + normal * 0.013, 1.0);
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    let unfaded = 1.0 - smoothstep(0.44, 0.5, max(abs(uv.x - 0.5), abs(uv.y - 0.5)));
    return mix(1.0, textureSampleCompareLevel(shadow_map, shadow_sampler, uv, ndc.z - 0.00035), unfaded);
}

fn grain(point: vec3<f32>) -> f32 {
    let p = fract(point * 0.1031);
    let q = p + dot(p, p.yxz + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn seam(coordinate: f32, width: f32) -> f32 {
    let distance = abs(fract(coordinate + 0.5) - 0.5);
    let fade = max(fwidth(coordinate), 0.0001);
    return 1.0 - smoothstep(width, width + fade, distance);
}

fn lit(i: Fragment, to_eye: vec3<f32>) -> vec3<f32> {
    let n = normalize(i.normal);
    let light = frame.sun.xyz;
    let half_vector = normalize(light + to_eye);
    let facing = max(dot(n, to_eye), 0.0);
    let lambert = max(dot(n, light), 0.0);
    let roughness = i.surface.x;
    let metal = i.surface.y;
    let material = i.surface.z;
    let footprint = max(length(dpdx(i.world)), length(dpdy(i.world)));
    let random = mix(grain(floor(i.world * 160.0)), 0.5, smoothstep(0.008, 0.035, footprint));
    let floor_grid = max(seam(i.world.x / 2.5, 0.002), seam(i.world.z / 2.5, 0.002));
    let belt_seam = seam((i.local.x - i.color.w) / 0.25, 0.014);
    var base = i.color.rgb;
    if material == 1.0 {
        base *= (0.94 + (random - 0.5) * 0.12) * (1.0 - floor_grid * 0.17);
    } else if material == 2.0 {
        base *= 1.0 - belt_seam * 0.27 + (random - 0.5) * 0.09;
    } else if material == 3.0 {
        base *= 0.98 + (random - 0.5) * 0.22;
    } else {
        base *= 1.0 + (random - 0.5) * 0.045;
    }

    var visibility = 1.0;
    if frame.eye.w > 0.5 {
        visibility = mix(0.24, 1.0, shadow_at(i.world, n));
    }
    let fill = max(dot(n, normalize(vec3<f32>(0.62, 0.48, -0.48))), 0.0);
    let ambient = 0.26 + n.y * 0.10;
    let f0 = mix(vec3<f32>(0.045), base, metal);
    let specular = pow(max(dot(n, half_vector), 0.0), mix(120.0, 5.0, roughness));
    let reflected = reflect(-to_eye, n);
    let sky = smoothstep(-0.2, 0.9, reflected.y);
    let strip = pow(max(1.0 - abs(reflected.x), 0.0), 15.0) * sky;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - facing, 5.0);
    let environment = mix(vec3<f32>(0.075, 0.09, 0.11), vec3<f32>(0.38, 0.46, 0.55), sky)
        + vec3<f32>(0.46, 0.54, 0.60) * strip;
    return base * (ambient + lambert * visibility * vec3<f32>(1.05, 0.94, 0.80) + fill * vec3<f32>(0.18, 0.24, 0.32))
        * (1.0 - metal * 0.73)
        + f0 * specular * visibility * 1.8
        + fresnel * environment * (0.54 + metal * 0.48)
        + base * i.surface.w;
}

@fragment
fn fs_reflection(i: Fragment) -> @location(0) vec4<f32> {
    let rgb = lit(i, frame.eye.xyz * vec3<f32>(1.0, -1.0, 1.0));
    if i.world.y < 0.0 { discard; }
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn fs_scene(i: Fragment) -> @location(0) vec4<f32> {
    var rgb = lit(i, frame.eye.xyz);
    if i.surface.z == 1.0 && frame.sun.w > 0.5 {
        let uv = i.position.xy * frame.viewport.zw;
        let reflected = textureSampleLevel(reflection, reflection_sampler, uv, 0.0);
        let fresnel = pow(1.0 - clamp(frame.eye.y, 0.0, 1.0), 3.0);
        rgb = mix(rgb, reflected.rgb, reflected.a * (0.10 + 0.24 * fresnel));
    }
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn fs_window(i: Fragment) -> @location(0) vec4<f32> {
    let grazing = 1.0 - abs(dot(normalize(i.normal), frame.eye.xyz));
    let sheen = grazing * grazing;
    return vec4<f32>(i.color.rgb + vec3<f32>(0.15 * sheen), 0.20 + 0.20 * sheen);
}
