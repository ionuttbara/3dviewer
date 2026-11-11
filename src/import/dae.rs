use std::{collections::HashMap, fs, path::Path};

use glam::{Mat4, Quat, Vec3};
use roxmltree::{Document, Node};

use crate::{
    assets::resolve_referenced_texture,
    mesh::{SceneDocument, SceneMesh, SceneObject, Vertex},
};

#[derive(Clone, Debug)]
struct SourceData {
    values: Vec<f32>,
    stride: usize,
}

#[derive(Clone, Debug)]
struct PrimitiveInput {
    semantic: String,
    source: String,
    offset: usize,
    set: usize,
}

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = Document::parse(&text).map_err(|e| format!("Invalid COLLADA XML: {e}"))?;
    let unit_to_mm = document_unit_to_mm(&doc);
    let up_axis = document_up_axis(&doc);
    let dae_images = collect_images(&doc, path);

    let mut geometries = HashMap::<String, Vec<SceneObject>>::new();
    for geometry in doc.descendants().filter(|n| n.has_tag_name("geometry")) {
        let Some(id) = geometry.attribute("id") else { continue; };
        let name = geometry.attribute("name").unwrap_or(id);
        let objects = parse_geometry(geometry, name, unit_to_mm, path, &dae_images)?;
        if !objects.is_empty() { geometries.insert(id.to_string(), objects); }
    }
    if geometries.is_empty() { return Err("COLLADA contains no supported mesh geometry".into()); }

    let mut objects = Vec::<SceneObject>::new();
    if let Some(visual_scene) = active_visual_scene(&doc) {
        for node in visual_scene.children().filter(|n| n.has_tag_name("node")) {
            append_scene_node(node, Mat4::IDENTITY, unit_to_mm, up_axis, &geometries, &mut objects)?;
        }
    }

    // Some DAE files contain geometry but no visual_scene. Keep them viewable.
    if objects.is_empty() {
        for group in geometries.values() {
            for mut object in group.clone() {
                apply_up_axis(&mut object.mesh, up_axis);
                objects.push(object);
            }
        }
    }
    if objects.is_empty() { return Err("COLLADA contains no renderable triangle primitives".into()); }

    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("COLLADA model");
    let mut scene = SceneDocument::new(name, objects)?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

