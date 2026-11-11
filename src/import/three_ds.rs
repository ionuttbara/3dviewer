use std::{collections::{HashMap, HashSet}, fs, path::Path};

use glam::Vec3;

use crate::{
    assets::resolve_referenced_texture,
    mesh::{SceneDocument, SceneMesh, SceneObject, Vertex},
};

#[derive(Clone, Debug)]
struct Material3ds {
    name: String,
    diffuse: [f32; 4],
    texture: Option<String>,
}

impl Default for Material3ds {
    fn default() -> Self {
        Self { name: "Default".into(), diffuse: [0.70, 0.72, 0.76, 1.0], texture: None }
    }
}

#[derive(Clone, Debug, Default)]
struct Object3ds {
    name: String,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    faces: Vec<[u16; 3]>,
    material_faces: HashMap<String, Vec<usize>>,
    local: Option<[[f32; 3]; 4]>, // X axis, Y axis, Z axis, origin
}

#[derive(Default)]
struct ParseState {
    materials: HashMap<String, Material3ds>,
    objects: Vec<Object3ds>,
    master_scale: f32,
}

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 6 { return Err("3DS file is too small".into()); }
    let mut state = ParseState { master_scale: 1.0, ..Default::default() };
    parse_root(&bytes, 0, bytes.len(), &mut state)?;
    if state.objects.is_empty() { return Err("3DS contains no triangle-mesh objects".into()); }

    let mut objects = Vec::<SceneObject>::new();
    for object in state.objects {
        append_object(path, &state.materials, state.master_scale, object, &mut objects)?;
    }
    if objects.is_empty() { return Err("3DS contains no renderable triangle faces".into()); }

    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("3DS model");
    let mut scene = SceneDocument::new(name, objects)?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

fn parse_root(data: &[u8], start: usize, end: usize, state: &mut ParseState) -> Result<(), String> {
    let mut pos = start;
    while pos + 6 <= end {
        let id = read_u16(data, pos)?;
        let len = read_u32(data, pos + 2)? as usize;
        if len < 6 || pos + len > end { return Err(format!("Invalid 3DS chunk 0x{id:04X}")); }
        let body = pos + 6;
        let chunk_end = pos + len;
        match id {
            0x4D4D | 0x3D3D => parse_root(data, body, chunk_end, state)?,
            0x0100 => {
                if body + 4 <= chunk_end {
                    let scale = read_f32(data, body)?;
                    if scale.is_finite() && scale > 0.0 { state.master_scale = scale; }
                }
            }
            0xAFFF => {
                let mat = parse_material(data, body, chunk_end)?;
                state.materials.insert(mat.name.clone(), mat);
            }
            0x4000 => {
                let (name, child_start) = read_c_string(data, body, chunk_end)?;
                let mut object = Object3ds { name, ..Default::default() };
                parse_object_chunks(data, child_start, chunk_end, &mut object)?;
                if !object.positions.is_empty() && !object.faces.is_empty() { state.objects.push(object); }
            }
            _ => {}
        }
        pos = chunk_end;
    }
    Ok(())
}

fn parse_material(data: &[u8], start: usize, end: usize) -> Result<Material3ds, String> {
    let mut mat = Material3ds::default();
    let mut pos = start;
    while pos + 6 <= end {
        let id = read_u16(data, pos)?;
        let len = read_u32(data, pos + 2)? as usize;
        if len < 6 || pos + len > end { return Err("Invalid 3DS material chunk".into()); }
        let body = pos + 6;
        let chunk_end = pos + len;
        match id {
            0xA000 => mat.name = read_c_string(data, body, chunk_end)?.0,
            0xA020 => mat.diffuse = parse_color(data, body, chunk_end).unwrap_or(mat.diffuse),
            0xA200 => mat.texture = parse_texture_map(data, body, chunk_end)?,
            _ => {}
        }
        pos = chunk_end;
    }
    Ok(mat)
}

fn parse_texture_map(data: &[u8], start: usize, end: usize) -> Result<Option<String>, String> {
    let mut pos = start;
    while pos + 6 <= end {
        let id = read_u16(data, pos)?;
        let len = read_u32(data, pos + 2)? as usize;
        if len < 6 || pos + len > end { return Err("Invalid 3DS texture-map chunk".into()); }
        if id == 0xA300 {
            return Ok(Some(read_c_string(data, pos + 6, pos + len)?.0));
        }
        pos += len;
    }
    Ok(None)
}

