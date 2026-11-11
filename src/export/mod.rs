use std::{
    fs::{self, File},
    io::{BufWriter, Cursor, Write},
    path::Path,
};

use glam::Vec3;
use serde_json::{json, Value};

use crate::mesh::{SceneDocument, SceneMesh};

/// Export a complete scene. Scene-aware formats preserve object names/materials;
/// triangle-only formats are flattened because the format has no scene hierarchy.
pub fn save(scene: &SceneDocument, path: &Path) -> Result<(), String> {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "obj" => save_obj_scene(scene, path),
        "gltf" => save_gltf_scene(scene, path, false),
        "glb" => save_gltf_scene(scene, path, true),
        "bim" => save_bim_scene(scene, path),
        "stl" => save_stl(&scene.combined_mesh(false)?, path),
        "off" => save_off(&scene.combined_mesh(false)?, path),
        "ply" => save_ply(&scene.combined_mesh(false)?, path),
        other => Err(format!("Export to {other} is not enabled in v0.2.2")),
    }
}

fn save_obj_scene(scene: &SceneDocument, path: &Path) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = safe_filename(path.file_stem().and_then(|v| v.to_str()).unwrap_or("model"));
    let mtl_name = format!("{stem}.mtl");
    let mtl_path = parent.join(&mtl_name);
    let textures_dir = parent.join("textures");

    let mut obj = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let mut mtl = BufWriter::new(File::create(&mtl_path).map_err(|e| e.to_string())?);
    writeln!(obj, "# 3D Viewer v0.2.2") .map_err(|e| e.to_string())?;
    writeln!(obj, "mtllib {mtl_name}").map_err(|e| e.to_string())?;
    writeln!(mtl, "# 3D Viewer v0.2.2 materials").map_err(|e| e.to_string())?;

    let mut vertex_base = 1u32;
    let mut texcoord_base = 1u32;
    for (object_index, object) in scene.objects.iter().enumerate() {
        let mesh = &object.mesh;
        let object_name = safe_obj_name(&object.name, object_index);
        let material_name = format!("mat_{:03}_{}", object_index + 1, safe_obj_token(&mesh.material_name));

        writeln!(obj, "\no {object_name}").map_err(|e| e.to_string())?;
        writeln!(obj, "usemtl {material_name}").map_err(|e| e.to_string())?;

        for v in &mesh.vertices {
            writeln!(obj, "v {} {} {}", v.position[0], v.position[1], v.position[2]).map_err(|e| e.to_string())?;
        }
        let has_uv = mesh.has_texture_coordinates();
        if has_uv {
            for v in &mesh.vertices {
                writeln!(obj, "vt {} {}", v.texcoord[0], 1.0 - v.texcoord[1]).map_err(|e| e.to_string())?;
            }
        }
        for v in &mesh.vertices {
            writeln!(obj, "vn {} {} {}", v.normal[0], v.normal[1], v.normal[2]).map_err(|e| e.to_string())?;
        }
        for tri in mesh.indices.chunks_exact(3) {
            let a = vertex_base + tri[0];
            let b = vertex_base + tri[1];
            let c = vertex_base + tri[2];
            if has_uv {
                let ta = texcoord_base + tri[0];
                let tb = texcoord_base + tri[1];
                let tc = texcoord_base + tri[2];
                writeln!(obj, "f {a}/{ta}/{a} {b}/{tb}/{b} {c}/{tc}/{c}").map_err(|e| e.to_string())?;
            } else {
                writeln!(obj, "f {0}//{0} {1}//{1} {2}//{2}", a, b, c).map_err(|e| e.to_string())?;
            }
        }
        vertex_base += mesh.vertices.len() as u32;
        if has_uv { texcoord_base += mesh.vertices.len() as u32; }

        let c = mesh.base_color_factor;
        writeln!(mtl, "\nnewmtl {material_name}").map_err(|e| e.to_string())?;
        writeln!(mtl, "Ka 0 0 0").map_err(|e| e.to_string())?;
        writeln!(mtl, "Kd {} {} {}", c[0], c[1], c[2]).map_err(|e| e.to_string())?;
        writeln!(mtl, "Ks 0.04 0.04 0.04").map_err(|e| e.to_string())?;
        writeln!(mtl, "d {}", c[3]).map_err(|e| e.to_string())?;
        writeln!(mtl, "illum 2").map_err(|e| e.to_string())?;

        if let Some(png) = material_png(mesh)? {
            fs::create_dir_all(&textures_dir).map_err(|e| e.to_string())?;
            let tex_name = format!("{:03}_{}_basecolor.png", object_index + 1, safe_filename(&object.name));
            fs::write(textures_dir.join(&tex_name), png).map_err(|e| e.to_string())?;
            writeln!(mtl, "map_Kd textures/{tex_name}").map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn save_off(mesh: &SceneMesh, path: &Path) -> Result<(), String> {
    let mut w = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    writeln!(w, "OFF\n{} {} 0", mesh.vertices.len(), mesh.triangle_count()).map_err(|e| e.to_string())?;
    for v in &mesh.vertices {
        writeln!(w, "{} {} {}", v.position[0], v.position[1], v.position[2]).map_err(|e| e.to_string())?;
    }
    for t in mesh.indices.chunks_exact(3) {
        writeln!(w, "3 {} {} {}", t[0], t[1], t[2]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_ply(mesh: &SceneMesh, path: &Path) -> Result<(), String> {
    let mut w = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    writeln!(w, "ply\nformat ascii 1.0\ncomment 3D Viewer 0.2.2\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nelement face {}\nproperty list uchar uint vertex_indices\nend_header", mesh.vertices.len(), mesh.triangle_count()).map_err(|e| e.to_string())?;
    for v in &mesh.vertices {
        writeln!(w, "{} {} {} {} {} {}", v.position[0], v.position[1], v.position[2], v.normal[0], v.normal[1], v.normal[2]).map_err(|e| e.to_string())?;
    }
    for t in mesh.indices.chunks_exact(3) {
        writeln!(w, "3 {} {} {}", t[0], t[1], t[2]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_stl(mesh: &SceneMesh, path: &Path) -> Result<(), String> {
    let mut w = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let mut header = [0u8; 80];
    let label = b"3D Viewer v0.2.2 binary STL";
    header[..label.len()].copy_from_slice(label);
    w.write_all(&header).map_err(|e| e.to_string())?;
    w.write_all(&(mesh.triangle_count() as u32).to_le_bytes()).map_err(|e| e.to_string())?;
    for t in mesh.indices.chunks_exact(3) {
        let a = Vec3::from_array(mesh.vertices[t[0] as usize].position);
        let b = Vec3::from_array(mesh.vertices[t[1] as usize].position);
        let c = Vec3::from_array(mesh.vertices[t[2] as usize].position);
        let n = (b - a).cross(c - a).normalize_or_zero();
        for f in [n.x, n.y, n.z, a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z] {
            w.write_all(&f.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        w.write_all(&0u16.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_gltf_scene(scene: &SceneDocument, path: &Path, binary_glb: bool) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = safe_filename(path.file_stem().and_then(|v| v.to_str()).unwrap_or("model"));
    let textures_dir = parent.join("textures");

    let mut bin = Vec::<u8>::new();
    let mut buffer_views = Vec::<Value>::new();
    let mut accessors = Vec::<Value>::new();
    let mut meshes = Vec::<Value>::new();
    let mut nodes = Vec::<Value>::new();
    let mut materials = Vec::<Value>::new();
    let mut images = Vec::<Value>::new();
    let mut textures = Vec::<Value>::new();

    for (object_index, object) in scene.objects.iter().enumerate() {
        let mesh = &object.mesh;
        let positions_offset = bin.len();
        for v in &mesh.vertices {
            for f in v.position { bin.extend_from_slice(&(f / 1000.0).to_le_bytes()); }
        }
        let positions_len = bin.len() - positions_offset;
        align4(&mut bin);

        let normals_offset = bin.len();
        for v in &mesh.vertices {
            for f in v.normal { bin.extend_from_slice(&f.to_le_bytes()); }
        }
        let normals_len = bin.len() - normals_offset;
        align4(&mut bin);

        let uv_offset = bin.len();
        for v in &mesh.vertices {
            for f in v.texcoord { bin.extend_from_slice(&f.to_le_bytes()); }
        }
        let uv_len = bin.len() - uv_offset;
        align4(&mut bin);

        let index_offset = bin.len();
        for &i in &mesh.indices { bin.extend_from_slice(&i.to_le_bytes()); }
        let index_len = bin.len() - index_offset;
        align4(&mut bin);

        let pos_view = buffer_views.len();
        buffer_views.push(json!({"buffer":0,"byteOffset":positions_offset,"byteLength":positions_len,"target":34962}));
        let normal_view = buffer_views.len();
        buffer_views.push(json!({"buffer":0,"byteOffset":normals_offset,"byteLength":normals_len,"target":34962}));
        let uv_view = buffer_views.len();
        buffer_views.push(json!({"buffer":0,"byteOffset":uv_offset,"byteLength":uv_len,"target":34962}));
        let index_view = buffer_views.len();
        buffer_views.push(json!({"buffer":0,"byteOffset":index_offset,"byteLength":index_len,"target":34963}));

        let min = mesh.bounds_min / 1000.0;
        let max = mesh.bounds_max / 1000.0;
        let pos_accessor = accessors.len();
        accessors.push(json!({"bufferView":pos_view,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3","min":[min.x,min.y,min.z],"max":[max.x,max.y,max.z]}));
        let normal_accessor = accessors.len();
        accessors.push(json!({"bufferView":normal_view,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3"}));
        let uv_accessor = accessors.len();
        accessors.push(json!({"bufferView":uv_view,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC2"}));
        let index_accessor = accessors.len();
        accessors.push(json!({"bufferView":index_view,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}));

        let mut pbr = json!({
            "baseColorFactor": mesh.base_color_factor,
            "metallicFactor": 0.0,
            "roughnessFactor": 0.85
        });
        if let Some(png) = material_png(mesh)? {
            let image_index = images.len();
            if binary_glb {
                let image_offset = bin.len();
                let image_len = png.len();
                bin.extend_from_slice(&png);
                align4(&mut bin);
                let view_index = buffer_views.len();
                buffer_views.push(json!({"buffer":0,"byteOffset":image_offset,"byteLength":image_len}));
                images.push(json!({"bufferView":view_index,"mimeType":"image/png","name":format!("{} Base Color", object.name)}));
            } else {
                fs::create_dir_all(&textures_dir).map_err(|e| e.to_string())?;
                let image_name = format!("{:03}_{}_basecolor.png", object_index + 1, safe_filename(&object.name));
                fs::write(textures_dir.join(&image_name), png).map_err(|e| e.to_string())?;
                images.push(json!({"uri":format!("textures/{image_name}"),"name":format!("{} Base Color", object.name)}));
            }
            let texture_index = textures.len();
            textures.push(json!({"sampler":0,"source":image_index}));
            pbr["baseColorTexture"] = json!({"index":texture_index});
        }
        let material_index = materials.len();
        materials.push(json!({
            "name": if mesh.material_name.is_empty() { format!("Material {}", object_index + 1) } else { mesh.material_name.clone() },
            "pbrMetallicRoughness": pbr,
            "doubleSided": true,
            "alphaMode": if mesh.base_color_factor[3] < 0.999 { "BLEND" } else { "OPAQUE" }
        }));

        let mesh_index = meshes.len();
        meshes.push(json!({
            "name": object.name,
            "primitives":[{
                "attributes":{"POSITION":pos_accessor,"NORMAL":normal_accessor,"TEXCOORD_0":uv_accessor},
                "indices":index_accessor,
                "material":material_index,
                "mode":4
            }]
        }));
        nodes.push(json!({"mesh":mesh_index,"name":object.name}));
    }

    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let mut root = json!({
        "asset":{"version":"2.0","generator":"3D Viewer 0.2.2"},
        "scene":0,
        "scenes":[{"name":scene.name,"nodes":scene_nodes}],
        "nodes":nodes,
        "meshes":meshes,
        "bufferViews":buffer_views,
        "accessors":accessors,
        "materials":materials,
        "images":images,
        "textures":textures,
    });
    if !root["textures"].as_array().map(|v| v.is_empty()).unwrap_or(true) {
        root["samplers"] = json!([{"magFilter":9729,"minFilter":9987,"wrapS":10497,"wrapT":10497}]);
    }

    root["buffers"] = if binary_glb {
        json!([{"byteLength":bin.len()}])
    } else {
        let bin_name = format!("{stem}.bin");
        fs::write(parent.join(&bin_name), &bin).map_err(|e| e.to_string())?;
        json!([{"byteLength":bin.len(),"uri":bin_name}])
    };

    if binary_glb {
        let mut json_bytes = serde_json::to_vec(&root).map_err(|e| e.to_string())?;
        while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
        align4(&mut bin);
        let total_len = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(&0x46546C67u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total_len as u32).to_le_bytes());
        out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        fs::write(path, out).map_err(|e| e.to_string())?;
    } else {
        fs::write(path, serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_bim_scene(scene: &SceneDocument, path: &Path) -> Result<(), String> {
    let mut meshes = Vec::<Value>::new();
    let mut elements = Vec::<Value>::new();
    for (i, object) in scene.objects.iter().enumerate() {
        let mesh = &object.mesh;
        let mut coordinates = Vec::<f64>::with_capacity(mesh.vertices.len() * 3);
        for v in &mesh.vertices {
            coordinates.push(v.position[0] as f64 / 1000.0);
            coordinates.push(v.position[1] as f64 / 1000.0);
            coordinates.push(v.position[2] as f64 / 1000.0);
        }
        let indices: Vec<u64> = mesh.indices.iter().map(|&idx| idx as u64).collect();
        meshes.push(json!({"mesh_id":i,"coordinates":coordinates,"indices":indices}));
        let c = mesh.base_color_factor;
        elements.push(json!({
            "mesh_id": i,
            "vector":{"x":0.0,"y":0.0,"z":0.0},
            "rotation":{"qx":0.0,"qy":0.0,"qz":0.0,"qw":1.0},
            "guid": format!("00000000-0000-0000-0000-{:012}", i + 1),
            "type": if mesh.material_name.is_empty() { "Mesh" } else { mesh.material_name.as_str() },
            "color":{
                "r":float_to_u8(c[0]),"g":float_to_u8(c[1]),"b":float_to_u8(c[2]),"a":float_to_u8(c[3])
            },
            "info":{
                "Name":object.name,
                "Material":mesh.material_name,
                "VisibleInSourceViewer":object.visible,
                "Exporter":"3D Viewer 0.2.2"
            }
        }));
    }
    let root = json!({
        "schema_version":"1.1.0",
        "meshes":meshes,
        "elements":elements,
        "info":{"Name":scene.name,"Exporter":"3D Viewer 0.2.2"}
    });
    fs::write(path, serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn material_png(mesh: &SceneMesh) -> Result<Option<Vec<u8>>, String> {
    if !mesh.has_texture_coordinates() { return Ok(None); }
    let rgba = if let Some(embedded) = &mesh.embedded_base_color {
        image::RgbaImage::from_raw(embedded.width, embedded.height, embedded.rgba8.clone())
            .ok_or("Embedded texture has an invalid RGBA buffer")?
    } else if let Some(path) = &mesh.base_color_texture {
        image::open(path).map_err(|e| format!("Cannot open base-color texture: {e}"))?.to_rgba8()
    } else {
        return Ok(None);
    };
    let mut cursor = Cursor::new(Vec::<u8>::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(Some(cursor.into_inner()))
}

fn align4(bytes: &mut Vec<u8>) { while bytes.len() % 4 != 0 { bytes.push(0); } }
fn float_to_u8(v: f32) -> u8 { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 }

fn safe_obj_name(name: &str, index: usize) -> String {
    let clean = safe_obj_token(name);
    if clean.is_empty() { format!("Object_{}", index + 1) } else { clean }
}
fn safe_obj_token(value: &str) -> String {
    value.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' }).collect()
}
fn safe_filename(value: &str) -> String {
    let mut out: String = value.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' }).collect();
    while out.contains("__") { out = out.replace("__", "_"); }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() { "model".into() } else { out }
}
