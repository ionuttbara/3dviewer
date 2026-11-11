use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use glam::camera::rh::{proj::directx::{orthographic, perspective}, view::look_at_mat4};
use glam::{Mat4, Vec3, Vec4};
use slint::wgpu_30::wgpu;
use wgpu::util::DeviceExt;

use crate::mesh::{SceneDocument, SceneMesh, Vertex};

#[derive(Clone, Debug)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Camera {
    const VERTICAL_FOV_RADIANS: f32 = std::f32::consts::FRAC_PI_4;
    const FIT_MARGIN: f32 = 1.34;

    fn fit_distance(radius: f32, aspect: f32) -> f32 {
        let radius = radius.max(0.001);
        let aspect = aspect.clamp(0.10, 10.0);
        let half_v = Self::VERTICAL_FOV_RADIANS * 0.5;
        let half_h = (half_v.tan() * aspect).atan();
        let limiting_half_fov = half_v.min(half_h).max(1.0_f32.to_radians());
        (radius / limiting_half_fov.sin() * Self::FIT_MARGIN).max(0.01)
    }

    pub fn fit_scene(scene: &SceneDocument, aspect: f32) -> Self {
        Self {
            target: scene.center(),
            yaw: 0.72,
            pitch: 0.42,
            distance: Self::fit_distance(scene.radius(), aspect),
        }
    }

    pub fn fit_mesh(mesh: &SceneMesh, aspect: f32) -> Self {
        Self {
            target: mesh.center(),
            yaw: 0.72,
            pitch: 0.42,
            distance: Self::fit_distance(mesh.radius(), aspect),
        }
    }

    fn orbit_direction(&self) -> Vec3 {
        Vec3::new(self.yaw.cos()*self.pitch.cos(), self.pitch.sin(), self.yaw.sin()*self.pitch.cos()).normalize_or_zero()
    }

    pub fn view_proj(&self, aspect:f32, orthographic_2d:bool)->Mat4 {
        let aspect=aspect.clamp(0.10,10.0);
        let distance=self.distance.max(0.0001);
        let dir=self.orbit_direction();
        let eye=self.target+dir*distance;
        let view=look_at_mat4(eye,self.target,Vec3::Y);
        let near=(distance/1000.0).max(0.0001);
        let far=(distance*100.0).max(100.0);
        let proj=if orthographic_2d {
            // Match the apparent scale of the perspective camera at its target plane.
            // This makes switching 3D <-> 2D predictable and lets the same zoom model work.
            let half_h=(distance*(Self::VERTICAL_FOV_RADIANS*0.5).tan()).max(0.0001);
            let half_w=(half_h*aspect).max(0.0001);
            orthographic(-half_w,half_w,-half_h,half_h,near,far)
        } else {
            perspective(Self::VERTICAL_FOV_RADIANS,aspect,near,far)
        };
        proj*view
    }

    pub fn pan(&mut self,dx_pixels:f32,dy_pixels:f32){
        let eye_dir=self.orbit_direction(); let view_dir=-eye_dir; let right=view_dir.cross(Vec3::Y).normalize_or_zero(); let up=right.cross(view_dir).normalize_or_zero();
        let world_per_pixel=(self.distance*0.0016).max(0.00001);
        self.target+=right*(-dx_pixels*world_per_pixel); self.target+=up*(dy_pixels*world_per_pixel);
    }
}

pub struct SharedRenderState {
    pub scene: SceneDocument,
    pub camera: Camera,
    pub generation: u64,
    pub wireframe: bool,
    pub show_edges: bool,
    pub use_textures: bool,
    pub selected_index: usize,
    pub viewport_aspect: f32,
    pub mode_2d: bool,
    zoom_reference_distance: f32,
}

impl SharedRenderState {
    pub const MIN_ZOOM_PERCENT:f32=10.0;
    pub const MAX_ZOOM_PERCENT:f32=800.0;

