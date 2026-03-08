// Unsharp mask: sharpened = original + amount * (original - blurred)
@group(0) @binding(0) var original: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var blurred: texture_storage_2d<rgba32float, read>;
@group(0) @binding(2) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    amount: f32,
}
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(original);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let orig = textureLoad(original, gid.xy);
    let blur = textureLoad(blurred, gid.xy);
    let sharp = orig.rgb + params.amount * (orig.rgb - blur.rgb);
    textureStore(output, gid.xy, vec4<f32>(max(sharp, vec3<f32>(0.0)), 1.0));
}
