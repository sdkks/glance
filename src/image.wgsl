@group(0) @binding(0) var<uniform> bounds: vec4<f32>;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let coordinates = array<vec2<f32>, 6>(vec2(0, 0), vec2(1, 0), vec2(0, 1), vec2(0, 1), vec2(1, 0), vec2(1, 1));
    let uv = coordinates[index];
    var result: Vertex;
    result.position = vec4(mix(bounds.xy, bounds.zw, uv), 0, 1);
    result.uv = uv;
    return result;
}

fn composite(uv: vec2<f32>) -> vec3<f32> {
    let color = textureSample(image, image_sampler, uv);
    // Interim SDR RGBA8 surface, over the same dark canvas in linear light.
    return color.rgb * color.a + vec3(0.006048833) * (1.0 - color.a);
}

@fragment fn linear(input: Vertex) -> @location(0) vec4<f32> {
    return vec4(composite(input.uv), 1);
}

@fragment fn encoded(input: Vertex) -> @location(0) vec4<f32> {
    let rgb = composite(input.uv);
    let encoded = select(1.055 * pow(rgb, vec3(1.0 / 2.4)) - 0.055, 12.92 * rgb, rgb <= vec3(0.0031308));
    return vec4(encoded, 1);
}
