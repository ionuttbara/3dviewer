use std::{collections::HashMap, fs, path::{Path,PathBuf}};

use crate::{
    assets::{choose_fallback_texture, discover_textures, is_texture_file, resolve_referenced_texture},
    mesh::{SceneDocument,SceneMesh,SceneObject,Vertex},
};

#[derive(Default)]
struct Bucket {
    name:String,
    material:String,
    vertices:Vec<Vertex>,
    indices:Vec<u32>,
    map:HashMap<(i32,i32,i32),u32>,
    missing_normal:bool,
}

#[derive(Clone)]
struct MaterialDef {
    color:[f32;4],
    map_kd:Option<(PathBuf,String)>,
}
impl Default for MaterialDef { fn default()->Self{Self{color:[0.38,0.64,0.98,1.0],map_kd:None}} }

pub fn load_scene(path:&Path)->Result<SceneDocument,String>{
    let text=fs::read_to_string(path).map_err(|e|e.to_string())?;
    let mut positions=Vec::<[f32;3]>::new();let mut normals=Vec::<[f32;3]>::new();let mut texcoords=Vec::<[f32;2]>::new();
    let mut material_libraries=Vec::<String>::new();let mut buckets=Vec::<Bucket>::new();let mut bucket_lookup=HashMap::<(String,String),usize>::new();
    let mut current_object="Mesh".to_string();let mut current_material="Default".to_string();

    for raw_line in text.lines(){
        let line=raw_line.trim();if line.is_empty()||line.starts_with('#'){continue;}let mut it=line.split_whitespace();
        match it.next().unwrap_or(""){
            "v"=>{let x: f32=it.next().ok_or("OBJ vertex missing X")?.parse().map_err(|_|"Invalid OBJ vertex")?;let y=it.next().ok_or("OBJ vertex missing Y")?.parse().map_err(|_|"Invalid OBJ vertex")?;let z=it.next().ok_or("OBJ vertex missing Z")?.parse().map_err(|_|"Invalid OBJ vertex")?;positions.push([x,y,z]);}
            "vn"=>{let x: f32=it.next().ok_or("OBJ normal missing X")?.parse().map_err(|_|"Invalid OBJ normal")?;let y: f32=it.next().ok_or("OBJ normal missing Y")?.parse().map_err(|_|"Invalid OBJ normal")?;let z: f32=it.next().ok_or("OBJ normal missing Z")?.parse().map_err(|_|"Invalid OBJ normal")?;normals.push([x,y,z]);}
            "vt"=>{let u: f32=it.next().ok_or("OBJ texture coordinate missing U")?.parse().map_err(|_|"Invalid OBJ texture coordinate")?;let v: f32=it.next().unwrap_or("0").parse().map_err(|_|"Invalid OBJ texture coordinate")?;texcoords.push([u,1.0_f32-v]);}
            "mtllib"=>{let rest=line["mtllib".len()..].trim();if !rest.is_empty(){if rest.starts_with('"')&&rest.ends_with('"'){material_libraries.push(rest.trim_matches('"').to_string());}else{material_libraries.extend(rest.split_whitespace().map(str::to_string));}}}
            "o"|"g"=>{let rest=line[1..].trim();if !rest.is_empty(){current_object=rest.to_string();}}
            "usemtl"=>{let rest=line["usemtl".len()..].trim();current_material=if rest.is_empty(){"Default".into()}else{rest.to_string()};}
            "f"=>{
                let parts:Vec<_>=it.collect();if parts.len()<3{continue;}
                let key=(current_object.clone(),current_material.clone());
                let bucket_index=*bucket_lookup.entry(key.clone()).or_insert_with(||{let idx=buckets.len();buckets.push(Bucket{name:key.0.clone(),material:key.1.clone(),..Default::default()});idx});
                let bucket=&mut buckets[bucket_index];let mut face=Vec::<u32>::with_capacity(parts.len());
                for token in parts{
                    let seg:Vec<_>=token.split('/').collect();let vi_raw:i32=seg.first().and_then(|s|s.parse().ok()).ok_or("Invalid OBJ face index")?;let ti_raw:i32=if seg.len()>=2&&!seg[1].is_empty(){seg[1].parse().unwrap_or(0)}else{0};let ni_raw:i32=if seg.len()>=3&&!seg[2].is_empty(){seg[2].parse().unwrap_or(0)}else{0};
                    let vi=resolve_index(vi_raw,positions.len()).ok_or("OBJ vertex index out of range")?;let ti=if ti_raw==0{None}else{resolve_index(ti_raw,texcoords.len())};let ni=if ni_raw==0{None}else{resolve_index(ni_raw,normals.len())};
                    let vertex_key=(vi as i32,ti.map(|x|x as i32).unwrap_or(-1),ni.map(|x|x as i32).unwrap_or(-1));
                    let idx=if let Some(&existing)=bucket.map.get(&vertex_key){existing}else{let normal=ni.map(|n|normals[n]).unwrap_or([0.0;3]);let texcoord=ti.map(|t|texcoords[t]).unwrap_or([0.0;2]);if ni.is_none(){bucket.missing_normal=true;}let id=bucket.vertices.len() as u32;bucket.vertices.push(Vertex{position:positions[vi],normal,texcoord});bucket.map.insert(vertex_key,id);id};
                    face.push(idx);
                }
                for i in 1..face.len()-1{bucket.indices.extend_from_slice(&[face[0],face[i],face[i+1]]);}
            }
            _=>{}
        }
    }

    let materials=parse_materials(path,&material_libraries);
    let discovery=discover_textures(path);let mut objects=Vec::<SceneObject>::new();
    let mut name_counts=HashMap::<String,usize>::new();
    for mut bucket in buckets{
        if bucket.vertices.is_empty()||bucket.indices.is_empty(){continue;}
        let count=name_counts.entry(bucket.name.clone()).or_insert(0);*count+=1;
        let display_name=if *count>1||bucket.material!="Default"{format!("{} • {}",bucket.name,bucket.material)}else{bucket.name.clone()};
        let mut mesh=SceneMesh::new(display_name.clone(),std::mem::take(&mut bucket.vertices),std::mem::take(&mut bucket.indices))?;
        if bucket.missing_normal{mesh.recalculate_normals();}
        mesh.material_name=bucket.material.clone();mesh.material_count=usize::from(bucket.material!="Default");
        mesh.discovered_textures=discovery.files.clone();mesh.searched_texture_directories=discovery.searched_directories.clone();
        if let Some(def)=materials.get(&bucket.material){mesh.base_color_factor=def.color;if let Some((mtl_path,reference))=&def.map_kd{mesh.base_color_texture=resolve_texture_from_mtl(path,mtl_path,reference);}}
        if mesh.base_color_texture.is_none()&&mesh.has_texture_coordinates(){mesh.base_color_texture=choose_fallback_texture(path,&discovery);}
        if let Some(texture)=mesh.base_color_texture.clone(){if !mesh.discovered_textures.contains(&texture){mesh.discovered_textures.push(texture);}}
        objects.push(SceneObject::new(display_name,mesh));
    }
    if objects.is_empty(){return Err("OBJ contains no renderable faces".into());}
    let name=path.file_name().and_then(|v|v.to_str()).unwrap_or("OBJ model");let mut scene=SceneDocument::new(name,objects)?;scene.source_path=Some(path.to_path_buf());Ok(scene)
}

