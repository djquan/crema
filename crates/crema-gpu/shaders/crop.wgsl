@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    src_x: u32,
    src_y: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_dims = textureDimensions(output);
    if gid.x >= out_dims.x || gid.y >= out_dims.y { return; }

    let src = vec2<i32>(vec2(gid.x + params.src_x, gid.y + params.src_y));
    let color = textureLoad(input, src);
    textureStore(output, vec2<i32>(vec2(gid.xy)), color);
}
