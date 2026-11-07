#version 440

layout(location = 0) in vec2 texCoord;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
};

layout(binding = 1) uniform sampler2D yTex;
layout(binding = 2) uniform sampler2D uvTex;

void main() {
    float y = texture(yTex, texCoord).r;
    vec2 uv = texture(uvTex, texCoord).rg - vec2(0.5, 0.5);
    float r = y + 1.402 * uv.y;
    float g = y - 0.344136 * uv.x - 0.714136 * uv.y;
    float b = y + 1.772 * uv.x;
    fragColor = vec4(r, g, b, 1.0) * qt_Opacity;
}