fn parse_color(data: &[u8], start: usize, end: usize) -> Option<[f32; 4]> {
    let mut pos = start;
    while pos + 6 <= end {
        let id = read_u16(data, pos).ok()?;
        let len = read_u32(data, pos + 2).ok()? as usize;
        if len < 6 || pos + len > end { return None; }
        let body = pos + 6;
        let c = match id {
            0x0010 | 0x0013 if body + 12 <= pos + len => Some([
                read_f32(data, body).ok()?.clamp(0.0, 1.0),
                read_f32(data, body + 4).ok()?.clamp(0.0, 1.0),
                read_f32(data, body + 8).ok()?.clamp(0.0, 1.0),
                1.0,
            ]),
            0x0011 | 0x0012 if body + 3 <= pos + len => Some([
                data[body] as f32 / 255.0,
                data[body + 1] as f32 / 255.0,
                data[body + 2] as f32 / 255.0,
                1.0,
            ]),
            _ => None,
        };
        if c.is_some() { return c; }
        pos += len;
    }
    None
}

fn parse_object_chunks(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    let mut pos = start;
    while pos + 6 <= end {
        let id = read_u16(data, pos)?;
        let len = read_u32(data, pos + 2)? as usize;
        if len < 6 || pos + len > end { return Err(format!("Invalid 3DS object chunk 0x{id:04X}")); }
        let body = pos + 6;
        let chunk_end = pos + len;
        match id {
            0x4100 => parse_object_chunks(data, body, chunk_end, object)?,
            0x4110 => parse_vertices(data, body, chunk_end, object)?,
            0x4120 => parse_faces(data, body, chunk_end, object)?,
            0x4140 => parse_uvs(data, body, chunk_end, object)?,
            0x4160 => parse_local_matrix(data, body, chunk_end, object)?,
            _ => {}
        }
        pos = chunk_end;
    }
    Ok(())
}

fn parse_vertices(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    if start + 2 > end { return Err("3DS vertex list is truncated".into()); }
    let count = read_u16(data, start)? as usize;
    if start + 2 + count * 12 > end { return Err("3DS vertex list is truncated".into()); }
    object.positions.clear();
    object.positions.reserve(count);
    let mut p = start + 2;
    for _ in 0..count {
        object.positions.push([read_f32(data,p)?, read_f32(data,p+4)?, read_f32(data,p+8)?]);
        p += 12;
    }
    Ok(())
}

fn parse_faces(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    if start + 2 > end { return Err("3DS face list is truncated".into()); }
    let count = read_u16(data, start)? as usize;
    let face_bytes = count.checked_mul(8).ok_or("3DS face count overflow")?;
    if start + 2 + face_bytes > end { return Err("3DS face list is truncated".into()); }
    object.faces.clear();
    object.faces.reserve(count);
    let mut p = start + 2;
    for _ in 0..count {
        object.faces.push([read_u16(data,p)?, read_u16(data,p+2)?, read_u16(data,p+4)?]);
        p += 8; // fourth u16 is edge visibility flags
    }

    // 0x4120 can contain subchunks after the face array.
    while p + 6 <= end {
        let id = read_u16(data,p)?;
        let len = read_u32(data,p+2)? as usize;
        if len < 6 || p + len > end { return Err("Invalid 3DS face subchunk".into()); }
        if id == 0x4130 { parse_face_material(data,p+6,p+len,object)?; }
        p += len;
    }
    Ok(())
}

fn parse_face_material(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    let (name, mut p) = read_c_string(data,start,end)?;
    if p + 2 > end { return Err("3DS face-material assignment is truncated".into()); }
    let count = read_u16(data,p)? as usize;
    p += 2;
    if p + count * 2 > end { return Err("3DS face-material assignment is truncated".into()); }
    let list = object.material_faces.entry(name).or_default();
    for _ in 0..count { list.push(read_u16(data,p)? as usize); p += 2; }
    Ok(())
}

fn parse_uvs(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    if start + 2 > end { return Err("3DS mapping coordinates are truncated".into()); }
    let count = read_u16(data,start)? as usize;
    if start + 2 + count * 8 > end { return Err("3DS mapping coordinates are truncated".into()); }
    object.uvs.clear(); object.uvs.reserve(count);
    let mut p=start+2;
    for _ in 0..count { object.uvs.push([read_f32(data,p)?, 1.0-read_f32(data,p+4)?]); p+=8; }
    Ok(())
}

