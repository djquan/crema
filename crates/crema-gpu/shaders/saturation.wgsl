@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    blend: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y { return; }

    let color = textureLoad(input, vec2<i32>(vec2(gid.xy)));
    let r = color.r; let g = color.g; let b = color.b;
    let blend = params.blend;

    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;

    textureStore(output, vec2<i32>(vec2(gid.xy)), vec4(
        max(y + blend * (r - y), 0.0),
        max(y + blend * (g - y), 0.0),
        max(y + blend * (b - y), 0.0),
        color.a
    ));
}
