use std::{fs, path::Path};
use quick_xml::{events::{BytesStart, Event}, Reader};
use crate::mesh::{SceneDocument, SceneMesh, SceneObject, Vertex};

pub fn load_scene(path: &Path) -> Result<SceneDocument, String> {
    let xml = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_str(&xml); reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut objects = Vec::<SceneObject>::new();
    let mut vertices = Vec::<Vertex>::new();
    let mut indices = Vec::<u32>::new();
    let mut coord = [0.0f32;3]; let mut tri = [0u32;3]; let mut field = Field::None;
    let mut unit_scale = 1.0f32; let mut mesh_index = 0usize; let mut in_mesh = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"amf" => unit_scale = unit_scale_mm(attr(&e,b"unit").as_deref()),
                b"mesh" => { vertices.clear(); indices.clear(); mesh_index += 1; in_mesh = true; }
                b"vertex" if in_mesh => coord=[0.0;3], b"triangle" if in_mesh => tri=[0;3],
                b"x" => field=Field::X, b"y" => field=Field::Y, b"z" => field=Field::Z,
                b"v1" => field=Field::V1, b"v2" => field=Field::V2, b"v3" => field=Field::V3,
                _ => {}
            },
            Ok(Event::Text(e)) => {
                let text=String::from_utf8_lossy(e.as_ref());
                match field {
                    Field::X => coord[0]=text.parse::<f32>().map_err(|_|"Invalid AMF X")?*unit_scale,
                    Field::Y => coord[1]=text.parse::<f32>().map_err(|_|"Invalid AMF Y")?*unit_scale,
                    Field::Z => coord[2]=text.parse::<f32>().map_err(|_|"Invalid AMF Z")?*unit_scale,
                    Field::V1 => tri[0]=text.parse().map_err(|_|"Invalid AMF v1")?,
                    Field::V2 => tri[1]=text.parse().map_err(|_|"Invalid AMF v2")?,
                    Field::V3 => tri[2]=text.parse().map_err(|_|"Invalid AMF v3")?, Field::None=>{}
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"vertex" if in_mesh => vertices.push(Vertex{position:coord,normal:[0.0;3],texcoord:[0.0;2]}),
                b"triangle" if in_mesh => indices.extend_from_slice(&tri),
                b"mesh" => {
                    if !vertices.is_empty() && !indices.is_empty() {
                        let name=format!("Mesh {mesh_index}");
                        let mut mesh=SceneMesh::new(name.clone(),vertices.clone(),indices.clone())?; mesh.recalculate_normals();
                        mesh.material_name="AMF material".into(); objects.push(SceneObject::new(name,mesh));
                    }
                    in_mesh=false;
                }
                b"x"|b"y"|b"z"|b"v1"|b"v2"|b"v3" => field=Field::None, _=>{}
            },
            Ok(Event::Eof)=>break, Err(e)=>return Err(format!("AMF XML parse error: {e}")), _=>{}
        }
        buf.clear();
    }
    if objects.is_empty(){return Err("AMF contains no renderable meshes".into());}
    let name=path.file_name().and_then(|v|v.to_str()).unwrap_or("AMF model");
    let mut scene=SceneDocument::new(name,objects)?; scene.source_path=Some(path.to_path_buf()); Ok(scene)
}
#[derive(Clone,Copy)] enum Field{None,X,Y,Z,V1,V2,V3}
fn local_name(name:&[u8])->&[u8]{name.rsplit(|b|*b==b':').next().unwrap_or(name)}
fn attr(e:&BytesStart<'_>,key:&[u8])->Option<String>{for a in e.attributes().flatten(){if local_name(a.key.as_ref())==key{return Some(String::from_utf8_lossy(a.value.as_ref()).into_owned());}}None}
fn unit_scale_mm(unit:Option<&str>)->f32{match unit.unwrap_or("millimeter").to_ascii_lowercase().as_str(){"micron"=>0.001,"centimeter"=>10.0,"meter"=>1000.0,"inch"=>25.4,"foot"|"feet"=>304.8,_=>1.0}}
