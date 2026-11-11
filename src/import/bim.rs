use std::{collections::HashMap, fs, path::Path};

use glam::{Mat4, Quat, Vec3};
use serde_json::Value;

use crate::mesh::{SceneDocument, SceneMesh, SceneObject, Vertex};

#[derive(Clone)]
struct BimMesh { positions_m: Vec<[f32;3]>, indices: Vec<u32> }

pub fn load_scene(path:&Path)->Result<SceneDocument,String>{
    let text=fs::read_to_string(path).map_err(|e|e.to_string())?;
    let root:Value=serde_json::from_str(&text).map_err(|e|format!("Invalid dotbim JSON: {e}"))?;
    let meshes=root.get("meshes").and_then(Value::as_array).ok_or("dotbim 'meshes' array is missing")?;
    let mut by_id=HashMap::<i64,BimMesh>::new();
    for mesh in meshes {
        let id=mesh.get("mesh_id").and_then(Value::as_i64).ok_or("dotbim mesh_id missing")?;
        let coords=mesh.get("coordinates").and_then(Value::as_array).ok_or("dotbim coordinates missing")?;
        let raw_indices=mesh.get("indices").and_then(Value::as_array).ok_or("dotbim indices missing")?;
        if coords.len()%3!=0||raw_indices.len()%3!=0{return Err("dotbim mesh arrays are malformed".into());}
        let mut positions=Vec::with_capacity(coords.len()/3);
        for xyz in coords.chunks_exact(3){positions.push([number_f32(&xyz[0])?,number_f32(&xyz[1])?,number_f32(&xyz[2])?]);}
        let mut indices=Vec::with_capacity(raw_indices.len());
        for value in raw_indices{indices.push(value.as_u64().ok_or("Invalid dotbim mesh index")? as u32);}
        by_id.insert(id,BimMesh{positions_m:positions,indices});
    }

    let mut objects=Vec::<SceneObject>::new();
    if let Some(elements)=root.get("elements").and_then(Value::as_array){
        for (element_index,element) in elements.iter().enumerate(){
            let mesh_id=element.get("mesh_id").and_then(Value::as_i64).ok_or("dotbim element mesh_id missing")?;
            let Some(source)=by_id.get(&mesh_id) else {continue;};
            let vector=element.get("vector");
            let tx=get_num(vector,"x").unwrap_or(0.0); let ty=get_num(vector,"y").unwrap_or(0.0); let tz=get_num(vector,"z").unwrap_or(0.0);
            let rotation=element.get("rotation");
            let raw_q=Quat::from_xyzw(get_num(rotation,"qx").unwrap_or(0.0),get_num(rotation,"qy").unwrap_or(0.0),get_num(rotation,"qz").unwrap_or(0.0),get_num(rotation,"qw").unwrap_or(1.0));
            let q=if raw_q.length_squared().is_finite()&&raw_q.length_squared()>1.0e-12{raw_q.normalize()}else{Quat::IDENTITY};
            let transform=Mat4::from_rotation_translation(q,Vec3::new(tx,ty,tz));
            let name=element.get("info").and_then(|v|v.get("Name")).and_then(Value::as_str)
                .or_else(||element.get("type").and_then(Value::as_str)).map(str::to_string)
                .unwrap_or_else(||format!("Element {}",element_index+1));
            let mut mesh=make_mesh(&name,source,transform)?;
            mesh.material_name=element.get("type").and_then(Value::as_str).unwrap_or("dotbim element").to_string();
            if let Some(color)=element.get("color"){
                let r=color.get("r").and_then(Value::as_u64).unwrap_or(97) as f32/255.0;
                let g=color.get("g").and_then(Value::as_u64).unwrap_or(163) as f32/255.0;
                let b=color.get("b").and_then(Value::as_u64).unwrap_or(250) as f32/255.0;
                let a=color.get("a").and_then(Value::as_u64).unwrap_or(255) as f32/255.0;
                mesh.base_color_factor=[r,g,b,a]; mesh.material_count=1;
            }
            objects.push(SceneObject::new(name,mesh));
        }
    }
    if objects.is_empty(){
        for (i,source) in by_id.values().enumerate(){
            let name=format!("Mesh {}",i+1); objects.push(SceneObject::new(name.clone(),make_mesh(&name,source,Mat4::IDENTITY)?));
        }
    }
    if objects.is_empty(){return Err("dotbim contains no renderable elements".into());}
    let name=path.file_name().and_then(|v|v.to_str()).unwrap_or("dotbim model");
    let mut scene=SceneDocument::new(name,objects)?; scene.source_path=Some(path.to_path_buf()); Ok(scene)
}

fn make_mesh(name:&str,source:&BimMesh,transform:Mat4)->Result<SceneMesh,String>{
    let mut vertices=Vec::<Vertex>::with_capacity(source.positions_m.len());
    for p in &source.positions_m{let world_mm=transform.transform_point3(Vec3::from_array(*p))*1000.0;vertices.push(Vertex{position:world_mm.to_array(),normal:[0.0;3],texcoord:[0.0;2]});}
    for &i in &source.indices{if i as usize>=source.positions_m.len(){return Err("dotbim index out of range".into());}}
    let mut mesh=SceneMesh::new(name,vertices,source.indices.clone())?; mesh.recalculate_normals(); Ok(mesh)
}
fn number_f32(value:&Value)->Result<f32,String>{value.as_f64().map(|v|v as f32).ok_or_else(||"Invalid dotbim number".into())}
fn get_num(parent:Option<&Value>,key:&str)->Option<f32>{parent?.get(key)?.as_f64().map(|v|v as f32)}