    pub fn new(scene:SceneDocument)->Self{
        let viewport_aspect=16.0/10.0;
        let camera=Camera::fit_scene(&scene,viewport_aspect);
        let zoom_reference_distance=camera.distance;
        Self{scene,camera,generation:1,wireframe:false,show_edges:true,use_textures:true,selected_index:0,viewport_aspect,mode_2d:false,zoom_reference_distance}
    }
    pub fn set_viewport_aspect(&mut self,aspect:f32){if aspect.is_finite()&&aspect>0.01{self.viewport_aspect=aspect.clamp(0.10,10.0);}}
    pub fn set_scene(&mut self,scene:SceneDocument){self.camera=Camera::fit_scene(&scene,self.viewport_aspect);self.scene=scene;if self.mode_2d{self.front();}self.zoom_reference_distance=self.camera.distance;self.selected_index=0;self.generation=self.generation.wrapping_add(1).max(1);}
    pub fn fit(&mut self){self.camera=Camera::fit_scene(&self.scene,self.viewport_aspect);if self.mode_2d{self.front();}self.zoom_reference_distance=self.camera.distance;}
    pub fn fit_selected(&mut self){if let Some(object)=self.scene.objects.get(self.selected_index){self.camera=Camera::fit_mesh(&object.mesh,self.viewport_aspect);if self.mode_2d{self.front();}self.zoom_reference_distance=self.camera.distance;}}
    pub fn select(&mut self,index:usize){if index<self.scene.objects.len(){self.selected_index=index;}}
    pub fn toggle_visible(&mut self,index:usize){if let Some(o)=self.scene.objects.get_mut(index){o.visible=!o.visible;}}
    pub fn isolate(&mut self,index:usize){if index<self.scene.objects.len(){for (i,o) in self.scene.objects.iter_mut().enumerate(){o.visible=i==index;} self.selected_index=index;}}
    pub fn show_all(&mut self){for o in &mut self.scene.objects{o.visible=true;}}
    pub fn pick(&mut self,x_px:f32,y_px:f32,width:f32,height:f32)->Option<usize>{
        if width<=1.0||height<=1.0{return None;}
        let aspect=(width/height).clamp(0.10,10.0);let inv=self.camera.view_proj(aspect,self.mode_2d).inverse();
        let ndc_x=x_px/width*2.0-1.0;let ndc_y=1.0-y_px/height*2.0;
        let near_h=inv*Vec4::new(ndc_x,ndc_y,0.0,1.0);let far_h=inv*Vec4::new(ndc_x,ndc_y,1.0,1.0);
        if near_h.w.abs()<1.0e-8||far_h.w.abs()<1.0e-8{return None;}
        let origin=near_h.truncate()/near_h.w;let far=far_h.truncate()/far_h.w;let dir=(far-origin).normalize_or_zero();
        if dir.length_squared()<1.0e-10{return None;}
        let mut best:Option<(usize,f32)>=None;
        for (object_index,object) in self.scene.objects.iter().enumerate(){
            if !object.visible{continue;}
            if !ray_hits_aabb(origin,dir,object.mesh.bounds_min,object.mesh.bounds_max){continue;}
            for tri in object.mesh.indices.chunks_exact(3){
                let a=Vec3::from_array(object.mesh.vertices[tri[0] as usize].position);
                let b=Vec3::from_array(object.mesh.vertices[tri[1] as usize].position);
                let c=Vec3::from_array(object.mesh.vertices[tri[2] as usize].position);
                if let Some(t)=ray_triangle(origin,dir,a,b,c){if t>=0.0&&best.map(|(_,bt)|t<bt).unwrap_or(true){best=Some((object_index,t));}}
            }
        }
        if let Some((index,_))=best{self.selected_index=index;Some(index)}else{None}
    }
    pub fn rotate(&mut self,yaw_delta:f32,pitch_delta:f32){if self.mode_2d{return;}self.camera.yaw+=yaw_delta;self.camera.pitch=(self.camera.pitch+pitch_delta).clamp(-1.5533,1.5533);}
    pub fn pan(&mut self,dx:f32,dy:f32){self.camera.pan(dx,dy);}
    pub fn zoom(&mut self,factor:f32){
        if !factor.is_finite()||factor<=0.0{return;}
        let min_distance=(self.zoom_reference_distance*100.0/Self::MAX_ZOOM_PERCENT).max(0.0001);
        let max_distance=(self.zoom_reference_distance*100.0/Self::MIN_ZOOM_PERCENT).max(min_distance*2.0);
        self.camera.distance=(self.camera.distance*factor).clamp(min_distance,max_distance);
    }
    pub fn zoom_percent(&self)->f32{
        if !self.camera.distance.is_finite()||self.camera.distance<=0.0{return 100.0;}
        (self.zoom_reference_distance/self.camera.distance*100.0).clamp(Self::MIN_ZOOM_PERCENT,Self::MAX_ZOOM_PERCENT)
    }
    pub fn set_zoom_percent(&mut self,percent:f32){
        let percent=if percent.is_finite(){percent.clamp(Self::MIN_ZOOM_PERCENT,Self::MAX_ZOOM_PERCENT)}else{100.0};
        self.camera.distance=(self.zoom_reference_distance*100.0/percent).max(0.0001);
    }
    pub fn set_2d_mode(&mut self,enabled:bool){
        if self.mode_2d==enabled{return;}
        self.mode_2d=enabled;
        if enabled{self.camera=Camera::fit_scene(&self.scene,self.viewport_aspect);self.front();}
        else{self.camera=Camera::fit_scene(&self.scene,self.viewport_aspect);self.iso();}
        self.zoom_reference_distance=self.camera.distance;
    }
    pub fn front(&mut self){self.camera.yaw=std::f32::consts::PI;self.camera.pitch=0.0;}
    pub fn back(&mut self){self.camera.yaw=0.0;self.camera.pitch=0.0;}
    pub fn right(&mut self){self.camera.yaw=-std::f32::consts::FRAC_PI_2;self.camera.pitch=0.0;}
    pub fn left(&mut self){self.camera.yaw=std::f32::consts::FRAC_PI_2;self.camera.pitch=0.0;}
    pub fn top(&mut self){self.camera.pitch=1.5533;self.camera.yaw=0.0;}
    pub fn bottom(&mut self){self.camera.pitch=-1.5533;self.camera.yaw=0.0;}
    pub fn iso(&mut self){self.camera.yaw=0.72;self.camera.pitch=0.42;}
}

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Uniforms { view_proj:[[f32;4];4], model:[[f32;4];4], light:[f32;4], material:[f32;4] }

