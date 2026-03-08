@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    // Hue rotation matrix (3x3 packed as 3 vec4s)
    row0: vec4<f32>,
    row1: vec4<f32>,
    row2: vec4<f32>,
    // x: sat_blend, y: light_scale, z: do_hue, w: do_sat
    controls: vec4<f32>,
}

@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y { return; }

    let color = textureLoad(input, vec2<i32>(vec2(gid.xy)));
    var r = color.r; var g = color.g; var b = color.b;

    // Hue rotation
    if params.controls.z > 0.5 {
        let rgb = vec3(r, g, b);
        r = dot(params.row0.xyz, rgb);
        g = dot(params.row1.xyz, rgb);
        b = dot(params.row2.xyz, rgb);
    }

    // Saturation
    if params.controls.w > 0.5 {
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let sat_blend = params.controls.x;
        r = y + sat_blend * (r - y);
        g = y + sat_blend * (g - y);
        b = y + sat_blend * (b - y);
    }

    // Lightness
    let light_scale = params.controls.y;
    if light_scale != 1.0 {
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if y > 0.0 {
            let scale = (y * light_scale) / y;
            r *= scale;
            g *= scale;
            b *= scale;
        }
    }

    textureStore(output, vec2<i32>(vec2(gid.xy)), vec4(
        max(r, 0.0), max(g, 0.0), max(b, 0.0), color.a
    ));
}
