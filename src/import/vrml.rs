use std::{fs, path::Path};

use glam::{Mat4, Quat, Vec3};

use crate::{
    assets::resolve_referenced_texture,
    mesh::{SceneDocument, SceneMesh, SceneObject, Vertex},
};

#[derive(Clone, Debug)]
enum Tok { Word(String), Number(f32), Str(String), LBrace, RBrace, LBracket, RBracket, Comma }

#[derive(Clone, Debug)]
struct Appearance {
    color: [f32;4],
    texture: Option<String>,
}
impl Default for Appearance { fn default()->Self{Self{color:[0.70,0.72,0.76,1.0],texture:None}} }

#[derive(Clone, Debug, Default)]
struct Ifs {
    points: Vec<[f32;3]>,
    texcoords: Vec<[f32;2]>,
    coord_index: Vec<i32>,
    texcoord_index: Vec<i32>,
    ccw: bool,
}

pub fn load_scene(path:&Path)->Result<SceneDocument,String>{
    let text=fs::read_to_string(path).map_err(|e|e.to_string())?;
    if !text.contains("#VRML") && !text.contains("IndexedFaceSet") {
        return Err("File does not look like VRML/WRL".into());
    }
    let tokens=tokenize(&text)?;
    let mut p=Parser{tokens,pos:0,objects:Vec::new(),model_path:path};
    p.walk_until(None,Mat4::IDENTITY)?;
    if p.objects.is_empty(){return Err("VRML contains no supported IndexedFaceSet geometry".into());}
    let name=path.file_name().and_then(|v|v.to_str()).unwrap_or("VRML model");
    let mut scene=SceneDocument::new(name,p.objects)?;
    scene.source_path=Some(path.to_path_buf());
    Ok(scene)
}

struct Parser<'a>{tokens:Vec<Tok>,pos:usize,objects:Vec<SceneObject>,model_path:&'a Path}
impl<'a> Parser<'a>{
    fn walk_until(&mut self, stop:Option<char>, world:Mat4)->Result<(),String>{
        while self.pos<self.tokens.len(){
            if matches!((stop,self.peek()),(Some('}'),Some(Tok::RBrace))|(Some(']'),Some(Tok::RBracket))){self.pos+=1;return Ok(());}
            match self.peek_word(){
                Some("DEF")=>{self.pos+=1;self.pos+=usize::from(self.pos<self.tokens.len());continue;}
                Some("USE")=>{self.pos+=1;self.pos+=usize::from(self.pos<self.tokens.len());continue;}
                Some("Transform")=>{self.pos+=1;self.parse_transform(world)?;}
                Some("Shape")=>{self.pos+=1;self.parse_shape(world)?;}
                Some(_) => {
                    self.pos+=1;
                    match self.peek(){
                        Some(Tok::LBrace)=>{self.pos+=1;self.walk_until(Some('}'),world)?;}
                        Some(Tok::LBracket)=>{self.pos+=1;self.walk_until(Some(']'),world)?;}
                        _=>{}
                    }
                }
                None=>{
                    match self.peek(){
                        Some(Tok::LBrace)=>{self.pos+=1;self.walk_until(Some('}'),world)?;}
                        Some(Tok::LBracket)=>{self.pos+=1;self.walk_until(Some(']'),world)?;}
                        Some(Tok::RBrace)|Some(Tok::RBracket)=>{self.pos+=1;if stop.is_some(){return Ok(());}}
                        _=>self.pos+=1,
                    }
                }
            }
        }
        Ok(())
    }