fn parse_geometry(
    geometry: Node<'_, '_>,
    geometry_name: &str,
    unit_to_mm: f32,
    path: &Path,
    dae_images: &[std::path::PathBuf],
) -> Result<Vec<SceneObject>, String> {
    let Some(mesh_node) = geometry.children().find(|n| n.has_tag_name("mesh")) else { return Ok(Vec::new()); };
    let mut sources = HashMap::<String, SourceData>::new();
    for source in mesh_node.children().filter(|n| n.has_tag_name("source")) {
        let Some(id) = source.attribute("id") else { continue; };
        let Some(array) = source.descendants().find(|n| n.has_tag_name("float_array")) else { continue; };
        let values = parse_f32_list(array.text().unwrap_or(""))?;
        let stride = source
            .descendants()
            .find(|n| n.has_tag_name("accessor"))
            .and_then(|n| n.attribute("stride"))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(3)
            .max(1);
        sources.insert(id.to_string(), SourceData { values, stride });
    }

    let mut vertices_position_source = HashMap::<String, String>::new();
    for vertices in mesh_node.children().filter(|n| n.has_tag_name("vertices")) {
        let Some(id) = vertices.attribute("id") else { continue; };
        if let Some(input) = vertices.children().find(|n| n.has_tag_name("input") && n.attribute("semantic") == Some("POSITION")) {
            if let Some(source) = input.attribute("source") {
                vertices_position_source.insert(id.to_string(), strip_hash(source).to_string());
            }
        }
    }

    let mut result = Vec::<SceneObject>::new();
    let primitives = mesh_node.children().filter(|n| n.has_tag_name("triangles") || n.has_tag_name("polylist"));
    for (primitive_index, primitive) in primitives.enumerate() {
        let mut inputs = Vec::<PrimitiveInput>::new();
        let mut tuple_stride = 0usize;
        for input in primitive.children().filter(|n| n.has_tag_name("input")) {
            let semantic = input.attribute("semantic").unwrap_or("").to_string();
            let source = strip_hash(input.attribute("source").unwrap_or("")).to_string();
            let offset = input.attribute("offset").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            let set = input.attribute("set").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            tuple_stride = tuple_stride.max(offset + 1);
            inputs.push(PrimitiveInput { semantic, source, offset, set });
        }
        if tuple_stride == 0 { continue; }
        let Some(p_node) = primitive.children().find(|n| n.has_tag_name("p")) else { continue; };
        let raw = parse_usize_list(p_node.text().unwrap_or(""))?;
        if raw.len() < tuple_stride * 3 { continue; }

        let mut corners = Vec::<Vec<usize>>::new();
        if primitive.has_tag_name("triangles") {
            for tuple in raw.chunks_exact(tuple_stride) { corners.push(tuple.to_vec()); }
        } else {
            let counts_node = primitive.children().find(|n| n.has_tag_name("vcount")).ok_or("COLLADA polylist is missing vcount")?;
            let counts = parse_usize_list(counts_node.text().unwrap_or(""))?;
            let mut cursor = 0usize;
            for count in counts {
                if count < 3 { cursor = cursor.saturating_add(count * tuple_stride); continue; }
                let needed = count * tuple_stride;
                if cursor + needed > raw.len() { return Err("COLLADA polylist index data is truncated".into()); }
                let polygon: Vec<Vec<usize>> = raw[cursor..cursor + needed].chunks_exact(tuple_stride).map(|v| v.to_vec()).collect();
                for i in 1..polygon.len() - 1 {
                    corners.push(polygon[0].clone());
                    corners.push(polygon[i].clone());
                    corners.push(polygon[i + 1].clone());
                }
                cursor += needed;
            }
        }
        if corners.len() < 3 { continue; }

        let mut vertices = Vec::<Vertex>::with_capacity(corners.len());
        let mut indices = Vec::<u32>::with_capacity(corners.len());
        for tuple in corners {
            let mut position = [0.0f32; 3];
            let mut normal = [0.0f32; 3];
            let mut texcoord = [0.0f32; 2];
            for input in &inputs {
                let Some(&index) = tuple.get(input.offset) else { continue; };
                match input.semantic.as_str() {
                    "VERTEX" => {
                        if let Some(pos_source) = vertices_position_source.get(&input.source) {
                            position = source_vec3(&sources, pos_source, index)?;
                        }
                    }
                    "POSITION" => position = source_vec3(&sources, &input.source, index)?,
                    "NORMAL" => normal = source_vec3(&sources, &input.source, index)?,
                    "TEXCOORD" if input.set == 0 => texcoord = source_vec2(&sources, &input.source, index)?,
                    _ => {}
                }
            }
            position = [position[0] * unit_to_mm, position[1] * unit_to_mm, position[2] * unit_to_mm];
            vertices.push(Vertex { position, normal, texcoord });
            indices.push(vertices.len() as u32 - 1);
        }

        let material = primitive.attribute("material").unwrap_or("Default");
        let object_name = if primitive_index == 0 {
            geometry_name.to_string()
        } else {
            format!("{geometry_name} • {}", primitive_index + 1)
        };
        let mut mesh = SceneMesh::new(object_name.clone(), vertices, indices)?;
        mesh.material_name = material.to_string();
        mesh.material_count = usize::from(material != "Default");
        if mesh.has_texture_coordinates() && dae_images.len() == 1 {
            mesh.base_color_texture = dae_images.first().cloned();
            mesh.discovered_textures = dae_images.to_vec();
        }
        // Also allow an image filename to match the material symbol.
        if mesh.has_texture_coordinates() && mesh.base_color_texture.is_none() {
            if let Some(found) = resolve_referenced_texture(path, material) { mesh.base_color_texture = Some(found); }
        }
        result.push(SceneObject::new(object_name, mesh));
    }
    Ok(result)
}

