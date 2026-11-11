use std::{fs, path::Path};
use crate::mesh::{SceneMesh, Vertex};

pub fn load(path: &Path) -> Result<SceneMesh, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    load_bytes(path.file_name().and_then(|v| v.to_str()).unwrap_or("STL model"), &bytes)
}

pub fn load_bytes(name: &str, bytes: &[u8]) -> Result<SceneMesh, String> {
    let (vertices, indices) = if looks_binary(bytes) { parse_binary(bytes)? } else { parse_ascii(bytes)? };
    let mut mesh = SceneMesh::new(name, vertices, indices)?;
    mesh.recalculate_normals();
    Ok(mesh)
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 { return false; }
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    84usize.saturating_add(count.saturating_mul(50)) <= bytes.len()
}

fn parse_binary(bytes: &[u8]) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let mut vertices = Vec::with_capacity(count * 3);
    let mut indices = Vec::with_capacity(count * 3);
    let mut pos = 84usize;
    for _ in 0..count {
        if pos + 50 > bytes.len() { return Err("Binary STL is truncated".into()); }
        let normal = [f32le(bytes,pos), f32le(bytes,pos+4), f32le(bytes,pos+8)];
        pos += 12;
        for _ in 0..3 {
            let p = [f32le(bytes,pos), f32le(bytes,pos+4), f32le(bytes,pos+8)];
            let id = vertices.len() as u32;
            vertices.push(Vertex { position:p, normal, texcoord:[0.0;2] });
            indices.push(id);
            pos += 12;
        }
        pos += 2;
    }
    Ok((vertices, indices))
}

fn f32le(bytes: &[u8], pos: usize) -> f32 { f32::from_le_bytes(bytes[pos..pos+4].try_into().unwrap()) }

fn parse_ascii(bytes: &[u8]) -> Result<(Vec<Vertex>, Vec<u32>), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "STL is neither valid binary nor ASCII")?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("vertex ") {
            let mut p = rest.split_whitespace();
            let x: f32 = p.next().ok_or("STL vertex missing X")?.parse().map_err(|_| "Invalid STL vertex")?;
            let y: f32 = p.next().ok_or("STL vertex missing Y")?.parse().map_err(|_| "Invalid STL vertex")?;
            let z: f32 = p.next().ok_or("STL vertex missing Z")?.parse().map_err(|_| "Invalid STL vertex")?;
            let id = vertices.len() as u32;
            vertices.push(Vertex { position:[x,y,z], normal:[0.0;3], texcoord:[0.0;2] });
            indices.push(id);
        }
    }
    if indices.len() % 3 != 0 { return Err("Malformed ASCII STL triangle list".into()); }
    Ok((vertices, indices))
}
