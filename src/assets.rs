use std::{
    fs,
    path::{Path, PathBuf},
};

const TEXTURE_DIR_NAMES: &[&str] = &[
    "textures", "texture", "maps", "materials", "material", "images", "image",
];

const TEXTURE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tga", "tif", "tiff", "dds",
];

const MAX_SCANNED_ENTRIES: usize = 4096;

#[derive(Clone, Debug, Default)]
pub struct TextureDiscovery {
    pub files: Vec<PathBuf>,
    pub searched_directories: Vec<PathBuf>,
}

pub fn discover_textures(model_path: &Path) -> TextureDiscovery {
    let Some(parent) = model_path.parent() else {
        return TextureDiscovery::default();
    };

    let mut out = TextureDiscovery::default();
    let mut budget = MAX_SCANNED_ENTRIES;

    // First inspect image files next to the model itself.
    scan_directory(parent, 0, &mut budget, &mut out.files);
    out.searched_directories.push(parent.to_path_buf());

    // Then inspect well-known texture/material directories, case-insensitively.
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if budget == 0 {
                break;
            }
            budget = budget.saturating_sub(1);
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if TEXTURE_DIR_NAMES.iter().any(|candidate| *candidate == name.as_str()) {
                out.searched_directories.push(path.clone());
                scan_directory(&path, 2, &mut budget, &mut out.files);
            }
        }
    }

    out.files.sort_by(|a, b| a.to_string_lossy().to_ascii_lowercase().cmp(&b.to_string_lossy().to_ascii_lowercase()));
    out.files.dedup_by(|a, b| path_eq_case_insensitive(a, b));
    out.searched_directories.sort_by(|a, b| a.to_string_lossy().to_ascii_lowercase().cmp(&b.to_string_lossy().to_ascii_lowercase()));
    out.searched_directories.dedup_by(|a, b| path_eq_case_insensitive(a, b));
    out
}

pub fn resolve_referenced_texture(model_path: &Path, reference: &str) -> Option<PathBuf> {
    let reference = reference.trim().trim_matches('"').trim_matches('\'');
    if reference.is_empty() {
        return None;
    }

    let reference_path = Path::new(reference);
    if reference_path.is_absolute() && reference_path.is_file() && is_texture_file(reference_path) {
        return Some(reference_path.to_path_buf());
    }

    let parent = model_path.parent()?;

    // Exact path from the model/MTL has highest priority.
    let direct = parent.join(reference_path);
    if direct.is_file() && is_texture_file(&direct) {
        return Some(direct);
    }

    let filename = reference_path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let discovery = discover_textures(model_path);
    discovery
        .files
        .into_iter()
        .find(|candidate| candidate.file_name().map(|v| v.to_string_lossy().to_ascii_lowercase()) == Some(filename.clone()))
}

pub fn choose_fallback_texture(model_path: &Path, discovery: &TextureDiscovery) -> Option<PathBuf> {
    if discovery.files.is_empty() {
        return None;
    }

    let model_stem = model_path
        .file_stem()
        .map(|v| normalize_name(&v.to_string_lossy()))
        .unwrap_or_default();

    // Rank likely base-colour maps instead of giving up as soon as a package contains
    // several PBR maps. This is common for STL downloads that ship a textures folder
    // with base-color, normal, roughness, AO and metallic images.
    let positive = ["basecolor", "basecolour", "base_color", "base_colour", "albedo", "diffuse", "color", "colour"];
    let negative = ["normal", "nrm", "rough", "roughness", "metal", "metallic", "ao", "ambientocclusion", "occlusion", "height", "displacement", "disp", "bump", "opacity", "alpha", "emissive", "emission", "specular"];

    let mut ranked: Vec<(i32, &PathBuf)> = discovery.files.iter().map(|path| {
        let file_stem = path.file_stem().map(|v| normalize_name(&v.to_string_lossy())).unwrap_or_default();
        let mut score = 0i32;
        if !model_stem.is_empty() && file_stem.contains(&model_stem) { score += 90; }
        if positive.iter().any(|k| file_stem.contains(k)) { score += 80; }
        if negative.iter().any(|k| file_stem.contains(k)) { score -= 140; }
        if path.parent().and_then(|p| p.file_name()).map(|v| {
            let n = v.to_string_lossy().to_ascii_lowercase();
            TEXTURE_DIR_NAMES.iter().any(|candidate| *candidate == n.as_str())
        }).unwrap_or(false) { score += 10; }
        (score, path)
    }).collect();

    ranked.sort_by(|(sa, pa), (sb, pb)| {
        sb.cmp(sa).then_with(|| pa.to_string_lossy().to_ascii_lowercase().cmp(&pb.to_string_lossy().to_ascii_lowercase()))
    });

    if let Some((score, path)) = ranked.first() {
        if *score > 0 || discovery.files.len() == 1 {
            return Some((*path).clone());
        }
    }

    // Last resort: use the first image that does not look like a technical PBR map.
    // This keeps texture-enabled STL packages useful even when authors use generic
    // names such as texture_01.png.
    ranked.iter()
        .find(|(score, _)| *score >= 0)
        .map(|(_, path)| (*path).clone())
        .or_else(|| discovery.files.first().cloned())
}

fn normalize_name(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

pub fn is_texture_file(path: &Path) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            TEXTURE_EXTENSIONS.iter().any(|candidate| *candidate == ext.as_str())
        })
        .unwrap_or(false)
}

fn scan_directory(dir: &Path, depth: usize, budget: &mut usize, output: &mut Vec<PathBuf>) {
    if *budget == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget = budget.saturating_sub(1);
        let path = entry.path();
        if path.is_file() && is_texture_file(&path) {
            output.push(path);
        } else if depth > 0 && path.is_dir() {
            scan_directory(&path, depth - 1, budget, output);
        }
    }
}

fn path_eq_case_insensitive(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}
