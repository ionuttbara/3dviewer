use std::{fs, path::Path};

use oxideav_fbx::FbxDecoder;
use oxideav_mesh3d::Mesh3DDecoder;

use crate::mesh::{SceneDocument, SceneMesh, SceneObject, Vertex};

/// Import Autodesk FBX through the pure-Rust OxideAV decoder.  The OxideAV
/// scene model exposes mesh primitives, UV channels and file unit metadata;
/// 3D Viewer normalises positions to millimetres before handing them
/// to the renderer/print estimator.
pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let bytes = fs::read(path).map_err(|e| format!("Unable to read FBX: {e}"))?;
    let scene = FbxDecoder::new()
        .decode(&bytes)
        .map_err(|e| format!("FBX decode failed: {e}"))?;

    let unit_to_mm = (scene.unit.to_metres() * 1000.0) as f32;
    let unit_to_mm = if unit_to_mm.is_finite() && unit_to_mm > 0.0 {
        unit_to_mm
    } else {
        1.0
    };

    let mut objects = Vec::<SceneObject>::new();
    for (mesh_index, source_mesh) in scene.meshes.iter().enumerate() {
        let base_name = source_mesh
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("FBX Mesh {}", mesh_index + 1));

        for (primitive_index, primitive) in source_mesh.primitives.iter().enumerate() {
            let triangles = primitive.triangle_indices();
            if primitive.positions.is_empty() || triangles.is_empty() {
                continue;
            }

            let normals = primitive.normals.as_ref();
            let uv0 = primitive.uvs.first();
            let mut vertices = Vec::<Vertex>::with_capacity(primitive.positions.len());
            for (i, position) in primitive.positions.iter().enumerate() {
                let normal = normals
                    .and_then(|values| values.get(i))
                    .copied()
                    .unwrap_or([0.0; 3]);
                let texcoord = uv0
                    .and_then(|values| values.get(i))
                    .copied()
                    .unwrap_or([0.0; 2]);
                vertices.push(Vertex {
                    position: [
                        position[0] * unit_to_mm,
                        position[1] * unit_to_mm,
                        position[2] * unit_to_mm,
                    ],
                    normal,
                    texcoord,
                });
            }

            let mut indices = Vec::<u32>::with_capacity(triangles.len() * 3);
            for triangle in triangles {
                if triangle.iter().all(|&i| (i as usize) < vertices.len()) {
                    indices.extend_from_slice(&triangle);
                }
            }
            if indices.len() < 3 {
                continue;
            }

            let name = if source_mesh.primitives.len() == 1 {
                base_name.clone()
            } else {
                format!("{} • Primitive {}", base_name, primitive_index + 1)
            };
            let mut mesh = SceneMesh::new(name.clone(), vertices, indices)?;
            mesh.source_mesh_count = 1;
            if primitive.material.is_some() {
                mesh.material_count = 1;
                mesh.material_name = format!("FBX Material {}", primitive_index + 1);
            }
            objects.push(SceneObject::new(name, mesh));
        }
    }

    if objects.is_empty() {
        return Err("FBX contains no supported triangle geometry".into());
    }

    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("FBX model");
    let mut document = SceneDocument::new(name, objects)?;
    document.source_path = Some(path.to_path_buf());
    Ok(document)
}
