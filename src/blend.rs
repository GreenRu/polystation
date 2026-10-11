//! .blend support: runs Blender headless with `export_blend.py` and returns the exported files.
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = include_str!("export_blend.py");

pub struct Exported {
    pub obj: String,
    pub bmp: Option<String>,
}

fn find_blender() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BLENDER") {
        return Some(p.into());
    }
    for root in [r"C:\Program Files\Blender Foundation", r"C:\Program Files (x86)\Blender Foundation"] {
        let Ok(dir) = std::fs::read_dir(root) else { continue };
        let mut found: Vec<PathBuf> = dir.flatten().map(|e| e.path().join("blender.exe")).filter(|p| p.exists()).collect();
        found.sort(); // newest version last
        if let Some(p) = found.pop() {
            return Some(p);
        }
    }
    Some("blender".into()) // fall back to PATH
}

/// Model formats Blender can import (converted to the engine's OBJ form through the same exporter).
/// Voxel formats are deliberately not in this list.
pub const FOREIGN: [&str; 15] = ["fbx", "gltf", "glb", "stl", "ply", "dae", "abc", "usd", "usda", "usdc", "usdz", "x3d", "wrl", "3ds", "svg"];

/// True for every file type that opens as a model: .obj, .blend and everything in [`FOREIGN`].
pub fn is_model(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ext == "obj" || ext == "blend" || FOREIGN.contains(&ext.as_str())
}

pub fn export(blend: &str) -> Result<Exported, String> {
    let blender = find_blender().ok_or("Blender not found (set the BLENDER env var)")?;
    let dir = std::env::temp_dir().join("gpurender_blend");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("export_blend.py");
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    let stem = Path::new(blend).file_stem().and_then(|s| s.to_str()).unwrap_or("model");
    let (obj, bmp) = (dir.join(format!("{stem}.obj")), dir.join(format!("{stem}.bmp")));
    let _ = std::fs::remove_file(&bmp);
    let _ = std::fs::remove_file(&obj); // never fall back to output from an earlier run

    println!("exporting {blend} with {} ...", blender.display());
    let foreign = !blend.to_lowercase().ends_with(".blend");
    let mut cmd = Command::new(&blender);
    cmd.args(["--factory-startup", "--background"]);
    if foreign {
        cmd.env("GPR_IMPORT", blend); // the script imports it into the empty startup scene
    } else {
        cmd.arg(blend);
    }
    let out = cmd
        .arg("--python")
        .arg(&script)
        .arg("--")
        .arg(&obj)
        .arg(&bmp)
        .output()
        .map_err(|e| format!("could not run Blender ({}): {e}", blender.display()))?;
    let log = String::from_utf8_lossy(&out.stdout);
    for l in log.lines().filter(|l| l.starts_with("GPURENDER:")) {
        println!("{l}");
    }
    // Blender exits 0 even when the script raised, so require the script's own completion marker.
    if !out.status.success() || !log.contains("GPURENDER: done") || !obj.exists() {
        return Err(format!("Blender export failed:\n{}{}", log, String::from_utf8_lossy(&out.stderr)));
    }
    Ok(Exported { obj: obj.to_string_lossy().into(), bmp: bmp.exists().then(|| bmp.to_string_lossy().into()) })
}

const CONVERT_SCRIPT: &str = include_str!("convert_image.py");

/// Converts any image Blender can read (png, jpg, tga ...) to a BMP next to the original and returns
/// its path. An existing .bmp that is at least as new as the source is reused.
pub fn convert_image(src: &str) -> Result<String, String> {
    let dst = Path::new(src).with_extension("bmp");
    if let (Ok(a), Ok(b)) = (std::fs::metadata(&dst), std::fs::metadata(src)) {
        if a.modified().ok() >= b.modified().ok() {
            return Ok(dst.to_string_lossy().into_owned());
        }
    }
    let blender = find_blender().ok_or("Blender not found (set the BLENDER env var)")?;
    let dir = std::env::temp_dir().join("gpurender_blend");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("convert_image.py");
    std::fs::write(&script, CONVERT_SCRIPT).map_err(|e| e.to_string())?;
    let out = Command::new(&blender)
        .args(["--factory-startup", "--background", "--python"])
        .arg(&script)
        .arg("--")
        .arg(src)
        .arg(&dst)
        .output()
        .map_err(|e| format!("could not run Blender ({}): {e}", blender.display()))?;
    let log = String::from_utf8_lossy(&out.stdout);
    if !log.contains("GPURENDER: converted") || !dst.exists() {
        return Err(format!("could not convert {src}:\n{}{}", log, String::from_utf8_lossy(&out.stderr)));
    }
    Ok(dst.to_string_lossy().into_owned())
}
