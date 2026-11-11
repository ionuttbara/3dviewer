use std::{fs, path::Path};
use crate::mesh::{SceneMesh, Vertex};

pub fn load(path: &Path) -> Result<SceneMesh, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let clean: String = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let mut t = clean.split_whitespace();
    if t.next() != Some("OFF") { return Err("Only OFF polygon files are supported in v0.2.2".into()); }
    let nv: usize = t.next().ok_or("OFF vertex count missing")?.parse().map_err(|_| "Invalid OFF header")?;
    let nf: usize = t.next().ok_or("OFF face count missing")?.parse().map_err(|_| "Invalid OFF header")?;
    let _ne = t.next();
    let mut vertices = Vec::with_capacity(nv);
    for _ in 0..nv {
        let x: f32 = t.next().ok_or("OFF vertex truncated")?.parse().map_err(|_| "Invalid OFF vertex")?;
        let y: f32 = t.next().ok_or("OFF vertex truncated")?.parse().map_err(|_| "Invalid OFF vertex")?;
        let z: f32 = t.next().ok_or("OFF vertex truncated")?.parse().map_err(|_| "Invalid OFF vertex")?;
        vertices.push(Vertex { position:[x,y,z], normal:[0.0;3], texcoord:[0.0;2] });
    }
    let mut indices = Vec::new();
    for _ in 0..nf {
        let n: usize = t.next().ok_or("OFF face truncated")?.parse().map_err(|_| "Invalid OFF face")?;
        let mut face = Vec::with_capacity(n);
        for _ in 0..n { face.push(t.next().ok_or("OFF face index missing")?.parse::<u32>().map_err(|_| "Invalid OFF index")?); }
        for i in 1..face.len().saturating_sub(1) { indices.extend_from_slice(&[face[0], face[i], face[i+1]]); }
    }
    let mut mesh = SceneMesh::new(path.file_name().and_then(|v| v.to_str()).unwrap_or("OFF model"), vertices, indices)?;
    mesh.recalculate_normals();
    Ok(mesh)
}
