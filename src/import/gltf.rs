use std::path::Path;

use glam::{Mat4, Vec3};
use gltf::{image::Format, mesh::Mode};

use crate::{
    assets::resolve_referenced_texture,
    mesh::{EmbeddedTexture, SceneDocument, SceneMesh, SceneObject, Vertex},
};

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let (document, buffers, images) = gltf::import(path).map_err(|e| format!("glTF import failed: {e}"))?;
    let scene = document.default_scene().or_else(|| document.scenes().next()).ok_or("glTF contains no scene")?;
    let image_names: Vec<String> = document.images().enumerate().map(|(i, image)| {
        image.name().map(str::to_string).unwrap_or_else(|| format!("Image {i}"))
    }).collect();

    let mut objects = Vec::<SceneObject>::new();
    for node in scene.nodes() {
        append_node(node, Mat4::IDENTITY, &buffers, &images, &image_names, path, &mut objects)?;
    }
    if objects.is_empty() { return Err("glTF contains no supported triangle primitives".into()); }

    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("glTF model");
    let mut result = SceneDocument::new(name, objects)?;
    result.source_path = Some(path.to_path_buf());
    Ok(result)
}

fn append_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    image_names: &[String],
    path: &Path,
    objects: &mut Vec<SceneObject>,
) -> Result<(), String> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    let normal_matrix = world.inverse().transpose();

    if let Some(source_mesh) = node.mesh() {
        for (primitive_index, primitive) in source_mesh.primitives().enumerate() {
            if !matches!(primitive.mode(), Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan) { continue; }
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()].0));
            let Some(position_iter) = reader.read_positions() else { continue; };
            let positions: Vec<[f32; 3]> = position_iter.collect();
            if positions.is_empty() { continue; }

            let normals: Vec<[f32; 3]> = reader.read_normals().map(|it| it.collect()).unwrap_or_else(|| vec![[0.0; 3]; positions.len()]);
            let texcoords: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|it| it.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
            let mut vertices = Vec::<Vertex>::with_capacity(positions.len());
            for (i, position) in positions.iter().enumerate() {
                let p = world.transform_point3(Vec3::from_array(*position)) * 1000.0;
                let n = normals.get(i).copied().map(Vec3::from_array).map(|n| normal_matrix.transform_vector3(n).normalize_or_zero()).unwrap_or(Vec3::ZERO);
                let uv = texcoords.get(i).copied().unwrap_or([0.0; 2]);
                vertices.push(Vertex { position: p.to_array(), normal: n.to_array(), texcoord: uv });
            }

            let source_indices: Vec<u32> = reader.read_indices().map(|it| it.into_u32().collect()).unwrap_or_else(|| (0..positions.len() as u32).collect());
            let mut indices = Vec::<u32>::new();
            append_primitive_indices(primitive.mode(), &source_indices, &mut indices);
            if indices.is_empty() { continue; }

            let node_name = node.name().or_else(|| source_mesh.name()).unwrap_or("Mesh");
            let object_name = if source_mesh.primitives().count() > 1 { format!("{node_name} • Primitive {}", primitive_index + 1) } else { node_name.to_string() };
            let mut mesh = SceneMesh::new(object_name.clone(), vertices, indices)?;
            mesh.source_mesh_count = 1;
            mesh.embedded_texture_names = image_names.to_vec();

            let material = primitive.material();
            mesh.material_count = usize::from(material.index().is_some());
            mesh.material_name = material.name().map(str::to_string).unwrap_or_else(|| material.index().map(|i| format!("Material {}", i + 1)).unwrap_or_else(|| "Default".into()));
            mesh.base_color_factor = material.pbr_metallic_roughness().base_color_factor();

            if let Some(info) = material.pbr_metallic_roughness().base_color_texture() {
                let source_image = info.texture().source();
                let image_index = source_image.index();
                if let Some(data) = images.get(image_index) {
                    if let Some(rgba) = image_to_rgba8(data) {
                        mesh.embedded_base_color = Some(EmbeddedTexture {
                            name: image_names.get(image_index).cloned().unwrap_or_else(|| format!("Image {image_index}")),
                            width: data.width,
                            height: data.height,
                            rgba8: rgba,
                        });
                    }
                }
                if let gltf::image::Source::Uri { uri, .. } = source_image.source() {
                    if !uri.starts_with("data:") {
                        if let Some(resolved) = resolve_referenced_texture(path, uri) {
                            mesh.base_color_texture = Some(resolved.clone());
                            mesh.discovered_textures.push(resolved);
                        }
                    }
                }
            }
            objects.push(SceneObject::new(object_name, mesh));
        }
    }

    for child in node.children() {
        append_node(child, world, buffers, images, image_names, path, objects)?;
    }
    Ok(())
}

