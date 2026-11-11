mod amf;
mod bim;
mod cad;
mod dae;
mod fbx;
mod gltf;
mod ifc;
mod obj;
mod off;
mod ply;
mod stl;
mod three_ds;
mod three_mf;
mod vrml;

use std::path::Path;

use crate::{
    assets::{choose_fallback_texture, discover_textures},
    mesh::SceneDocument,
};

pub const ALL_EXTENSIONS: &[&str] = &[
    "3dm", "3ds", "3mf", "amf", "bim", "brep", "brp", "dae", "fbx", "fcstd", "gltf", "glb", "ifc", "iges", "igs", "step", "stp", "stl", "obj", "off", "ply", "wrl", "vrml",
];

#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    pub cad_linear_deflection_mm: f32,
    pub cad_angular_deflection_deg: f32,
    pub cad_relative_deflection: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        let cad = cad::CadOptions::default();
        Self {
            cad_linear_deflection_mm: cad.linear_deflection_mm,
            cad_angular_deflection_deg: cad.angular_deflection_deg,
            cad_relative_deflection: cad.relative_linear,
        }
    }
}

impl ImportOptions {
    fn cad(self) -> cad::CadOptions {
        cad::CadOptions {
            linear_deflection_mm: self.cad_linear_deflection_mm,
            angular_deflection_deg: self.cad_angular_deflection_deg,
            relative_linear: self.cad_relative_deflection,
        }
    }
}

pub struct ImportedScene {
    pub scene: SceneDocument,
    pub backend: String,
}

pub fn backend_status_for_path(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "step" | "stp" => "OpenCASCADE (embedded) • STEP exact CAD → tessellated viewport",
        "brep" | "brp" => "OpenCASCADE (embedded) • binary BRep → tessellated viewport",
        "fcstd" => "FCStd ZIP + embedded OpenCASCADE binary BRep payload reader",
        "ifc" => "Native IFC4 tessellated-geometry backend",
        "iges" | "igs" => "IGES recognized • full NURBS backend not bundled in v0.2.2",
        "3dm" => "3DM recognized • openNURBS backend not bundled in v0.2.2",
        "fbx" => "OxideAV FBX • static mesh scene",
        "3ds" => "Native 3DS chunk parser",
        "wrl" | "vrml" => "Native VRML97 IndexedFaceSet parser",
        "gltf" | "glb" => "glTF 1.4 scene importer",
        "3mf" => "Native 3MF ZIP/XML importer",
        "amf" => "Native AMF XML importer",
        "bim" => "Native dotbim scene importer",
        "dae" => "Native COLLADA importer",
        "obj" => "Native OBJ/MTL importer",
        "stl" => "Native ASCII/Binary STL importer",
        "off" => "Native OFF importer",
        "ply" => "Native ASCII/Binary PLY importer",
        _ => "Importer not selected",
    }
}

pub fn load(path: &Path, options: &ImportOptions) -> Result<ImportedScene, String> {
    let ext = extension(path);
    let (mut scene, backend): (SceneDocument, &str) = match ext.as_str() {
        "gltf" | "glb" => (gltf::load_scene(path)?, "glTF 1.4"),
        "3mf" => (three_mf::load_scene(path)?, "Native 3MF"),
        "amf" => (amf::load_scene(path)?, "Native AMF"),
        "bim" => (bim::load_scene(path)?, "Native dotbim"),
        "dae" => (dae::load_scene(path)?, "Native COLLADA"),
        "3ds" => (three_ds::load_scene(path)?, "Native 3DS"),
        "fbx" => (fbx::load_scene(path)?, "OxideAV FBX"),
        "wrl" | "vrml" => (vrml::load_scene(path)?, "Native VRML97"),
        "obj" => (obj::load_scene(path)?, "Native OBJ/MTL"),
        "stl" => (SceneDocument::from_mesh(stl::load(path)?), "Native STL"),
        "off" => (SceneDocument::from_mesh(off::load(path)?), "Native OFF"),
        "ply" => (SceneDocument::from_mesh(ply::load(path)?), "Native PLY"),
        "step" | "stp" => (cad::load_step(path, options.cad())?, "Embedded OpenCASCADE STEP"),
        "brep" | "brp" => (cad::load_brep(path, options.cad())?, "Embedded OpenCASCADE binary BRep"),
        "fcstd" => (cad::load_fcstd(path, options.cad())?, "FCStd + embedded OpenCASCADE binary BRep"),
        "ifc" => (ifc::load_scene(path)?, "Native IFC4 tessellated geometry"),
        "iges" | "igs" => {
            return Err("IGES/IGS is recognized and registered, but v0.2.2 does not silently approximate NURBS/analytic IGES with a partial parser. Use STEP/STP for the embedded OpenCASCADE CAD backend.".into());
        }
        "3dm" => {
            return Err("Rhino 3DM is recognized and registered, but a full openNURBS geometry backend is not bundled in v0.2.2. The viewer keeps the association staged instead of returning incomplete geometry.".into());
        }
        _ => return Err(format!("Unsupported 3D file extension: {}", if ext.is_empty() { "(none)" } else { &ext })),
    };

    scene.source_path = Some(path.to_path_buf());
    attach_local_texture_candidates(path, &mut scene);
    Ok(ImportedScene { scene, backend: backend.to_string() })
}

fn attach_local_texture_candidates(path: &Path, scene: &mut SceneDocument) {
    let discovery = discover_textures(path);
    let ext = extension(path);
    for object in &mut scene.objects {
        let mesh = &mut object.mesh;
        for item in discovery.files.iter().cloned() {
            if !mesh.discovered_textures.contains(&item) { mesh.discovered_textures.push(item); }
        }
        if mesh.searched_texture_directories.is_empty() {
            mesh.searched_texture_directories = discovery.searched_directories.clone();
        }

        // Pick the most likely base-colour map even for formats such as STL that do
        // not carry UV coordinates themselves. The UV fallback is applied below.
        if mesh.base_color_texture.is_none() && mesh.embedded_base_color.is_none() {
            mesh.base_color_texture = choose_fallback_texture(path, &discovery);
        }

        // STL has no standard UV channel. If a local texture package exists, create
        // box-projection UVs so the Use textures toggle actually renders the image.
        if ext == "stl"
            && (mesh.base_color_texture.is_some() || mesh.embedded_base_color.is_some())
            && !mesh.has_texture_coordinates()
        {
            mesh.generate_box_uvs();
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase()
}