fn append_scene_node(
    node: Node<'_, '_>,
    parent: Mat4,
    unit_to_mm: f32,
    up_axis: &str,
    geometries: &HashMap<String, Vec<SceneObject>>,
    output: &mut Vec<SceneObject>,
) -> Result<(), String> {
    let local = node_transform(node, unit_to_mm)?;
    let world = parent * local;
    let node_name = node.attribute("name").or_else(|| node.attribute("id")).unwrap_or("Node");

    for instance in node.children().filter(|n| n.has_tag_name("instance_geometry")) {
        let Some(url) = instance.attribute("url") else { continue; };
        let Some(group) = geometries.get(strip_hash(url)) else { continue; };
        for (part_index, source) in group.iter().enumerate() {
            let mut object = source.clone();
            object.name = if group.len() > 1 { format!("{node_name} • {}", part_index + 1) } else { node_name.to_string() };
            object.mesh.name = object.name.clone();
            transform_mesh(&mut object.mesh, world);
            apply_up_axis(&mut object.mesh, up_axis);
            output.push(object);
        }
    }
    for child in node.children().filter(|n| n.has_tag_name("node")) {
        append_scene_node(child, world, unit_to_mm, up_axis, geometries, output)?;
    }
    Ok(())
}

fn node_transform(node: Node<'_, '_>, unit_to_mm: f32) -> Result<Mat4, String> {
    let mut out = Mat4::IDENTITY;
    for child in node.children().filter(|n| n.is_element()) {
        if child.has_tag_name("matrix") {
            let values = parse_f32_list(child.text().unwrap_or(""))?;
            if values.len() >= 16 {
                let mut m = [0.0f32; 16];
                m.copy_from_slice(&values[..16]);
                // COLLADA transform translations use the document unit.
                m[12] *= unit_to_mm; m[13] *= unit_to_mm; m[14] *= unit_to_mm;
                out *= Mat4::from_cols_array(&m);
            }
        } else if child.has_tag_name("translate") {
            let v = parse_f32_list(child.text().unwrap_or(""))?;
            if v.len() >= 3 { out *= Mat4::from_translation(Vec3::new(v[0], v[1], v[2]) * unit_to_mm); }
        } else if child.has_tag_name("scale") {
            let v = parse_f32_list(child.text().unwrap_or(""))?;
            if v.len() >= 3 { out *= Mat4::from_scale(Vec3::new(v[0], v[1], v[2])); }
        } else if child.has_tag_name("rotate") {
            let v = parse_f32_list(child.text().unwrap_or(""))?;
            if v.len() >= 4 {
                let axis = Vec3::new(v[0], v[1], v[2]).normalize_or_zero();
                if axis.length_squared() > 0.0 { out *= Mat4::from_quat(Quat::from_axis_angle(axis, v[3].to_radians())); }
            }
        }
    }
    Ok(out)
}

fn transform_mesh(mesh: &mut SceneMesh, transform: Mat4) {
    let normal_matrix = transform.inverse().transpose();
    for vertex in &mut mesh.vertices {
        vertex.position = transform.transform_point3(Vec3::from_array(vertex.position)).to_array();
        vertex.normal = normal_matrix.transform_vector3(Vec3::from_array(vertex.normal)).normalize_or_zero().to_array();
    }
    mesh.recalculate_bounds();
}

