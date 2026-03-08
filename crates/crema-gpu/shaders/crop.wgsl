@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    src_x: u32,
    src_y: u32,
    sin_angle: f32,
    cos_angle: f32,
}

@group(0) @binding(2) var<uniform> params: Params;

fn bilinear_sample(x: f32, y: f32, dims: vec2<u32>) -> vec4<f32> {
    let cx = clamp(x, 0.0, f32(dims.x) - 1.0);
    let cy = clamp(y, 0.0, f32(dims.y) - 1.0);

    let x0 = u32(cx);
    let y0 = u32(cy);
    let x1 = min(x0 + 1u, dims.x - 1u);
    let y1 = min(y0 + 1u, dims.y - 1u);

    let fx = cx - f32(x0);
    let fy = cy - f32(y0);

    let c00 = textureLoad(input, vec2<i32>(vec2(x0, y0)));
    let c10 = textureLoad(input, vec2<i32>(vec2(x1, y0)));
    let c01 = textureLoad(input, vec2<i32>(vec2(x0, y1)));
    let c11 = textureLoad(input, vec2<i32>(vec2(x1, y1)));

    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    return c00 * w00 + c10 * w10 + c01 * w01 + c11 * w11;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_dims = textureDimensions(output);
    if gid.x >= out_dims.x || gid.y >= out_dims.y { return; }

    let in_dims = textureDimensions(input);
    let cx = f32(in_dims.x) * 0.5;
    let cy = f32(in_dims.y) * 0.5;

    let px = f32(gid.x + params.src_x) + 0.5;
    let py = f32(gid.y + params.src_y) + 0.5;

    let rx = cx + params.cos_angle * (px - cx) - params.sin_angle * (py - cy) - 0.5;
    let ry = cy + params.sin_angle * (px - cx) + params.cos_angle * (py - cy) - 0.5;

    let color = bilinear_sample(rx, ry, in_dims);
    textureStore(output, vec2<i32>(vec2(gid.xy)), color);
}
