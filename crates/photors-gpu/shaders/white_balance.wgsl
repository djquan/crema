@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    r_mult: f32,
    g_mult: f32,
    b_mult: f32,
    _pad: f32,
}

@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let color = textureLoad(input, vec2<i32>(gid.xy));
    let adjusted = vec4<f32>(
        color.r * params.r_mult,
        color.g * params.g_mult,
        color.b * params.b_mult,
        color.a,
    );
    textureStore(output, vec2<i32>(gid.xy), adjusted);
}