fn parse_materials(model_path:&Path,libraries:&[String])->HashMap<String,MaterialDef>{
    let Some(parent)=model_path.parent()else{return HashMap::new();};let mut defs=HashMap::<String,MaterialDef>::new();
    for library in libraries{
        let Some(library_path)=resolve_sidecar(parent,library)else{continue;};let Ok(text)=fs::read_to_string(&library_path)else{continue;};let mut current=String::new();
        for raw in text.lines(){let line=raw.trim();if line.is_empty()||line.starts_with('#'){continue;}
            if let Some(rest)=line.strip_prefix("newmtl "){current=rest.trim().to_string();defs.entry(current.clone()).or_default();}
            else if let Some(rest)=line.strip_prefix("Kd "){if current.is_empty(){continue;}let mut p=rest.split_whitespace().filter_map(|v|v.parse::<f32>().ok());if let(Some(r),Some(g),Some(b))=(p.next(),p.next(),p.next()){defs.entry(current.clone()).or_default().color[0..3].copy_from_slice(&[r.clamp(0.0,1.0),g.clamp(0.0,1.0),b.clamp(0.0,1.0)]);}}
            else if let Some(rest)=line.strip_prefix("d "){if let Ok(a)=rest.trim().parse::<f32>(){defs.entry(current.clone()).or_default().color[3]=a.clamp(0.0,1.0);}}
            else if let Some(rest)=line.strip_prefix("Tr "){if let Ok(t)=rest.trim().parse::<f32>(){defs.entry(current.clone()).or_default().color[3]=(1.0-t).clamp(0.0,1.0);}}
            else if let Some(rest)=line.strip_prefix("map_Kd "){if !current.is_empty(){if let Some(reference)=parse_map_reference(rest){defs.entry(current.clone()).or_default().map_kd=Some((library_path.clone(),reference));}}}
        }
    }
    defs
}
fn resolve_texture_from_mtl(model_path:&Path,mtl_path:&Path,reference:&str)->Option<PathBuf>{let reference_path=Path::new(reference.trim().trim_matches('"').trim_matches('\''));if reference_path.is_absolute()&&reference_path.is_file()&&is_texture_file(reference_path){return Some(reference_path.to_path_buf());}if let Some(mtl_parent)=mtl_path.parent(){let candidate=mtl_parent.join(reference_path);if candidate.is_file()&&is_texture_file(&candidate){return Some(candidate);}}resolve_referenced_texture(model_path,reference)}
fn resolve_sidecar(parent:&Path,reference:&str)->Option<PathBuf>{let direct=parent.join(reference.trim_matches('"'));if direct.is_file(){return Some(direct);}let filename=Path::new(reference).file_name()?.to_string_lossy().to_ascii_lowercase();fs::read_dir(parent).ok()?.flatten().map(|e|e.path()).find(|p|p.is_file()&&p.file_name().map(|v|v.to_string_lossy().to_ascii_lowercase()==filename).unwrap_or(false))}
fn parse_map_reference(rest:&str)->Option<String>{let rest=rest.trim();if rest.is_empty(){return None;}if let Some(stripped)=rest.strip_prefix('"'){if let Some(end)=stripped.rfind('"'){return Some(stripped[..end].to_string());}}rest.split_whitespace().last().map(str::to_string)}
fn resolve_index(index:i32,len:usize)->Option<usize>{if index>0{usize::try_from(index-1).ok().filter(|&i|i<len)}else if index<0{let i=len as i32+index;usize::try_from(i).ok().filter(|&i|i<len)}else{None}}
