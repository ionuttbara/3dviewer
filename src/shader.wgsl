struct Uniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    light: vec4<f32>,
    material: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var base_color_texture: texture_2d<f32>;
@group(0) @binding(2) var base_color_sampler: sampler;

struct VsIn { @location(0) position:vec3<f32>, @location(1) normal:vec3<f32>, @location(2) texcoord:vec2<f32>, };
struct VsOut { @builtin(position) position:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) world_pos:vec3<f32>, @location(2) texcoord:vec2<f32>, };

@vertex fn vs_main(v:VsIn)->VsOut{
    var o:VsOut;let world=u.model*vec4<f32>(v.position,1.0);o.position=u.view_proj*world;o.normal=normalize((u.model*vec4<f32>(v.normal,0.0)).xyz);o.world_pos=world.xyz;o.texcoord=v.texcoord;return o;
}

@fragment fn fs_main(i:VsOut)->@location(0) vec4<f32>{
    let n=normalize(i.normal);let l=normalize(u.light.xyz);let diffuse=max(dot(n,l),0.0);let rim=pow(1.0-abs(n.z),2.0)*0.10;
    var base=vec4<f32>(u.material.y,u.material.z,u.material.w,1.0);
    if(u.material.x>0.5){base=textureSample(base_color_texture,base_color_sampler,i.texcoord);}
    if(u.light.w>0.5){let selected_rgb=mix(base.rgb,vec3<f32>(1.0,0.62,0.16),0.10);base=vec4<f32>(selected_rgb,base.a);}
    let lit=base.rgb*(0.24+diffuse*0.76)+vec3<f32>(rim);return vec4<f32>(lit,base.a);
}

@fragment fn fs_edge(_i:VsOut)->@location(0) vec4<f32>{
    if(u.light.w>0.5){return vec4<f32>(1.0,0.62,0.16,1.0);}return vec4<f32>(0.025,0.035,0.050,1.0);
}
