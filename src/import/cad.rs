use std::path::Path;

#[cfg(feature = "cad-occt")]
use std::{fs::File, io::{Cursor, Read}};
#[cfg(feature = "cad-occt")]
use cadrum::{Solid, Tessellation};
#[cfg(feature = "cad-occt")]
use zip::ZipArchive;

use crate::mesh::SceneDocument;
#[cfg(feature = "cad-occt")]
use crate::mesh::SceneObject;
#[cfg(feature = "cad-occt")]
use super::stl;

#[derive(Clone, Copy, Debug)]
pub struct CadOptions {
    pub linear_deflection_mm: f32,
    pub angular_deflection_deg: f32,
    pub relative_linear: bool,
}

impl Default for CadOptions {
    fn default() -> Self {
        Self {
            linear_deflection_mm: 0.10,
            angular_deflection_deg: 20.0,
            relative_linear: false,
        }
    }
}

impl CadOptions {
    #[cfg(feature = "cad-occt")]
    fn tessellation(self) -> Tessellation {
        Tessellation {
            deflection_linear: self.linear_deflection_mm.clamp(0.001, 10.0) as f64,
            deflection_angular: (self.angular_deflection_deg.clamp(0.5, 90.0) as f64).to_radians(),
            relative_linear: self.relative_linear,
        }
    }
}

#[cfg(feature = "cad-occt")]
pub fn load_step(path: &Path, options: CadOptions) -> Result<SceneDocument, String> {
    let mut file = File::open(path).map_err(|e| format!("Could not open STEP: {e}"))?;
    let solids = Solid::read_step(&mut file).map_err(|e| format!("OpenCASCADE STEP import failed: {e}"))?;
    solids_to_scene(path, solids, options, "OpenCASCADE STEP")
}

#[cfg(feature = "cad-occt")]
pub fn load_brep(path: &Path, options: CadOptions) -> Result<SceneDocument, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not open BRep: {e}"))?;
    let solids = read_brep_payload(&bytes).map_err(|e| format!("OpenCASCADE BRep import failed: {e}"))?;
    solids_to_scene(path, solids, options, "OpenCASCADE BRep")
}

#[cfg(feature = "cad-occt")]
pub fn load_fcstd(path: &Path, options: CadOptions) -> Result<SceneDocument, String> {
    let file = File::open(path).map_err(|e| format!("Could not open FCStd: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("FCStd ZIP container is invalid: {e}"))?;
    let mut objects = Vec::new();
    let mut candidates = 0usize;
    let mut failures = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| format!("FCStd ZIP entry error: {e}"))?;
        let name = entry.name().to_string();
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".brp") || lower.ends_with(".brep")) {
            continue;
        }
        candidates += 1;

        let mut bytes = Vec::new();
        if let Err(e) = entry.read_to_end(&mut bytes) {
            failures.push(format!("{name}: could not read ({e})"));
            continue;
        }

        let solids = match read_brep_payload(&bytes) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };

        let label = Path::new(&name)
            .file_stem()
            .and_then(|v| v.to_str())
            .filter(|v| !v.is_empty())
            .unwrap_or("FreeCAD shape");
        append_solids(&mut objects, solids, options, "FreeCAD / OpenCASCADE BRep", label)?;
    }

    if objects.is_empty() {
        let detail = if candidates == 0 {
            "the FCStd archive contains no .brp/.brep shape payloads".to_string()
        } else if failures.is_empty() {
            "no renderable BRep solids were found".to_string()
        } else {
            failures.join("; ")
        };
        return Err(format!("FCStd geometry import failed: {detail}"));
    }

    let mut scene = SceneDocument::new(
        path.file_name().and_then(|v| v.to_str()).unwrap_or("FreeCAD document"),
        objects,
    )?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

#[cfg(not(feature = "cad-occt"))]
pub fn load_step(_path: &Path, _options: CadOptions) -> Result<SceneDocument, String> {
    Err("STEP/STP backend is disabled in this diagnostic build (Cargo feature `cad-occt` is off)".into())
}

#[cfg(not(feature = "cad-occt"))]
pub fn load_brep(_path: &Path, _options: CadOptions) -> Result<SceneDocument, String> {
    Err("BREP/BRP backend is disabled in this diagnostic build (Cargo feature `cad-occt` is off)".into())
}

#[cfg(not(feature = "cad-occt"))]
pub fn load_fcstd(_path: &Path, _options: CadOptions) -> Result<SceneDocument, String> {
    Err("FCStd CAD backend is disabled in this diagnostic build (Cargo feature `cad-occt` is off)".into())
}

#[cfg(feature = "cad-occt")]
fn read_brep_payload(bytes: &[u8]) -> Result<Vec<Solid>, String> {
    if looks_like_text_brep(bytes) {
        return Err("text/ASCII OpenCASCADE BRep is not accepted by the current embedded binary BRep reader; export the shape as STEP/STP or binary BRep first".into());
    }
    let mut cursor = Cursor::new(bytes);
    Solid::read_brep(&mut cursor).map_err(|e| format!("binary BRep reader failed: {e}"))
}

#[cfg(feature = "cad-occt")]
fn looks_like_text_brep(bytes: &[u8]) -> bool {
    let prefix = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(prefix);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("DBRep_DrawableShape") || text.starts_with("CASCADE Topology V")
}

#[cfg(feature = "cad-occt")]
fn solids_to_scene(
    path: &Path,
    solids: Vec<Solid>,
    options: CadOptions,
    material: &str,
) -> Result<SceneDocument, String> {
    if solids.is_empty() {
        return Err(format!("{material} importer returned no solids"));
    }
    let mut objects = Vec::new();
    append_solids(&mut objects, solids, options, material, "Solid")?;
    let mut scene = SceneDocument::new(
        path.file_name().and_then(|v| v.to_str()).unwrap_or("CAD model"),
        objects,
    )?;
    scene.source_path = Some(path.to_path_buf());
    Ok(scene)
}

#[cfg(feature = "cad-occt")]
fn append_solids(
    objects: &mut Vec<SceneObject>,
    solids: Vec<Solid>,
    options: CadOptions,
    material: &str,
    prefix: &str,
) -> Result<(), String> {
    let total = solids.len();
    for (index, solid) in solids.iter().enumerate() {
        let mesh = Solid::mesh(std::slice::from_ref(solid), options.tessellation())
            .map_err(|e| format!("CAD tessellation failed for solid {}: {e}", index + 1))?;
        let mut stl_bytes = Vec::<u8>::new();
        mesh.write_stl(&mut stl_bytes)
            .map_err(|e| format!("CAD mesh conversion failed for solid {}: {e}", index + 1))?;

        let object_name = if total == 1 {
            prefix.to_string()
        } else {
            format!("{prefix} {}", index + 1)
        };
        let mut scene_mesh = stl::load_bytes(&object_name, &stl_bytes)?;
        scene_mesh.material_count = 1;
        scene_mesh.material_name = material.to_string();
        scene_mesh.source_mesh_count = 1;
        objects.push(SceneObject::new(object_name, scene_mesh));
    }
    Ok(())
}
