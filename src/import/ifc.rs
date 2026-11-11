use std::{collections::HashMap, fs, path::Path};

use crate::mesh::{SceneDocument, SceneMesh, SceneObject, Vertex};

#[derive(Clone, Debug)]
enum Value {
    Atom(String),
    List(Vec<Value>),
}

#[derive(Clone, Debug)]
struct Entity {
    id: u64,
    kind: String,
    args: Value,
}

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Could not read IFC: {e}"))?;
    let cleaned = strip_block_comments(&text);
    let entities = parse_entities(&cleaned)?;
    if entities.is_empty() {
        return Err("IFC contains no STEP entities".into());
    }

    let mm_scale = detect_length_scale_mm(&entities);
    let mut point_lists: HashMap<u64, Vec<[f32; 3]>> = HashMap::new();
    let mut polygon_faces: HashMap<u64, Vec<usize>> = HashMap::new();

    for entity in &entities {
        match entity.kind.as_str() {
            "IFCCARTESIANPOINTLIST3D" => {
                if let Some(points) = parse_point_list_3d(&entity.args, mm_scale) {
                    point_lists.insert(entity.id, points);
                }
            }
            "IFCINDEXEDPOLYGONALFACE" | "IFCINDEXEDPOLYGONALFACEWITHVOIDS" => {
                if let Some(indices) = parse_indexed_face(&entity.args) {
                    polygon_faces.insert(entity.id, indices);
                }
            }
            _ => {}
        }
    }

    let mut objects = Vec::new();
    for entity in &entities {
        match entity.kind.as_str() {
            "IFCTRIANGULATEDFACESET" => {
                if let Some(object) = triangulated_face_set(entity, &point_lists)? {
                    objects.push(object);
                }
            }
            "IFCPOLYGONALFACESET" => {
                if let Some(object) = polygonal_face_set(entity, &point_lists, &polygon_faces)? {
                    objects.push(object);
                }
            }
            _ => {}
        }
    }

    if objects.is_empty() {
        return Err(
            "IFC has no supported tessellated geometry. v0.2.2 imports IFC4 IfcTriangulatedFaceSet and IfcPolygonalFaceSet geometry; BRep/swept-solid IFC representation conversion is not enabled in this build."
                .into(),
        );
    }

    let mut scene = SceneDocument::new(
        path.file_name().and_then(|v| v.to_str()).unwrap_or("IFC model"),
        objects,
    )?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

fn triangulated_face_set(
    entity: &Entity,
    point_lists: &HashMap<u64, Vec<[f32; 3]>>,
) -> Result<Option<SceneObject>, String> {
    let Some(args) = as_list(&entity.args) else { return Ok(None); };
    let Some(coord_id) = args.first().and_then(as_ref) else { return Ok(None); };
    let Some(points) = point_lists.get(&coord_id) else { return Ok(None); };

    // IFC4 schema: Coordinates, Normals, Closed, CoordIndex, PnIndex.
    let coord_index = args.get(3).and_then(as_list).or_else(|| {
        args.iter().skip(1).find_map(|v| {
            let list = as_list(v)?;
            list.iter().all(|row| as_list(row).is_some()).then_some(list)
        })
    });
    let Some(rows) = coord_index else { return Ok(None); };

    let pn_index: Option<Vec<usize>> = args.get(4).and_then(as_list).map(|v| v.iter().filter_map(as_usize).collect());
    let mut indices = Vec::new();
    for row in rows {
        let Some(values) = as_list(row) else { continue; };
        let face: Vec<usize> = values.iter().filter_map(as_usize).collect();
        if face.len() < 3 { continue; }
        let a = resolve_point_index(face[0], points.len(), pn_index.as_deref())?;
        for i in 1..face.len() - 1 {
            indices.push(a as u32);
            indices.push(resolve_point_index(face[i], points.len(), pn_index.as_deref())? as u32);
            indices.push(resolve_point_index(face[i + 1], points.len(), pn_index.as_deref())? as u32);
        }
    }
    build_object(entity.id, "IFC TriangulatedFaceSet", points, indices)
}

