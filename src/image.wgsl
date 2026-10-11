struct Mapping {
    bounds: vec4<f32>,
    source_x: vec4<f32>,
    source_y: vec4<f32>,
    other_x: vec4<f32>,
    other_y: vec4<f32>,
    // x = level blend weight; yzw = backdrop RGB in linear light.
    blend: vec4<f32>,
}
@group(0) @binding(0) var<uniform> mapping: Mapping;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
@group(0) @binding(3) var other: texture_2d<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) other_uv: vec2<f32>,
}
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let coordinates = array<vec2<f32>, 6>(vec2(0, 0), vec2(1, 0), vec2(0, 1), vec2(0, 1), vec2(1, 0), vec2(1, 1));
    let uv = coordinates[index];
    var result: Vertex;
    result.position = vec4(mix(mapping.bounds.xy, mapping.bounds.zw, uv), 0, 1);
    let coordinate = vec3(uv, 1);
    result.uv = vec2(dot(mapping.source_x.xyz, coordinate), dot(mapping.source_y.xyz, coordinate));
    result.other_uv = vec2(dot(mapping.other_x.xyz, coordinate), dot(mapping.other_y.xyz, coordinate));
    return result;
}
fn composite(input: Vertex) -> vec3<f32> {
    let a = textureSample(image, image_sampler, input.uv);
    let b = textureSample(other, image_sampler, input.other_uv);
    let color = mix(a, b, mapping.blend.x);
    // RGB is already premultiplied in linear light. Never multiply alpha again.
    return color.rgb + mapping.blend.yzw * (1.0 - color.a);
}
@fragment fn linear(input: Vertex) -> @location(0) vec4<f32> {
    return vec4(composite(input), 1);
}
@fragment fn encoded(input: Vertex) -> @location(0) vec4<f32> {
    let rgb = composite(input);
    let encoded = select(1.055 * pow(rgb, vec3(1.0 / 2.4)) - 0.055, 12.92 * rgb, rgb <= vec3(0.0031308));
    return vec4(encoded, 1);
}
