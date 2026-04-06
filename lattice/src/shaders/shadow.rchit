#version 460
#extension GL_EXT_ray_tracing : require

hitAttributeEXT vec3 hitNormal;

struct PrimaryPayload { vec3 normal; float t; int hit; };
layout(location = 0) rayPayloadInEXT PrimaryPayload primary;

void main() {
    primary.normal = normalize(hitNormal);
    primary.t      = gl_HitTEXT;
    primary.hit    = 1;
}