struct GpuObject {
    vertex:wgpu::Buffer,
    index:wgpu::Buffer,
    count:u32,
    edge_index:wgpu::Buffer,
    edge_count:u32,
    uniform:wgpu::Buffer,
    bind_group:wgpu::BindGroup,
    _material_texture:wgpu::Texture,
    material_loaded:bool,
}

pub struct Renderer {
    device:wgpu::Device,
    queue:wgpu::Queue,
    solid_pipeline:wgpu::RenderPipeline,
    edge_pipeline:wgpu::RenderPipeline,
    bind_group_layout:wgpu::BindGroupLayout,
    sampler:wgpu::Sampler,
    texture:wgpu::Texture,
    depth:wgpu::Texture,
    target_width:u32,
    target_height:u32,
    gpu_objects:Vec<GpuObject>,
    uploaded_generation:u64,
}

impl Renderer {
    pub fn new(device:&wgpu::Device,queue:&wgpu::Queue,width:u32,height:u32)->Self{
        let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("model shader"),source:wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into())});
        let bind_group_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("scene bgl"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX|wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
        ]});
        let sampler=device.create_sampler(&wgpu::SamplerDescriptor{label:Some("material sampler"),address_mode_u:wgpu::AddressMode::Repeat,address_mode_v:wgpu::AddressMode::Repeat,address_mode_w:wgpu::AddressMode::Repeat,mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,mipmap_filter:wgpu::MipmapFilterMode::Linear,..Default::default()});
        let layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("model layout"),bind_group_layouts:&[Some(&bind_group_layout)],immediate_size:0});
        let vertex_layout=wgpu::VertexBufferLayout{array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&[
            wgpu::VertexAttribute{offset:0,shader_location:0,format:wgpu::VertexFormat::Float32x3},
            wgpu::VertexAttribute{offset:12,shader_location:1,format:wgpu::VertexFormat::Float32x3},
            wgpu::VertexAttribute{offset:24,shader_location:2,format:wgpu::VertexFormat::Float32x2},
        ]};
        let make_pipeline=|label:&'static str,topology:wgpu::PrimitiveTopology,cull_mode:Option<wgpu::Face>,fragment_entry:&'static str,depth_write:bool,depth_compare:wgpu::CompareFunction|{
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some(label),layout:Some(&layout),
                vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs_main"),buffers:&[Some(vertex_layout.clone())],compilation_options:Default::default()},
                fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some(fragment_entry),targets:&[Some(wgpu::ColorTargetState{format:wgpu::TextureFormat::Rgba8UnormSrgb,blend:Some(wgpu::BlendState::REPLACE),write_mask:wgpu::ColorWrites::ALL})],compilation_options:Default::default()}),
                primitive:wgpu::PrimitiveState{topology,strip_index_format:None,front_face:wgpu::FrontFace::Ccw,cull_mode,unclipped_depth:false,polygon_mode:wgpu::PolygonMode::Fill,conservative:false},
                depth_stencil:Some(wgpu::DepthStencilState{format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(depth_write),depth_compare:Some(depth_compare),stencil:Default::default(),bias:Default::default()}),
                multisample:Default::default(),multiview_mask:None,cache:None})
        };
        let solid_pipeline=make_pipeline("solid pipeline",wgpu::PrimitiveTopology::TriangleList,Some(wgpu::Face::Back),"fs_main",true,wgpu::CompareFunction::Less);
        let edge_pipeline=make_pipeline("edge pipeline",wgpu::PrimitiveTopology::LineList,None,"fs_edge",false,wgpu::CompareFunction::LessEqual);
        let target_width=width.clamp(1,8192);let target_height=height.clamp(1,8192);let texture=create_target(device,target_width,target_height);let depth=create_depth(device,target_width,target_height);
        Self{device:device.clone(),queue:queue.clone(),solid_pipeline,edge_pipeline,bind_group_layout,sampler,texture,depth,target_width,target_height,gpu_objects:Vec::new(),uploaded_generation:0}
    }

    pub fn output_texture(&self) -> wgpu::Texture { self.texture.clone() }

    /// Resize the off-screen viewport target when the Slint viewport changes.
    /// Returns a clone of the new color texture so the UI can import it before
    /// Slint renders the current frame.
    pub fn ensure_targets(&mut self,width:u32,height:u32)->Option<wgpu::Texture>{
        let width=width.clamp(1,8192);
        let height=height.clamp(1,8192);
        if width==self.target_width&&height==self.target_height{return None;}
        self.target_width=width;
        self.target_height=height;
        self.texture=create_target(&self.device,width,height);
        self.depth=create_depth(&self.device,width,height);
        Some(self.texture.clone())
    }

    fn upload_scene(&mut self,state:&SharedRenderState){
        self.gpu_objects.clear();
        for (object_index,object) in state.scene.objects.iter().enumerate(){
            let mesh=&object.mesh;
            let mut edges=Vec::<u32>::with_capacity(mesh.indices.len()*2);for t in mesh.indices.chunks_exact(3){edges.extend_from_slice(&[t[0],t[1],t[1],t[2],t[2],t[0]]);}
            let vertex=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("mesh vertices"),contents:bytemuck::cast_slice(&mesh.vertices),usage:wgpu::BufferUsages::VERTEX});
            let index=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("mesh indices"),contents:bytemuck::cast_slice(&mesh.indices),usage:wgpu::BufferUsages::INDEX});
            let edge_index=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("mesh edges"),contents:bytemuck::cast_slice(&edges),usage:wgpu::BufferUsages::INDEX});
            let uniform=self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("object uniforms"),size:std::mem::size_of::<Uniforms>() as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let (material_texture,material_loaded)=load_material_texture(&self.device,&self.queue,mesh,object_index);
            let view=material_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group=create_scene_bind_group(&self.device,&self.bind_group_layout,&uniform,&view,&self.sampler);
            self.gpu_objects.push(GpuObject{vertex,index,count:mesh.indices.len() as u32,edge_index,edge_count:edges.len() as u32,uniform,bind_group,_material_texture:material_texture,material_loaded});
        }
        self.uploaded_generation=state.generation;
    }

    pub fn render(&mut self,shared:&Arc<Mutex<SharedRenderState>>){
        let state=shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.uploaded_generation!=state.generation||self.gpu_objects.len()!=state.scene.objects.len(){self.upload_scene(&state);}
        let aspect=self.target_width as f32/self.target_height.max(1) as f32;
        let view_proj=state.camera.view_proj(aspect,state.mode_2d).to_cols_array_2d();
        for (i,gpu) in self.gpu_objects.iter().enumerate(){
            let Some(object)=state.scene.objects.get(i) else {continue;};
            let texture_enabled=gpu.material_loaded&&state.use_textures&&object.mesh.has_texture_coordinates();
            let c=object.mesh.base_color_factor;
            let uniforms=Uniforms{view_proj,model:Mat4::IDENTITY.to_cols_array_2d(),light:[0.3,0.8,0.5,if i==state.selected_index{1.0}else{0.0}],material:[if texture_enabled{1.0}else{0.0},c[0],c[1],c[2]]};
            self.queue.write_buffer(&gpu.uniform,0,bytemuck::bytes_of(&uniforms));
        }

        let mut encoder=self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor{label:Some("model encoder")});
        {
            let color_view=self.texture.create_view(&wgpu::TextureViewDescriptor::default());let depth_view=self.depth.create_view(&wgpu::TextureViewDescriptor::default());
            let color_attachments=[Some(wgpu::RenderPassColorAttachment{view:&color_view,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color{r:0.035,g:0.045,b:0.065,a:1.0}),store:wgpu::StoreOp::Store},depth_slice:None})];
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("model pass"),color_attachments:&color_attachments,depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{view:&depth_view,depth_ops:Some(wgpu::Operations{load:wgpu::LoadOp::Clear(1.0),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            for (i,gpu) in self.gpu_objects.iter().enumerate(){
                let Some(object)=state.scene.objects.get(i) else {continue;}; if !object.visible{continue;}
                pass.set_bind_group(0,&gpu.bind_group,&[]);pass.set_vertex_buffer(0,gpu.vertex.slice(..));
                if !state.wireframe{pass.set_pipeline(&self.solid_pipeline);pass.set_index_buffer(gpu.index.slice(..),wgpu::IndexFormat::Uint32);pass.draw_indexed(0..gpu.count,0,0..1);}
                // Selection no longer forces a full triangle-edge overlay. Edges are drawn only
                // when the user explicitly enables Show edges, or when Wireframe is active.
                if state.wireframe||state.show_edges{pass.set_pipeline(&self.edge_pipeline);pass.set_index_buffer(gpu.edge_index.slice(..),wgpu::IndexFormat::Uint32);pass.draw_indexed(0..gpu.edge_count,0,0..1);}
            }
        }
        drop(state);
        self.queue.submit(Some(encoder.finish()));
    }
}