fn apply_up_axis(mesh: &mut SceneMesh, up_axis: &str) {
    if up_axis.eq_ignore_ascii_case("Y_UP") { return; }
    for v in &mut mesh.vertices {
        let p = Vec3::from_array(v.position);
        let n = Vec3::from_array(v.normal);
        let (p2, n2) = if up_axis.eq_ignore_ascii_case("Z_UP") {
            (Vec3::new(p.x, p.z, -p.y), Vec3::new(n.x, n.z, -n.y))
        } else if up_axis.eq_ignore_ascii_case("X_UP") {
            (Vec3::new(-p.y, p.x, p.z), Vec3::new(-n.y, n.x, n.z))
        } else { (p, n) };
        v.position = p2.to_array();
        v.normal = n2.normalize_or_zero().to_array();
    }
    mesh.recalculate_bounds();
}

fn active_visual_scene<'a, 'input>(doc: &'a Document<'input>) -> Option<Node<'a, 'input>> {
    let selected = doc.descendants().find(|n| n.has_tag_name("instance_visual_scene")).and_then(|n| n.attribute("url")).map(strip_hash);
    let library = doc.descendants().find(|n| n.has_tag_name("library_visual_scenes"))?;
    selected
        .and_then(|id| library.children().find(|n| n.has_tag_name("visual_scene") && n.attribute("id") == Some(id)))
        .or_else(|| library.children().find(|n| n.has_tag_name("visual_scene")))
}

fn collect_images(doc: &Document<'_>, model_path: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for image in doc.descendants().filter(|n| n.has_tag_name("image")) {
        if let Some(init) = image.descendants().find(|n| n.has_tag_name("init_from")).and_then(|n| n.text()) {
            if let Some(path) = resolve_referenced_texture(model_path, init.trim()) {
                if !out.contains(&path) { out.push(path); }
            }
        }
    }
    out
}

fn document_unit_to_mm(doc: &Document<'_>) -> f32 {
    doc.descendants()
        .find(|n| n.has_tag_name("unit"))
        .and_then(|n| n.attribute("meter"))
        .and_then(|v| v.parse::<f32>().ok())
        .map(|metres| metres * 1000.0)
        .unwrap_or(1000.0)
}
fn document_up_axis<'a, 'input: 'a>(doc: &'a Document<'input>) -> &'a str {
    doc.descendants()
        .find(|n| n.has_tag_name("up_axis"))
        .and_then(|n| n.text())
        .map(str::trim)
        .unwrap_or("Y_UP")
}
fn strip_hash(value: &str) -> &str { value.strip_prefix('#').unwrap_or(value) }

fn source_vec3(sources: &HashMap<String, SourceData>, id: &str, index: usize) -> Result<[f32; 3], String> {
    let source = sources.get(id).ok_or_else(|| format!("COLLADA source '{id}' is missing"))?;
    let start = index.checked_mul(source.stride).ok_or("COLLADA source index overflow")?;
    if start >= source.values.len() { return Err("COLLADA source index is out of range".into()); }
    Ok([
        *source.values.get(start).unwrap_or(&0.0),
        *source.values.get(start + 1).unwrap_or(&0.0),
        *source.values.get(start + 2).unwrap_or(&0.0),
    ])
}
fn source_vec2(sources: &HashMap<String, SourceData>, id: &str, index: usize) -> Result<[f32; 2], String> {
    let source = sources.get(id).ok_or_else(|| format!("COLLADA source '{id}' is missing"))?;
    let start = index.checked_mul(source.stride).ok_or("COLLADA source index overflow")?;
    if start >= source.values.len() { return Err("COLLADA source index is out of range".into()); }
    Ok([
        *source.values.get(start).unwrap_or(&0.0),
        *source.values.get(start + 1).unwrap_or(&0.0),
    ])
}
fn parse_f32_list(text: &str) -> Result<Vec<f32>, String> {
    text.split_whitespace().map(|v| v.parse::<f32>().map_err(|e| format!("Invalid COLLADA float '{v}': {e}"))).collect()
}
fn parse_usize_list(text: &str) -> Result<Vec<usize>, String> {
    text.split_whitespace().map(|v| v.parse::<usize>().map_err(|e| format!("Invalid COLLADA index '{v}': {e}"))).collect()
}
