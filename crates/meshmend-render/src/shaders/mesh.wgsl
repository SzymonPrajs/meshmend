struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
    material: vec4<f32>,
    clip_plane: vec4<f32>,
    settings: vec4<u32>,
    view: vec4<u32>,
};

struct Chunk {
    chunk_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct Triangle {
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    normal: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) barycentric: vec3<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(1) @binding(1) var<uniform> chunk: Chunk;

fn shade_surface(base: vec3<f32>, normal: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    if (camera.view.z == 2u) {
        let key = normalize(vec3<f32>(0.42, 0.72, 0.55));
        let fill = normalize(vec3<f32>(-0.74, 0.28, 0.36));
        let rim = normalize(vec3<f32>(-0.18, -0.42, -0.88));
        let diffuse =
            max(dot(normal, key), 0.0) * 0.46 +
            max(dot(normal, fill), 0.0) * 0.26 +
            max(dot(normal, rim), 0.0) * 0.22;
        let half_dir = normalize(key + view_dir);
        let specular = pow(max(dot(normal, half_dir), 0.0), 36.0) * 0.12;
        return base * (0.34 + diffuse) + vec3<f32>(specular);
    }

    let light = normalize(-camera.light_dir.xyz);
    let diffuse = max(dot(normal, light), 0.0);
    let half_dir = normalize(light + view_dir);
    let specular = pow(max(dot(normal, half_dir), 0.0), 32.0) * 0.18;
    return base * (0.34 + diffuse * 0.70) + vec3<f32>(specular);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOut {
    let triangle = triangles[instance_index];
    var position = triangle.p0;
    if (vertex_index == 1u) {
        position = triangle.p1;
    } else if (vertex_index == 2u) {
        position = triangle.p2;
    }

    var out: VertexOut;
    out.world_position = position.xyz;
    out.normal = normalize(triangle.normal.xyz);
    out.barycentric = vec3<f32>(0.0);
    if (vertex_index == 0u) {
        out.barycentric = vec3<f32>(1.0, 0.0, 0.0);
    } else if (vertex_index == 1u) {
        out.barycentric = vec3<f32>(0.0, 1.0, 0.0);
    } else {
        out.barycentric = vec3<f32>(0.0, 0.0, 1.0);
    }
    out.clip_position = camera.view_proj * vec4<f32>(position.xyz, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (camera.settings.z == 1u && dot(in.world_position, camera.clip_plane.xyz) < camera.clip_plane.w) {
        discard;
    }

    let normal = normalize(in.normal);
    if (camera.settings.y == 1u) {
        return vec4<f32>(normal * 0.5 + vec3<f32>(0.5), 1.0);
    }

    let view_dir = normalize(camera.eye.xyz - in.world_position);
    var clay = shade_surface(camera.material.rgb, normal, view_dir);
    var alpha = 1.0;

    if (camera.view.x == 1u || camera.view.y == 1u) {
        clay = mix(clay, vec3<f32>(0.68, 0.78, 0.78), 0.28);
        alpha = 0.34;
    }

    if (camera.view.y == 1u) {
        alpha = 0.24;
    }

    if (camera.settings.x == 1u) {
        let edge = min(min(in.barycentric.x, in.barycentric.y), in.barycentric.z);
        let width = max(fwidth(edge) * select(1.35, 1.9, camera.view.y == 1u), 0.0001);
        let wire = 1.0 - smoothstep(0.0, width, edge);
        let wire_color = select(vec3<f32>(0.05, 0.07, 0.08), vec3<f32>(0.18, 0.92, 1.0), camera.view.y == 1u);
        clay = mix(clay, wire_color, wire);
        if (camera.view.y == 1u) {
            alpha = max(alpha, wire * 0.86);
        }
    }

    return vec4<f32>(clay, alpha);
}