fn ray_triangle(origin:Vec3,dir:Vec3,a:Vec3,b:Vec3,c:Vec3)->Option<f32>{
    let eps=1.0e-7;let e1=b-a;let e2=c-a;let p=dir.cross(e2);let det=e1.dot(p);if det.abs()<eps{return None;}
    let inv_det=1.0/det;let tvec=origin-a;let u=tvec.dot(p)*inv_det;if !(0.0..=1.0).contains(&u){return None;}
    let q=tvec.cross(e1);let v=dir.dot(q)*inv_det;if v<0.0||u+v>1.0{return None;}
    let t=e2.dot(q)*inv_det;(t>=0.0).then_some(t)
}
fn ray_hits_aabb(origin:Vec3,dir:Vec3,min:Vec3,max:Vec3)->bool{
    let mut t_min=0.0f32;let mut t_max=f32::INFINITY;
    for axis in 0..3{
        let o=origin[axis];let d=dir[axis];let lo=min[axis];let hi=max[axis];
        if d.abs()<1.0e-12{if o<lo||o>hi{return false;}continue;}
        let mut a=(lo-o)/d;let mut b=(hi-o)/d;if a>b{std::mem::swap(&mut a,&mut b);}t_min=t_min.max(a);t_max=t_max.min(b);if t_max<t_min{return false;}
    }
    true
}