    fn parse_transform(&mut self,parent:Mat4)->Result<(),String>{
        self.expect_lbrace()?;
        let mut translation=Vec3::ZERO;let mut scale=Vec3::ONE;let mut rot_axis=Vec3::Y;let mut rot_angle=0.0f32;
        // VRML Transform order is T * C * R * SR * S * -SR * -C. We support T/R/S here.
        let mut child_ranges:Vec<(usize,usize)>=Vec::new();
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            match self.peek_word(){
                Some("translation")=>{self.pos+=1;translation=self.read_vec3()?;}
                Some("scale")=>{self.pos+=1;scale=self.read_vec3()?;}
                Some("rotation")=>{self.pos+=1;rot_axis=self.read_vec3()?;rot_angle=self.read_number()?;}
                Some("children")=>{
                    self.pos+=1;
                    if matches!(self.peek(),Some(Tok::LBracket)){
                        self.pos+=1;let start=self.pos;let end=find_matching(&self.tokens,start,'[')?;child_ranges.push((start,end));self.pos=end+1;
                    } else if matches!(self.peek(),Some(Tok::LBrace)){
                        self.pos+=1;let start=self.pos;let end=find_matching(&self.tokens,start,'{')?;child_ranges.push((start,end));self.pos=end+1;
                    } else {self.pos+=1;}
                }
                _=>self.skip_value(),
            }
        }
        let q=if rot_axis.length_squared()>1e-12{Quat::from_axis_angle(rot_axis.normalize(),rot_angle)}else{Quat::IDENTITY};
        let world=parent*Mat4::from_scale_rotation_translation(scale,q,translation*1000.0);
        for (start,end) in child_ranges{
            let saved=self.pos;self.pos=start;while self.pos<end{self.walk_one(world)?;}self.pos=saved;
        }
        Ok(())
    }

    fn walk_one(&mut self,world:Mat4)->Result<(),String>{
        match self.peek_word(){
            Some("Transform")=>{self.pos+=1;self.parse_transform(world)}
            Some("Shape")=>{self.pos+=1;self.parse_shape(world)}
            Some("DEF")=>{self.pos+=1;if self.pos<self.tokens.len(){self.pos+=1;}self.walk_one(world)}
            Some(_) => {self.pos+=1;if matches!(self.peek(),Some(Tok::LBrace)){self.pos+=1;self.walk_until(Some('}'),world)?;}else if matches!(self.peek(),Some(Tok::LBracket)){self.pos+=1;self.walk_until(Some(']'),world)?;}Ok(())}
            None=>{self.pos+=1;Ok(())}
        }
    }

    fn parse_shape(&mut self,world:Mat4)->Result<(),String>{
        self.expect_lbrace()?;
        let mut appearance=Appearance::default();let mut geom:Option<Ifs>=None;
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            match self.peek_word(){
                Some("appearance")=>{self.pos+=1;appearance=self.parse_appearance()?;}
                Some("geometry")=>{
                    self.pos+=1;
                    if self.peek_word()==Some("DEF") { self.pos+=1; if self.pos<self.tokens.len(){ self.pos+=1; } }
                    if self.peek_word()==Some("IndexedFaceSet") {
                        self.pos+=1; geom=Some(self.parse_ifs()?);
                    } else if self.peek_word()==Some("NULL") {
                        self.pos+=1;
                    } else {
                        self.skip_current_value();
                    }
                }
                _=>self.skip_value(),
            }
        }
        if let Some(ifs)=geom{self.append_ifs(world,appearance,ifs)?;}
        Ok(())
    }

    fn parse_appearance(&mut self)->Result<Appearance,String>{
        if self.peek_word()==Some("NULL") { self.pos+=1; return Ok(Appearance::default()); }
        if self.peek_word()==Some("DEF") { self.pos+=1; if self.pos<self.tokens.len(){ self.pos+=1; } }
        if self.peek_word()==Some("Appearance"){self.pos+=1;}
        self.expect_lbrace()?;let mut app=Appearance::default();
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            match self.peek_word(){
                Some("material")=>{self.pos+=1;if self.peek_word()==Some("Material"){self.pos+=1;}self.expect_lbrace()?;self.parse_material_body(&mut app)?;}
                Some("texture")=>{self.pos+=1;if self.peek_word()==Some("ImageTexture"){self.pos+=1;}self.expect_lbrace()?;self.parse_texture_body(&mut app)?;}
                _=>self.skip_value(),
            }
        }
        Ok(app)
    }
    fn parse_material_body(&mut self,app:&mut Appearance)->Result<(),String>{
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            match self.peek_word(){
                Some("diffuseColor")=>{self.pos+=1;let v=self.read_vec3()?;app.color[0]=v.x.clamp(0.0,1.0);app.color[1]=v.y.clamp(0.0,1.0);app.color[2]=v.z.clamp(0.0,1.0);}
                Some("transparency")=>{self.pos+=1;app.color[3]=(1.0-self.read_number()?).clamp(0.0,1.0);}
                _=>self.skip_value(),
            }
        }Ok(())
    }
    fn parse_texture_body(&mut self,app:&mut Appearance)->Result<(),String>{
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            if self.peek_word()==Some("url"){
                self.pos+=1;
                if matches!(self.peek(),Some(Tok::LBracket)){self.pos+=1;while self.pos<self.tokens.len()&&!matches!(self.peek(),Some(Tok::RBracket)){if let Some(Tok::Str(s))=self.peek().cloned(){if app.texture.is_none(){app.texture=Some(s);}self.pos+=1;}else{self.pos+=1;}}if matches!(self.peek(),Some(Tok::RBracket)){self.pos+=1;}}
                else if let Some(Tok::Str(s))=self.peek().cloned(){app.texture=Some(s);self.pos+=1;}
            }else{self.skip_value();}
        }Ok(())
    }

    fn parse_ifs(&mut self)->Result<Ifs,String>{
        self.expect_lbrace()?;let mut out=Ifs{ccw:true,..Default::default()};
        while self.pos<self.tokens.len(){
            if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}
            match self.peek_word(){
                Some("coord")=>{self.pos+=1;if self.peek_word()==Some("Coordinate"){self.pos+=1;}self.expect_lbrace()?;self.parse_coordinate_body(&mut out.points)?;}
                Some("texCoord")=>{self.pos+=1;if self.peek_word()==Some("TextureCoordinate"){self.pos+=1;}self.expect_lbrace()?;self.parse_texcoord_body(&mut out.texcoords)?;}
                Some("coordIndex")=>{self.pos+=1;out.coord_index=self.read_i32_array()?;}
                Some("texCoordIndex")=>{self.pos+=1;out.texcoord_index=self.read_i32_array()?;}
                Some("ccw")=>{self.pos+=1;out.ccw=!matches!(self.peek_word(),Some("FALSE")|Some("false"));self.pos+=usize::from(self.pos<self.tokens.len());}
                _=>self.skip_value(),
            }
        }
        Ok(out)
    }
    fn parse_coordinate_body(&mut self,out:&mut Vec<[f32;3]>)->Result<(),String>{
        while self.pos<self.tokens.len(){if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}if self.peek_word()==Some("point"){self.pos+=1;let vals=self.read_f32_array()?;for c in vals.chunks_exact(3){out.push([c[0],c[1],c[2]]);}}else{self.skip_value();}}Ok(())
    }
    fn parse_texcoord_body(&mut self,out:&mut Vec<[f32;2]>)->Result<(),String>{
        while self.pos<self.tokens.len(){if matches!(self.peek(),Some(Tok::RBrace)){self.pos+=1;break;}if self.peek_word()==Some("point"){self.pos+=1;let vals=self.read_f32_array()?;for c in vals.chunks_exact(2){out.push([c[0],1.0-c[1]]);}}else{self.skip_value();}}Ok(())
    }

    fn append_ifs(&mut self,world:Mat4,app:Appearance,ifs:Ifs)->Result<(),String>{
        let polys=split_polygons(&ifs.coord_index);if polys.is_empty(){return Ok(());}
        let uvpolys=split_polygons(&ifs.texcoord_index);
        let normal_mat=world.inverse().transpose();
        let mut vertices=Vec::<Vertex>::new();let mut indices=Vec::<u32>::new();
        for (pi,poly) in polys.iter().enumerate(){
            if poly.len()<3{continue;}
            let uvpoly=uvpolys.get(pi);
            for k in 1..poly.len()-1{
                let corners=if ifs.ccw {[0,k,k+1]}else{[0,k+1,k]};
                for &slot in &corners{
                    let ci=poly[slot];if ci<0||ci as usize>=ifs.points.len(){continue;}
                    let p=world.transform_point3(Vec3::from_array(ifs.points[ci as usize])*1000.0);
                    let uv=uvpoly.and_then(|v|v.get(slot)).and_then(|&ti|if ti>=0{ifs.texcoords.get(ti as usize)}else{None}).copied().or_else(||ifs.texcoords.get(ci as usize).copied()).unwrap_or([0.0;2]);
                    // normals are recomputed after import; the matrix is kept here to document transform semantics.
                    let _=normal_mat;
                    vertices.push(Vertex{position:p.to_array(),normal:[0.0;3],texcoord:uv});indices.push(vertices.len() as u32-1);
                }
            }
        }
        if indices.len()<3{return Ok(());}
        let name=format!("Shape {}",self.objects.len()+1);let mut mesh=SceneMesh::new(name.clone(),vertices,indices)?;mesh.base_color_factor=app.color;mesh.material_name="VRML Material".into();mesh.material_count=1;
        if let Some(tex)=app.texture{if let Some(p)=resolve_referenced_texture(self.model_path,&tex){mesh.base_color_texture=Some(p.clone());mesh.discovered_textures.push(p);}}
        self.objects.push(SceneObject::new(name,mesh));Ok(())
    }

    fn read_vec3(&mut self)->Result<Vec3,String>{Ok(Vec3::new(self.read_number()?,self.read_number()?,self.read_number()?))}
    fn read_number(&mut self)->Result<f32,String>{match self.next(){Some(Tok::Number(v))=>Ok(v),Some(Tok::Word(s))=>s.parse::<f32>().map_err(|_|format!("Expected VRML number, got {s}")),_=>Err("Expected VRML number".into())}}
    fn read_f32_array(&mut self)->Result<Vec<f32>,String>{self.expect_lbracket()?;let mut out=Vec::new();while self.pos<self.tokens.len(){match self.peek(){Some(Tok::RBracket)=>{self.pos+=1;break;}Some(Tok::Number(v))=>{out.push(*v);self.pos+=1;}Some(Tok::Word(s))=>{if let Ok(v)=s.parse::<f32>(){out.push(v);}self.pos+=1;}Some(Tok::Comma)=>self.pos+=1,_=>self.pos+=1}}Ok(out)}
    fn read_i32_array(&mut self)->Result<Vec<i32>,String>{self.expect_lbracket()?;let mut out=Vec::new();while self.pos<self.tokens.len(){match self.peek(){Some(Tok::RBracket)=>{self.pos+=1;break;}Some(Tok::Number(v))=>{out.push(*v as i32);self.pos+=1;}Some(Tok::Word(s))=>{if let Ok(v)=s.parse::<i32>(){out.push(v);}self.pos+=1;}Some(Tok::Comma)=>self.pos+=1,_=>self.pos+=1}}Ok(out)}
    fn skip_current_value(&mut self) {
        if self.pos >= self.tokens.len() { return; }
        if matches!(self.peek(), Some(Tok::Word(_)))
            && matches!(self.tokens.get(self.pos + 1), Some(Tok::LBrace))
        {
            self.pos += 1;
        }
        match self.peek() {
            Some(Tok::LBrace) => self.skip_balanced('{'),
            Some(Tok::LBracket) => self.skip_balanced('['),
            Some(_) => self.pos += 1,
            None => {}
        }
    }

    fn skip_value(&mut self) {
        // Most callers are positioned on a field name. Skip the name, then the
        // scalar, bracketed array, or nested node that constitutes its value.
        if self.pos < self.tokens.len() { self.pos += 1; }
        if self.pos >= self.tokens.len() { return; }

        // Unknown node type: `SomeNode { ... }`.
        if matches!(self.peek(), Some(Tok::Word(_)))
            && matches!(self.tokens.get(self.pos + 1), Some(Tok::LBrace))
        {
            self.pos += 1;
        }

        match self.peek() {
            Some(Tok::LBrace) => self.skip_balanced('{'),
            Some(Tok::LBracket) => self.skip_balanced('['),
            Some(_) => self.pos += 1,
            None => {}
        }
    }

    fn skip_balanced(&mut self, kind: char) {
        let (open_is, close_is): (fn(&Tok) -> bool, fn(&Tok) -> bool) = match kind {
            '[' => (|t| matches!(t, Tok::LBracket), |t| matches!(t, Tok::RBracket)),
            _ => (|t| matches!(t, Tok::LBrace), |t| matches!(t, Tok::RBrace)),
        };
        if self.pos >= self.tokens.len() || !open_is(&self.tokens[self.pos]) { return; }
        let mut depth = 0usize;
        while self.pos < self.tokens.len() {
            let t = &self.tokens[self.pos];
            if open_is(t) { depth += 1; }
            if close_is(t) {
                depth = depth.saturating_sub(1);
                self.pos += 1;
                if depth == 0 { break; }
                continue;
            }
            self.pos += 1;
        }
    }
    fn expect_lbrace(&mut self)->Result<(),String>{if matches!(self.next(),Some(Tok::LBrace)){Ok(())}else{Err("Expected '{' in VRML".into())}}
    fn expect_lbracket(&mut self)->Result<(),String>{if matches!(self.next(),Some(Tok::LBracket)){Ok(())}else{Err("Expected '[' in VRML".into())}}
    fn peek(&self)->Option<&Tok>{self.tokens.get(self.pos)}
    fn next(&mut self)->Option<Tok>{let v=self.tokens.get(self.pos).cloned();if v.is_some(){self.pos+=1;}v}
    fn peek_word(&self)->Option<&str>{match self.peek(){Some(Tok::Word(s))=>Some(s.as_str()),_=>None}}
}

