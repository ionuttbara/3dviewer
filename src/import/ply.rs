use std::{fs, io::{Cursor, Read}, path::Path};

use crate::mesh::{SceneMesh, Vertex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlyFormat { Ascii, BinaryLe, BinaryBe }
#[derive(Clone, Copy, Debug)]
enum ScalarType { I8, U8, I16, U16, I32, U32, F32, F64 }
#[derive(Clone, Debug)]
enum Property {
    Scalar { ty: ScalarType, name: String },
    List { count: ScalarType, item: ScalarType, name: String },
}
#[derive(Clone, Debug)]
struct Element { name: String, count: usize, properties: Vec<Property> }

pub fn load(path: &Path) -> Result<SceneMesh, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let (header, data_start) = split_header(&bytes)?;
    let (format, elements) = parse_header(header)?;
    let (vertices, indices) = match format {
        PlyFormat::Ascii => parse_ascii(&bytes[data_start..], &elements)?,
        PlyFormat::BinaryLe => parse_binary(&bytes[data_start..], &elements, true)?,
        PlyFormat::BinaryBe => parse_binary(&bytes[data_start..], &elements, false)?,
    };
    let mut mesh = SceneMesh::new(path.file_name().and_then(|v| v.to_str()).unwrap_or("PLY model"), vertices, indices)?;
    if mesh.vertices.iter().all(|v| v.normal == [0.0; 3]) { mesh.recalculate_normals(); }
    Ok(mesh)
}

fn split_header(bytes: &[u8]) -> Result<(&str, usize), String> {
    let marker_lf = b"end_header\n";
    let marker_crlf = b"end_header\r\n";
    let (end, marker_len) = find_subslice(bytes, marker_crlf).map(|i| (i, marker_crlf.len()))
        .or_else(|| find_subslice(bytes, marker_lf).map(|i| (i, marker_lf.len())))
        .ok_or("PLY header is incomplete")?;
    let header_end = end + marker_len;
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| "PLY header is not UTF-8/ASCII")?;
    Ok((header, header_end))
}

fn parse_header(header: &str) -> Result<(PlyFormat, Vec<Element>), String> {
    let mut lines = header.lines();
    if lines.next().map(str::trim) != Some("ply") { return Err("Invalid PLY header".into()); }
    let mut format = None;
    let mut elements = Vec::<Element>::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("comment ") || line.starts_with("obj_info ") { continue; }
        if line == "end_header" { break; }
        if let Some(rest) = line.strip_prefix("format ") {
            let kind = rest.split_whitespace().next().unwrap_or("");
            format = Some(match kind {
                "ascii" => PlyFormat::Ascii,
                "binary_little_endian" => PlyFormat::BinaryLe,
                "binary_big_endian" => PlyFormat::BinaryBe,
                _ => return Err(format!("Unsupported PLY format '{kind}'")),
            });
        } else if let Some(rest) = line.strip_prefix("element ") {
            let mut p = rest.split_whitespace();
            let name = p.next().ok_or("PLY element name missing")?.to_string();
            let count = p.next().ok_or("PLY element count missing")?.parse::<usize>().map_err(|_| "Invalid PLY element count")?;
            elements.push(Element { name, count, properties: Vec::new() });
        } else if let Some(rest) = line.strip_prefix("property ") {
            let element = elements.last_mut().ok_or("PLY property appears before an element")?;
            let mut p = rest.split_whitespace();
            let first = p.next().ok_or("PLY property type missing")?;
            if first == "list" {
                let count = parse_type(p.next().ok_or("PLY list count type missing")?)?;
                let item = parse_type(p.next().ok_or("PLY list item type missing")?)?;
                let name = p.next().ok_or("PLY list property name missing")?.to_string();
                element.properties.push(Property::List { count, item, name });
            } else {
                let ty = parse_type(first)?;
                let name = p.next().ok_or("PLY property name missing")?.to_string();
                element.properties.push(Property::Scalar { ty, name });
            }
        }
    }
    Ok((format.ok_or("PLY format declaration missing")?, elements))
}

fn parse_ascii(data: &[u8], elements: &[Element]) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let text = std::str::from_utf8(data).map_err(|_| "ASCII PLY data is not UTF-8/ASCII")?;
    let mut tokens = text.split_whitespace();
    let mut vertices = Vec::<Vertex>::new();
    let mut indices = Vec::<u32>::new();
    for element in elements {
        for _ in 0..element.count {
            let mut vertex = Vertex { position: [0.0; 3], normal: [0.0; 3], texcoord: [0.0; 2] };
            let mut face = None::<Vec<u32>>;
            for property in &element.properties {
                match property {
                    Property::Scalar { name, .. } => {
                        let token = tokens.next().ok_or("PLY ASCII data is truncated")?;
                        if element.name == "vertex" { assign_vertex_scalar(&mut vertex, name, token.parse::<f64>().map_err(|_| "Invalid PLY scalar")? as f32); }
                    }
                    Property::List { name, .. } => {
                        let count = tokens.next().ok_or("PLY ASCII list count missing")?.parse::<usize>().map_err(|_| "Invalid PLY list count")?;
                        let mut values = Vec::<u32>::with_capacity(count);
                        for _ in 0..count {
                            let token = tokens.next().ok_or("PLY ASCII list data is truncated")?;
                            values.push(token.parse::<u32>().map_err(|_| "Invalid PLY list index")?);
                        }
                        if element.name == "face" && is_vertex_index_name(name) { face = Some(values); }
                    }
                }
            }
            if element.name == "vertex" { vertices.push(vertex); }
            if let Some(face) = face { triangulate_face(&face, &mut indices); }
        }
    }
    validate_indices(&vertices, &indices)?;
    Ok((vertices, indices))
}

