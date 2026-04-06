#version 460
#extension GL_EXT_ray_tracing : require

struct PrimaryPayload { vec3 normal; float t; int hit; };
layout(location = 0) rayPayloadInEXT PrimaryPayload primary;

void main() { primary.hit = 0; }