fn split_polygons(raw:&[i32])->Vec<Vec<i32>>{let mut out=Vec::new();let mut cur=Vec::new();for &v in raw{if v==-1{if !cur.is_empty(){out.push(std::mem::take(&mut cur));}}else{cur.push(v);}}if !cur.is_empty(){out.push(cur);}out}

fn find_matching(tokens: &[Tok], start: usize, kind: char) -> Result<usize, String> {
    let is_open = |t: &Tok| match kind {
        '[' => matches!(t, Tok::LBracket),
        _ => matches!(t, Tok::LBrace),
    };
    let is_close = |t: &Tok| match kind {
        '[' => matches!(t, Tok::RBracket),
        _ => matches!(t, Tok::RBrace),
    };
    let mut depth = 1usize;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        if is_open(token) {
            depth += 1;
        } else if is_close(token) {
            depth -= 1;
            if depth == 0 { return Ok(i); }
        }
    }
    Err("Unterminated VRML block".into())
}

fn tokenize(src:&str)->Result<Vec<Tok>,String>{
    let b=src.as_bytes();let mut i=0usize;let mut out=Vec::new();
    while i<b.len(){
        match b[i] as char{
            c if c.is_whitespace()=>i+=1,
            '#'=>{while i<b.len()&&b[i] as char!='\n'{i+=1;}},
            '{'=>{out.push(Tok::LBrace);i+=1;},'}'=>{out.push(Tok::RBrace);i+=1;},'['=>{out.push(Tok::LBracket);i+=1;},']'=>{out.push(Tok::RBracket);i+=1;},','=>{out.push(Tok::Comma);i+=1;},
            '"'=>{i+=1;let start=i;while i<b.len()&&b[i] as char!='"'{if b[i] as char=='\\'&&i+1<b.len(){i+=2;}else{i+=1;}}if i>=b.len(){return Err("Unterminated VRML string".into());}out.push(Tok::Str(src[start..i].replace("\\\"","\"")));i+=1;},
            _=>{let start=i;while i<b.len(){let c=b[i] as char;if c.is_whitespace()||matches!(c,'{'|'}'|'['|']'|',')||c=='#'{break;}i+=1;}let s=&src[start..i];if let Ok(v)=s.parse::<f32>(){out.push(Tok::Number(v));}else{out.push(Tok::Word(s.to_string()));}}
        }
    }
    Ok(out)
}
