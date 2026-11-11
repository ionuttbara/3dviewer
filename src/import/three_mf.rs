use std::{fs::File, io::Read, path::Path};

use quick_xml::{events::{BytesStart, Event}, Reader};
use zip::ZipArchive;

use crate::mesh::{SceneDocument, SceneMesh, SceneObject, Vertex};

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid 3MF ZIP container: {e}"))?;
    let model_name = archive.file_names().find(|name| name.to_ascii_lowercase().ends_with(".model")).map(str::to_string).ok_or("3MF package does not contain a .model document")?;

    let mut xml = String::new();
    archive.by_name(&model_name).map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    let embedded_images: Vec<String> = archive.file_names().filter(|name| is_image_name(name)).map(str::to_string).collect();

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut objects = Vec::<SceneObject>::new();
    let mut positions = Vec::<[f32; 3]>::new();
    let mut indices = Vec::<u32>::new();
    let mut in_vertices = false;
    let mut in_triangles = false;
    let mut in_object = false;
    let mut object_name = String::new();
    let mut unit_scale = 1.0f32;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"model" => unit_scale = unit_scale_mm(attr(&e, b"unit").as_deref()),
                b"object" => {
                    in_object = true;
                    positions.clear(); indices.clear();
                    let object_id = attr(&e, b"id").unwrap_or_else(|| (objects.len() + 1).to_string());
                    object_name = attr(&e, b"name").unwrap_or_else(|| format!("Object {object_id}"));
                }
                b"vertices" if in_object => in_vertices = true,
                b"triangles" if in_object => in_triangles = true,
                b"vertex" if in_vertices => positions.push([
                    attr_f32(&e, b"x")? * unit_scale,
                    attr_f32(&e, b"y")? * unit_scale,
                    attr_f32(&e, b"z")? * unit_scale,
                ]),
                b"triangle" if in_triangles => indices.extend_from_slice(&[
                    attr_u32(&e, b"v1")?, attr_u32(&e, b"v2")?, attr_u32(&e, b"v3")?
                ]),
                _ => {}
            },
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"vertex" if in_vertices => positions.push([
                    attr_f32(&e, b"x")? * unit_scale,
                    attr_f32(&e, b"y")? * unit_scale,
                    attr_f32(&e, b"z")? * unit_scale,
                ]),
                b"triangle" if in_triangles => indices.extend_from_slice(&[
                    attr_u32(&e, b"v1")?, attr_u32(&e, b"v2")?, attr_u32(&e, b"v3")?
                ]),
                _ => {}
            },
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"vertices" => in_vertices = false,
                b"triangles" => in_triangles = false,
                b"object" => {
                    if !positions.is_empty() && !indices.is_empty() {
                        let vertices = positions.iter().copied().map(|position| Vertex { position, normal: [0.0;3], texcoord:[0.0;2] }).collect();
                        let mut mesh = SceneMesh::new(object_name.clone(), vertices, indices.clone())?;
                        mesh.recalculate_normals();
                        mesh.source_mesh_count = 1;
                        mesh.embedded_texture_names = embedded_images.clone();
                        mesh.material_name = "3MF material".into();
                        objects.push(SceneObject::new(object_name.clone(), mesh));
                    }
                    in_object = false;
                    in_vertices = false; in_triangles = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("3MF XML parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    if objects.is_empty() { return Err("3MF package contains no renderable mesh objects".into()); }
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("3MF model");
    let mut scene = SceneDocument::new(name, objects)?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

fn local_name(name: &[u8]) -> &[u8] { name.rsplit(|b| *b == b':').next().unwrap_or(name) }
fn attr(e: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() { if local_name(a.key.as_ref()) == key { return Some(String::from_utf8_lossy(a.value.as_ref()).into_owned()); } }
    None
}
fn attr_f32(e: &BytesStart<'_>, key: &[u8]) -> Result<f32, String> {
    attr(e,key).ok_or_else(|| format!("3MF attribute {} missing", String::from_utf8_lossy(key)))?.parse().map_err(|_| format!("Invalid 3MF {}", String::from_utf8_lossy(key)))
}
fn attr_u32(e: &BytesStart<'_>, key: &[u8]) -> Result<u32, String> {
    attr(e,key).ok_or_else(|| format!("3MF attribute {} missing", String::from_utf8_lossy(key)))?.parse().map_err(|_| format!("Invalid 3MF {}", String::from_utf8_lossy(key)))
}
fn unit_scale_mm(unit: Option<&str>) -> f32 {
    match unit.unwrap_or("millimeter").to_ascii_lowercase().as_str() {
        "micron" => 0.001, "centimeter" => 10.0, "meter" => 1000.0, "inch" => 25.4, "foot" => 304.8, _ => 1.0,
    }
}
fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".bmp", ".tga", ".webp"].iter().any(|ext| lower.ends_with(ext))
}
