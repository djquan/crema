@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    vignette_strength: f32,
    distortion_k: f32,
    center_x: f32,
    center_y: f32,
    r_max: f32,
    width: f32,
    height: f32,
    _padding: f32,
}

@group(0) @binding(2) var<uniform> params: Params;

fn bilinear_sample(pos: vec2<f32>, dims: vec2<i32>) -> vec4<f32> {
    let clamped = clamp(pos, vec2<f32>(0.0), vec2<f32>(f32(dims.x - 1), f32(dims.y - 1)));
    let x0 = i32(floor(clamped.x));
    let y0 = i32(floor(clamped.y));
    let x1 = min(x0 + 1, dims.x - 1);
    let y1 = min(y0 + 1, dims.y - 1);
    let fx = clamped.x - f32(x0);
    let fy = clamped.y - f32(y0);

    let c00 = textureLoad(input, vec2<i32>(x0, y0));
    let c10 = textureLoad(input, vec2<i32>(x1, y0));
    let c01 = textureLoad(input, vec2<i32>(x0, y1));
    let c11 = textureLoad(input, vec2<i32>(x1, y1));

    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    return c00 * w00 + c10 * w10 + c01 * w01 + c11 * w11;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(i32(params.width), i32(params.height));
    if i32(gid.x) >= dims.x || i32(gid.y) >= dims.y {
        return;
    }

    let x = f32(gid.x);
    let y = f32(gid.y);
    var src = vec2<f32>(x, y);

    if params.distortion_k != 0.0 {
        let nx = (x - params.center_x) / params.r_max;
        let ny = (y - params.center_y) / params.r_max;
        let r = sqrt(nx * nx + ny * ny);
        if r > 1e-8 {
            let r_corrected = r * (1.0 + params.distortion_k * r * r);
            let scale = r_corrected / r;
            src = vec2<f32>(
                params.center_x + nx * scale * params.r_max,
                params.center_y + ny * scale * params.r_max,
            );
        }
    }

    var color = bilinear_sample(src, dims);

    if params.vignette_strength != 0.0 {
        let dx = x / params.width - 0.5;
        let dy = y / params.height - 0.5;
        let d2 = dx * dx + dy * dy;
        let factor = 1.0 - params.vignette_strength * d2;
        color = vec4<f32>(color.rgb * factor, color.a);
    }

    textureStore(output, vec2<i32>(gid.xy), color);
}