fn polygonal_face_set(
    entity: &Entity,
    point_lists: &HashMap<u64, Vec<[f32; 3]>>,
    polygon_faces: &HashMap<u64, Vec<usize>>,
) -> Result<Option<SceneObject>, String> {
    let Some(args) = as_list(&entity.args) else { return Ok(None); };
    let Some(coord_id) = args.first().and_then(as_ref) else { return Ok(None); };
    let Some(points) = point_lists.get(&coord_id) else { return Ok(None); };

    // IFC4 schema: Coordinates, Closed, Faces, PnIndex.
    let Some(face_refs) = args.get(2).and_then(as_list) else { return Ok(None); };
    let pn_index: Option<Vec<usize>> = args.get(3).and_then(as_list).map(|v| v.iter().filter_map(as_usize).collect());
    let mut indices = Vec::new();
    for face_ref in face_refs {
        let Some(face_id) = as_ref(face_ref) else { continue; };
        let Some(face) = polygon_faces.get(&face_id) else { continue; };
        if face.len() < 3 { continue; }
        let a = resolve_point_index(face[0], points.len(), pn_index.as_deref())?;
        for i in 1..face.len() - 1 {
            indices.push(a as u32);
            indices.push(resolve_point_index(face[i], points.len(), pn_index.as_deref())? as u32);
            indices.push(resolve_point_index(face[i + 1], points.len(), pn_index.as_deref())? as u32);
        }
    }
    build_object(entity.id, "IFC PolygonalFaceSet", points, indices)
}

fn build_object(
    id: u64,
    kind: &str,
    points: &[[f32; 3]],
    indices: Vec<u32>,
) -> Result<Option<SceneObject>, String> {
    if indices.len() < 3 { return Ok(None); }
    let vertices = points
        .iter()
        .copied()
        .map(|position| Vertex { position, normal: [0.0; 3], texcoord: [0.0; 2] })
        .collect();
    let name = format!("{kind} #{id}");
    let mut mesh = SceneMesh::new(&name, vertices, indices)?;
    mesh.material_count = 1;
    mesh.material_name = "IFC tessellated geometry".into();
    Ok(Some(SceneObject::new(name, mesh)))
}

fn parse_point_list_3d(value: &Value, scale: f64) -> Option<Vec<[f32; 3]>> {
    let args = as_list(value)?;
    let rows = as_list(args.first()?)?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let xyz = as_list(row)?;
        if xyz.len() < 3 { return None; }
        result.push([
            (as_f64(&xyz[0])? * scale) as f32,
            (as_f64(&xyz[1])? * scale) as f32,
            (as_f64(&xyz[2])? * scale) as f32,
        ]);
    }
    (!result.is_empty()).then_some(result)
}

fn parse_indexed_face(value: &Value) -> Option<Vec<usize>> {
    let args = as_list(value)?;
    let boundary = as_list(args.first()?)?;
    let indices: Vec<usize> = boundary.iter().filter_map(as_usize).collect();
    (indices.len() >= 3).then_some(indices)
}

fn detect_length_scale_mm(entities: &[Entity]) -> f64 {
    for entity in entities {
        if entity.kind != "IFCSIUNIT" { continue; }
        let Some(args) = as_list(&entity.args) else { continue; };
        let atoms: Vec<String> = args.iter().filter_map(|v| match v { Value::Atom(a) => Some(a.to_ascii_uppercase()), _ => None }).collect();
        if !atoms.iter().any(|v| v == ".LENGTHUNIT.") || !atoms.iter().any(|v| v == ".METRE.") { continue; }
        if atoms.iter().any(|v| v == ".MILLI.") { return 1.0; }
        if atoms.iter().any(|v| v == ".CENTI.") { return 10.0; }
        if atoms.iter().any(|v| v == ".DECI.") { return 100.0; }
        if atoms.iter().any(|v| v == ".MICRO.") { return 0.001; }
        if atoms.iter().any(|v| v == ".KILO.") { return 1_000_000.0; }
        return 1000.0; // metre -> internal millimetres
    }
    1000.0
}