fn parse_local_matrix(data: &[u8], start: usize, end: usize, object: &mut Object3ds) -> Result<(), String> {
    if start + 48 > end { return Ok(()); }
    let mut vals=[0.0f32;12];
    for (i,v) in vals.iter_mut().enumerate(){*v=read_f32(data,start+i*4)?;}
    object.local=Some([
        [vals[0],vals[1],vals[2]],
        [vals[3],vals[4],vals[5]],
        [vals[6],vals[7],vals[8]],
        [vals[9],vals[10],vals[11]],
    ]);
    Ok(())
}

fn append_object(
    model_path: &Path,
    materials: &HashMap<String, Material3ds>,
    master_scale: f32,
    object: Object3ds,
    output: &mut Vec<SceneObject>,
) -> Result<(), String> {
    let mut assigned = HashSet::<usize>::new();
    for list in object.material_faces.values() { assigned.extend(list.iter().copied()); }

    if object.material_faces.is_empty() {
        let all: Vec<usize>=(0..object.faces.len()).collect();
        append_face_group(model_path, materials, master_scale, &object, "Default", &all, output)?;
    } else {
        for (material, faces) in &object.material_faces {
            append_face_group(model_path, materials, master_scale, &object, material, faces, output)?;
        }
        let unassigned:Vec<usize>=(0..object.faces.len()).filter(|i|!assigned.contains(i)).collect();
        if !unassigned.is_empty(){append_face_group(model_path,materials,master_scale,&object,"Default",&unassigned,output)?;}
    }
    Ok(())
}

fn append_face_group(
    model_path:&Path,
    materials:&HashMap<String,Material3ds>,
    master_scale:f32,
    object:&Object3ds,
    material_name:&str,
    face_ids:&[usize],
    output:&mut Vec<SceneObject>,
)->Result<(),String>{
    if face_ids.is_empty(){return Ok(());}
    let scale=if master_scale.is_finite() && master_scale>0.0 {master_scale}else{1.0};
    let mut vertices=Vec::<Vertex>::with_capacity(object.positions.len());
    for (i,p) in object.positions.iter().enumerate(){
        let mut v=Vec3::from_array(*p);
        if let Some(m)=object.local {
            let x=Vec3::from_array(m[0]); let y=Vec3::from_array(m[1]); let z=Vec3::from_array(m[2]); let o=Vec3::from_array(m[3]);
            v=o+x*v.x+y*v.y+z*v.z;
        }
        v*=scale;
        vertices.push(Vertex{position:v.to_array(),normal:[0.0;3],texcoord:object.uvs.get(i).copied().unwrap_or([0.0;2])});
    }
    let mut indices=Vec::<u32>::with_capacity(face_ids.len()*3);
    for &fi in face_ids {
        let Some(face)=object.faces.get(fi) else {continue;};
        if face.iter().any(|&v|v as usize>=vertices.len()){continue;}
        indices.extend_from_slice(&[face[0] as u32,face[1] as u32,face[2] as u32]);
    }
    if indices.is_empty(){return Ok(());}
    let name=if material_name=="Default" {object.name.clone()} else {format!("{} • {}",object.name,material_name)};
    let mut mesh=SceneMesh::new(name.clone(),vertices,indices)?;
    if let Some(mat)=materials.get(material_name){
        mesh.material_name=mat.name.clone(); mesh.material_count=1; mesh.base_color_factor=mat.diffuse;
        if let Some(tex)=&mat.texture {
            if let Some(resolved)=resolve_referenced_texture(model_path,tex){mesh.base_color_texture=Some(resolved.clone());mesh.discovered_textures.push(resolved);}
        }
    }
    output.push(SceneObject::new(name,mesh));
    Ok(())
}

fn read_u16(data:&[u8],pos:usize)->Result<u16,String>{
    let b=data.get(pos..pos+2).ok_or("Unexpected end of 3DS file")?;
    Ok(u16::from_le_bytes([b[0],b[1]]))
}
fn read_u32(data:&[u8],pos:usize)->Result<u32,String>{
    let b=data.get(pos..pos+4).ok_or("Unexpected end of 3DS file")?;
    Ok(u32::from_le_bytes([b[0],b[1],b[2],b[3]]))
}
fn read_f32(data:&[u8],pos:usize)->Result<f32,String>{
    let b=data.get(pos..pos+4).ok_or("Unexpected end of 3DS file")?;
    Ok(f32::from_le_bytes([b[0],b[1],b[2],b[3]]))
}
fn read_c_string(data:&[u8],start:usize,end:usize)->Result<(String,usize),String>{
    let mut p=start;
    while p<end && data[p]!=0{p+=1;}
    if p>=end{return Err("Unterminated string in 3DS file".into());}
    let s=String::from_utf8_lossy(&data[start..p]).to_string();
    Ok((s,p+1))
}
