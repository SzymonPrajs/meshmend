struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
    material: vec4<f32>,
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
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(1) @binding(1) var<uniform> chunk: Chunk;

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
    out.clip_position = camera.view_proj * vec4<f32>(position.xyz, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let light = normalize(-camera.light_dir.xyz);
    let diffuse = max(dot(normal, light), 0.0);
    let view_dir = normalize(camera.eye.xyz - in.world_position);
    let half_dir = normalize(light + view_dir);
    let specular = pow(max(dot(normal, half_dir), 0.0), 32.0) * 0.18;
    let ambient = 0.28;
    let clay = camera.material.rgb * (ambient + diffuse * 0.72) + vec3<f32>(specular);
    return vec4<f32>(clay, 1.0);
}