fn parse_entities(text: &str) -> Result<Vec<Entity>, String> {
    let mut entities = Vec::new();
    for statement in split_statements(text) {
        let s = statement.trim();
        if !s.starts_with('#') { continue; }
        let Some(eq) = s.find('=') else { continue; };
        let id: u64 = s[1..eq].trim().parse().map_err(|_| format!("Invalid IFC entity id in {s}"))?;
        let rhs = s[eq + 1..].trim();
        let Some(open) = rhs.find('(') else { continue; };
        let kind = rhs[..open].trim().to_ascii_uppercase();
        let Some(close) = rhs.rfind(')') else { continue; };
        if close <= open { continue; }
        let wrapped = &rhs[open..=close];
        let args = ValueParser::new(wrapped).parse_value()?;
        entities.push(Entity { id, kind, args });
    }
    Ok(entities)
}

fn split_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            current.push(c);
            if in_string && chars.peek() == Some(&'\'') {
                current.push(chars.next().unwrap());
                continue;
            }
            in_string = !in_string;
            continue;
        }
        if c == ';' && !in_string {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() { out.push(current); }
    out
}

fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
            i = (i + 2).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

struct ValueParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ValueParser<'a> {
    fn new(s: &'a str) -> Self { Self { bytes: s.as_bytes(), pos: 0 } }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        if self.pos >= self.bytes.len() { return Err("Unexpected end of IFC value".into()); }
        if self.bytes[self.pos] == b'(' { return self.parse_list(); }
        self.parse_atom()
    }

    fn parse_list(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut values = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.bytes.len() { return Err("Unclosed IFC list".into()); }
            if self.bytes[self.pos] == b')' { self.pos += 1; break; }
            values.push(self.parse_value()?);
            self.skip_ws();
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b',' { self.pos += 1; continue; }
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b')' { self.pos += 1; break; }
            return Err("Invalid IFC list separator".into());
        }
        Ok(Value::List(values))
    }

    fn parse_atom(&mut self) -> Result<Value, String> {
        let start = self.pos;
        let mut in_string = false;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == b'\'' {
                if in_string && self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'\'' {
                    self.pos += 2;
                    continue;
                }
                in_string = !in_string;
                self.pos += 1;
                continue;
            }
            if !in_string && (c == b',' || c == b')') { break; }
            self.pos += 1;
        }
        let atom = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| "Invalid UTF-8 in IFC token")?.trim();
        if atom.is_empty() { return Err("Empty IFC atom".into()); }
        Ok(Value::Atom(atom.to_string()))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() { self.pos += 1; }
    }
}

fn as_list(value: &Value) -> Option<&[Value]> { match value { Value::List(v) => Some(v), _ => None } }
fn as_ref(value: &Value) -> Option<u64> { match value { Value::Atom(a) => a.strip_prefix('#')?.trim().parse().ok(), _ => None } }
fn as_usize(value: &Value) -> Option<usize> { match value { Value::Atom(a) => a.trim().parse().ok(), _ => None } }
fn as_f64(value: &Value) -> Option<f64> { match value { Value::Atom(a) => a.trim().parse().ok(), _ => None } }

fn resolve_point_index(index: usize, len: usize, pn_index: Option<&[usize]>) -> Result<usize, String> {
    let coordinate_index = if let Some(mapping) = pn_index {
        if index == 0 || index > mapping.len() {
            return Err(format!("IFC face index {index} is outside PnIndex of {} entries", mapping.len()));
        }
        mapping[index - 1]
    } else {
        index
    };
    if coordinate_index == 0 || coordinate_index > len {
        return Err(format!("IFC coordinate index {coordinate_index} is outside point list of {len} vertices"));
    }
    Ok(coordinate_index - 1)
}