fn parse_binary(data: &[u8], elements: &[Element], little_endian: bool) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let mut cursor = Cursor::new(data);
    let mut vertices = Vec::<Vertex>::new();
    let mut indices = Vec::<u32>::new();
    for element in elements {
        for _ in 0..element.count {
            let mut vertex = Vertex { position: [0.0; 3], normal: [0.0; 3], texcoord: [0.0; 2] };
            let mut face = None::<Vec<u32>>;
            for property in &element.properties {
                match property {
                    Property::Scalar { ty, name } => {
                        let value = read_scalar_f64(&mut cursor, *ty, little_endian)? as f32;
                        if element.name == "vertex" { assign_vertex_scalar(&mut vertex, name, value); }
                    }
                    Property::List { count, item, name } => {
                        let n = read_scalar_u64(&mut cursor, *count, little_endian)? as usize;
                        if n > 10_000_000 { return Err("PLY list count is unreasonable".into()); }
                        let mut values = if element.name == "face" && is_vertex_index_name(name) { Some(Vec::with_capacity(n)) } else { None };
                        for _ in 0..n {
                            let value = read_scalar_u64(&mut cursor, *item, little_endian)?;
                            if let Some(ref mut out) = values { out.push(value as u32); }
                        }
                        if values.is_some() { face = values; }
                    }
                }
            }
            if element.name == "vertex" { vertices.push(vertex); }
            if let Some(face) = face { triangulate_face(&face, &mut indices); }
        }
    }
    validate_indices(&vertices, &indices)?;
    Ok((vertices, indices))
}

fn assign_vertex_scalar(vertex: &mut Vertex, name: &str, value: f32) {
    match name.to_ascii_lowercase().as_str() {
        "x" => vertex.position[0] = value,
        "y" => vertex.position[1] = value,
        "z" => vertex.position[2] = value,
        "nx" => vertex.normal[0] = value,
        "ny" => vertex.normal[1] = value,
        "nz" => vertex.normal[2] = value,
        "u" | "s" | "texture_u" | "texture_s" => vertex.texcoord[0] = value,
        "v" | "t" | "texture_v" | "texture_t" => vertex.texcoord[1] = value,
        _ => {}
    }
}
fn is_vertex_index_name(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "vertex_indices" | "vertex_index")
}
fn triangulate_face(face: &[u32], indices: &mut Vec<u32>) {
    if face.len() < 3 { return; }
    for i in 1..face.len() - 1 { indices.extend_from_slice(&[face[0], face[i], face[i + 1]]); }
}
fn validate_indices(vertices: &[Vertex], indices: &[u32]) -> Result<(), String> {
    if indices.iter().any(|&i| i as usize >= vertices.len()) { return Err("PLY face index is out of range".into()); }
    Ok(())
}

fn parse_type(value: &str) -> Result<ScalarType, String> {
    match value.to_ascii_lowercase().as_str() {
        "char" | "int8" => Ok(ScalarType::I8),
        "uchar" | "uint8" => Ok(ScalarType::U8),
        "short" | "int16" => Ok(ScalarType::I16),
        "ushort" | "uint16" => Ok(ScalarType::U16),
        "int" | "int32" => Ok(ScalarType::I32),
        "uint" | "uint32" => Ok(ScalarType::U32),
        "float" | "float32" => Ok(ScalarType::F32),
        "double" | "float64" => Ok(ScalarType::F64),
        other => Err(format!("Unsupported PLY scalar type '{other}'")),
    }
}
fn read_scalar_f64(cursor: &mut Cursor<&[u8]>, ty: ScalarType, le: bool) -> Result<f64, String> {
    Ok(match ty {
        ScalarType::I8 => read_arr::<1>(cursor)?[0] as i8 as f64,
        ScalarType::U8 => read_arr::<1>(cursor)?[0] as f64,
        ScalarType::I16 => (if le { i16::from_le_bytes(read_arr(cursor)?) } else { i16::from_be_bytes(read_arr(cursor)?) }) as f64,
        ScalarType::U16 => (if le { u16::from_le_bytes(read_arr(cursor)?) } else { u16::from_be_bytes(read_arr(cursor)?) }) as f64,
        ScalarType::I32 => (if le { i32::from_le_bytes(read_arr(cursor)?) } else { i32::from_be_bytes(read_arr(cursor)?) }) as f64,
        ScalarType::U32 => (if le { u32::from_le_bytes(read_arr(cursor)?) } else { u32::from_be_bytes(read_arr(cursor)?) }) as f64,
        ScalarType::F32 => (if le { f32::from_le_bytes(read_arr(cursor)?) } else { f32::from_be_bytes(read_arr(cursor)?) }) as f64,
        ScalarType::F64 => if le { f64::from_le_bytes(read_arr(cursor)?) } else { f64::from_be_bytes(read_arr(cursor)?) },
    })
}
fn read_scalar_u64(cursor: &mut Cursor<&[u8]>, ty: ScalarType, le: bool) -> Result<u64, String> {
    let value = read_scalar_f64(cursor, ty, le)?;
    if !value.is_finite() || value < 0.0 { return Err("PLY list count/index must be a non-negative integer".into()); }
    Ok(value as u64)
}
fn read_arr<const N: usize>(cursor: &mut Cursor<&[u8]>) -> Result<[u8; N], String> {
    let mut buf = [0u8; N];
    cursor.read_exact(&mut buf).map_err(|_| "Binary PLY data is truncated".to_string())?;
    Ok(buf)
}
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}