fn append_primitive_indices(mode: Mode, src: &[u32], dst: &mut Vec<u32>) {
    match mode {
        Mode::Triangles => for tri in src.chunks_exact(3) { dst.extend_from_slice(tri); },
        Mode::TriangleStrip => {
            for i in 2..src.len() {
                let (a, b) = if i % 2 == 0 { (src[i - 2], src[i - 1]) } else { (src[i - 1], src[i - 2]) };
                dst.extend_from_slice(&[a, b, src[i]]);
            }
        }
        Mode::TriangleFan => {
            if src.len() >= 3 { for i in 2..src.len() { dst.extend_from_slice(&[src[0], src[i - 1], src[i]]); } }
        }
        _ => {}
    }
}

fn image_to_rgba8(data: &gltf::image::Data) -> Option<Vec<u8>> {
    let count = data.width as usize * data.height as usize;
    let mut out = Vec::with_capacity(count * 4);
    match data.format {
        Format::R8 => for &r in &data.pixels { out.extend_from_slice(&[r, r, r, 255]); },
        Format::R8G8 => for p in data.pixels.chunks_exact(2) { out.extend_from_slice(&[p[0], p[0], p[0], p[1]]); },
        Format::R8G8B8 => for p in data.pixels.chunks_exact(3) { out.extend_from_slice(&[p[0], p[1], p[2], 255]); },
        Format::R8G8B8A8 => return Some(data.pixels.clone()),
        Format::R16 => for p in data.pixels.chunks_exact(2) { let r = (u16::from_le_bytes([p[0],p[1]]) >> 8) as u8; out.extend_from_slice(&[r,r,r,255]); },
        Format::R16G16 => for p in data.pixels.chunks_exact(4) { let r=(u16::from_le_bytes([p[0],p[1]])>>8) as u8; let a=(u16::from_le_bytes([p[2],p[3]])>>8) as u8; out.extend_from_slice(&[r,r,r,a]); },
        Format::R16G16B16 => for p in data.pixels.chunks_exact(6) { out.extend_from_slice(&[((u16::from_le_bytes([p[0],p[1]])>>8) as u8),((u16::from_le_bytes([p[2],p[3]])>>8) as u8),((u16::from_le_bytes([p[4],p[5]])>>8) as u8),255]); },
        Format::R16G16B16A16 => for p in data.pixels.chunks_exact(8) { out.extend_from_slice(&[((u16::from_le_bytes([p[0],p[1]])>>8) as u8),((u16::from_le_bytes([p[2],p[3]])>>8) as u8),((u16::from_le_bytes([p[4],p[5]])>>8) as u8),((u16::from_le_bytes([p[6],p[7]])>>8) as u8)]); },
        Format::R32G32B32FLOAT => for p in data.pixels.chunks_exact(12) { let r=f32::from_le_bytes(p[0..4].try_into().ok()?); let g=f32::from_le_bytes(p[4..8].try_into().ok()?); let b=f32::from_le_bytes(p[8..12].try_into().ok()?); out.extend_from_slice(&[float_u8(r),float_u8(g),float_u8(b),255]); },
        Format::R32G32B32A32FLOAT => for p in data.pixels.chunks_exact(16) { let r=f32::from_le_bytes(p[0..4].try_into().ok()?); let g=f32::from_le_bytes(p[4..8].try_into().ok()?); let b=f32::from_le_bytes(p[8..12].try_into().ok()?); let a=f32::from_le_bytes(p[12..16].try_into().ok()?); out.extend_from_slice(&[float_u8(r),float_u8(g),float_u8(b),float_u8(a)]); },
    }
    (out.len() == count * 4).then_some(out)
}

fn float_u8(v: f32) -> u8 { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 }
