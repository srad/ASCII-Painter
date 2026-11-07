struct Params {
    rect: vec4<f32>,
    viewport: vec4<f32>,
    clip: vec4<f32>,
    fade: vec4<u32>,
    encoding: vec4<f32>,
}
@group(0) @binding(0) var pixels: texture_2d<f32>;
@group(0) @binding(1) var sampling: sampler;
@group(0) @binding(2) var<uniform> p: Params;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Vertex {
    let uv = array<vec2<f32>, 6>(vec2(0.,0.), vec2(1.,0.), vec2(0.,1.), vec2(0.,1.), vec2(1.,0.), vec2(1.,1.))[i];
    let point = p.rect.xy + uv * p.rect.zw - p.viewport.zw;
    var out: Vertex;
    out.position = vec4(point.x / p.viewport.x * 2. - 1., 1. - point.y / p.viewport.y * 2., 0., 1.);
    out.uv = uv;
    return out;
}
fn linear(v: vec3<f32>) -> vec3<f32> {
    return select(pow((v + vec3(.055)) / 1.055, vec3(2.4)), v / 12.92, v <= vec3(.04045));
}
@fragment fn fs(input: Vertex) -> @location(0) vec4<f32> {
    var color = textureSample(pixels, sampling, input.uv);
    if p.encoding.x < .5 && color.a > 0. { color = vec4(linear(color.rgb / color.a) * color.a, color.a); }
    return color * clip_alpha(input.position.xy + p.viewport.zw, p.clip, p.fade);
}
