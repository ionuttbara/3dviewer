use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[derive(Clone, Debug)]
pub struct EmbeddedTexture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct SceneMesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    pub base_color_texture: Option<PathBuf>,
    pub embedded_base_color: Option<EmbeddedTexture>,
    pub source_mesh_count: usize,
    pub material_count: usize,
    pub material_name: String,
    pub base_color_factor: [f32; 4],
    pub embedded_texture_names: Vec<String>,
    pub discovered_textures: Vec<PathBuf>,
    pub searched_texture_directories: Vec<PathBuf>,
    pub generated_uvs: bool,
}

impl SceneMesh {
    pub fn new(name: impl Into<String>, vertices: Vec<Vertex>, indices: Vec<u32>) -> Result<Self, String> {
        if vertices.is_empty() || indices.len() < 3 {
            return Err("Model contains no renderable triangles".into());
        }
        let mut mesh = Self {
            name: name.into(),
            vertices,
            indices,
            bounds_min: Vec3::ZERO,
            bounds_max: Vec3::ZERO,
            base_color_texture: None,
            embedded_base_color: None,
            source_mesh_count: 1,
            material_count: 0,
            material_name: "Default".into(),
            base_color_factor: [0.38, 0.64, 0.98, 1.0],
            embedded_texture_names: Vec::new(),
            discovered_textures: Vec::new(),
            searched_texture_directories: Vec::new(),
            generated_uvs: false,
        };
        mesh.recalculate_bounds();
        if mesh.vertices.iter().all(|v| Vec3::from_array(v.normal).length_squared() < 1e-10) {
            mesh.recalculate_normals();
        }
        Ok(mesh)
    }

    pub fn unit_cube() -> Self {
        let p = [
            [-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5],
            [-0.5, -0.5,  0.5], [0.5, -0.5,  0.5], [0.5, 0.5,  0.5], [-0.5, 0.5,  0.5],
        ];
        let idx: Vec<u32> = vec![
            0,2,1, 0,3,2, 4,5,6, 4,6,7,
            0,1,5, 0,5,4, 3,7,6, 3,6,2,
            1,2,6, 1,6,5, 0,4,7, 0,7,3,
        ];
        let vertices = p
            .into_iter()
            .map(|position| Vertex { position, normal: [0.0; 3], texcoord: [0.0; 2] })
            .collect();
        let mut m = Self::new("Demo cube", vertices, idx).unwrap();
        m.recalculate_normals();
        m
    }

    pub fn recalculate_bounds(&mut self) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in &self.vertices {
            let p = Vec3::from_array(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        self.bounds_min = min;
        self.bounds_max = max;
    }

    pub fn recalculate_normals(&mut self) {
        let mut accum = vec![Vec3::ZERO; self.vertices.len()];
        for tri in self.indices.chunks_exact(3) {
            let a = Vec3::from_array(self.vertices[tri[0] as usize].position);
            let b = Vec3::from_array(self.vertices[tri[1] as usize].position);
            let c = Vec3::from_array(self.vertices[tri[2] as usize].position);
            let n = (b - a).cross(c - a);
            if n.length_squared() > 1e-12 {
                for &i in tri { accum[i as usize] += n; }
            }
        }
        for (v, n) in self.vertices.iter_mut().zip(accum) {
            v.normal = n.normalize_or_zero().to_array();
        }
    }

    pub fn has_renderable_texture(&self) -> bool {
        self.has_texture_coordinates() && (self.base_color_texture.is_some() || self.embedded_base_color.is_some())
    }

    pub fn texture_asset_count(&self) -> usize {
        self.discovered_textures.len() + self.embedded_texture_names.len()
    }

    pub fn has_texture_coordinates(&self) -> bool {
        self.vertices.iter().any(|v| v.texcoord[0].abs() > 1e-7 || v.texcoord[1].abs() > 1e-7)
    }

    /// Generate stable box-projection UV coordinates for triangle meshes that do not
    /// carry UVs in the source format (notably STL). STL duplicates vertices per
    /// face, so selecting the projection plane from the face/vertex normal produces
    /// useful seams without changing geometry or topology.
    pub fn generate_box_uvs(&mut self) {
        let extent = (self.bounds_max - self.bounds_min).max(Vec3::splat(1.0e-6));
        for vertex in &mut self.vertices {
            let p = (Vec3::from_array(vertex.position) - self.bounds_min) / extent;
            let n = Vec3::from_array(vertex.normal).abs();
            let (u, v) = if n.x >= n.y && n.x >= n.z {
                (p.z, p.y)
            } else if n.y >= n.x && n.y >= n.z {
                (p.x, p.z)
            } else {
                (p.x, p.y)
            };
            vertex.texcoord = [u.clamp(0.0, 1.0), (1.0 - v).clamp(0.0, 1.0)];
        }
        self.generated_uvs = true;
    }

    pub fn center(&self) -> Vec3 { (self.bounds_min + self.bounds_max) * 0.5 }
    pub fn dimensions(&self) -> Vec3 { self.bounds_max - self.bounds_min }
    pub fn radius(&self) -> f32 { self.dimensions().length().max(0.001) * 0.5 }
    pub fn triangle_count(&self) -> usize { self.indices.len() / 3 }

    pub fn surface_area(&self) -> f64 {
        self.indices.chunks_exact(3).map(|t| {
            let a = Vec3::from_array(self.vertices[t[0] as usize].position);
            let b = Vec3::from_array(self.vertices[t[1] as usize].position);
            let c = Vec3::from_array(self.vertices[t[2] as usize].position);
            (0.5 * (b-a).cross(c-a).length()) as f64
        }).sum()
    }

    pub fn volume(&self) -> f64 {
        let signed: f64 = self.indices.chunks_exact(3).map(|t| {
            let a = Vec3::from_array(self.vertices[t[0] as usize].position);
            let b = Vec3::from_array(self.vertices[t[1] as usize].position);
            let c = Vec3::from_array(self.vertices[t[2] as usize].position);
            a.dot(b.cross(c)) as f64 / 6.0
        }).sum();
        signed.abs()
    }
}

#[derive(Clone, Debug)]
pub struct SceneObject {
    pub name: String,
    pub mesh: SceneMesh,
    pub visible: bool,
}

impl SceneObject {
    pub fn new(name: impl Into<String>, mut mesh: SceneMesh) -> Self {
        let name = name.into();
        if mesh.name.is_empty() { mesh.name = name.clone(); }
        Self { name, mesh, visible: true }
    }
}

#[derive(Clone, Debug)]
pub struct SceneDocument {
    pub name: String,
    pub objects: Vec<SceneObject>,
    pub source_path: Option<PathBuf>,
}

impl SceneDocument {
    pub fn from_mesh(mesh: SceneMesh) -> Self {
        let name = mesh.name.clone();
        Self { name: name.clone(), objects: vec![SceneObject::new(name, mesh)], source_path: None }
    }