fn load_material_texture(device:&wgpu::Device,queue:&wgpu::Queue,mesh:&SceneMesh,index:usize)->(wgpu::Texture,bool){
    if mesh.has_texture_coordinates(){
        if let Some(embedded)=&mesh.embedded_base_color{if embedded.width>0&&embedded.height>0&&embedded.rgba8.len()==embedded.width as usize*embedded.height as usize*4{return(create_rgba_texture(device,queue,&format!("embedded material {index}"),embedded.width,embedded.height,&embedded.rgba8),true);}}
        if let Some(path)=&mesh.base_color_texture{if let Ok(decoded)=image::open(path){let rgba=decoded.to_rgba8();let (w,h)=rgba.dimensions();if w>0&&h>0{return(create_rgba_texture(device,queue,&format!("material {index}"),w,h,rgba.as_raw()),true);}}}
    }
    (create_rgba_texture(device,queue,"white material",1,1,&[255,255,255,255]),false)
}
fn create_scene_bind_group(device:&wgpu::Device,layout:&wgpu::BindGroupLayout,uniform:&wgpu::Buffer,texture_view:&wgpu::TextureView,sampler:&wgpu::Sampler)->wgpu::BindGroup{device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("scene bg"),layout,entries:&[
    wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::TextureView(texture_view)},wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::Sampler(sampler)}]})}
fn create_rgba_texture(device:&wgpu::Device,queue:&wgpu::Queue,label:&str,width:u32,height:u32,data:&[u8])->wgpu::Texture{device.create_texture_with_data(queue,&wgpu::TextureDescriptor{label:Some(label),size:wgpu::Extent3d{width:width.max(1),height:height.max(1),depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]},wgpu::util::TextureDataOrder::LayerMajor,data)}
fn create_target(device:&wgpu::Device,width:u32,height:u32)->wgpu::Texture{device.create_texture(&wgpu::TextureDescriptor{label:Some("viewport"),size:wgpu::Extent3d{width,height,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]})}
fn create_depth(device:&wgpu::Device,width:u32,height:u32)->wgpu::Texture{device.create_texture(&wgpu::TextureDescriptor{label:Some("depth"),size:wgpu::Extent3d{width,height,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Depth32Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]})}