    pub fn new(name: impl Into<String>, objects: Vec<SceneObject>) -> Result<Self, String> {
        if objects.is_empty() { return Err("Scene contains no renderable objects".into()); }
        Ok(Self { name: name.into(), objects, source_path: None })
    }

    pub fn unit_cube() -> Self { Self::from_mesh(SceneMesh::unit_cube()) }

    pub fn object_count(&self) -> usize { self.objects.len() }
    pub fn visible_object_count(&self) -> usize { self.objects.iter().filter(|o| o.visible).count() }
    pub fn triangle_count(&self) -> usize { self.objects.iter().map(|o| o.mesh.triangle_count()).sum() }
    pub fn material_count(&self) -> usize { self.objects.iter().map(|o| o.mesh.material_count.max(1)).sum() }
    pub fn texture_asset_count(&self) -> usize { self.objects.iter().map(|o| o.mesh.texture_asset_count()).sum() }

    pub fn bounds(&self, visible_only: bool) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for object in &self.objects {
            if visible_only && !object.visible { continue; }
            min = min.min(object.mesh.bounds_min);
            max = max.max(object.mesh.bounds_max);
            any = true;
        }
        any.then_some((min, max))
    }

    pub fn center(&self) -> Vec3 {
        self.bounds(true).or_else(|| self.bounds(false)).map(|(min,max)| (min+max)*0.5).unwrap_or(Vec3::ZERO)
    }

    pub fn dimensions(&self) -> Vec3 {
        self.bounds(true).or_else(|| self.bounds(false)).map(|(min,max)| max-min).unwrap_or(Vec3::ZERO)
    }

    pub fn radius(&self) -> f32 { self.dimensions().length().max(0.001) * 0.5 }

    pub fn combined_mesh(&self, visible_only: bool) -> Result<SceneMesh, String> {
        let mut vertices = Vec::<Vertex>::new();
        let mut indices = Vec::<u32>::new();
        let mut included = 0usize;
        for object in &self.objects {
            if visible_only && !object.visible { continue; }
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&object.mesh.vertices);
            indices.extend(object.mesh.indices.iter().map(|i| base + *i));
            included += 1;
        }
        if included == 0 { return Err("No visible objects to combine".into()); }
        let mut mesh = SceneMesh::new(self.name.clone(), vertices, indices)?;
        mesh.source_mesh_count = included;
        mesh.material_count = self.material_count();
        // A flattened mesh can safely retain the texture only when the scene contains one object.
        if included == 1 {
            if let Some(object) = self.objects.iter().find(|o| !visible_only || o.visible) {
                mesh.base_color_texture = object.mesh.base_color_texture.clone();
                mesh.embedded_base_color = object.mesh.embedded_base_color.clone();
                mesh.material_name = object.mesh.material_name.clone();
                mesh.base_color_factor = object.mesh.base_color_factor;
                mesh.discovered_textures = object.mesh.discovered_textures.clone();
                mesh.embedded_texture_names = object.mesh.embedded_texture_names.clone();
                mesh.searched_texture_directories = object.mesh.searched_texture_directories.clone();
                mesh.generated_uvs = object.mesh.generated_uvs;
            }
        }
        Ok(mesh)
    }
}
