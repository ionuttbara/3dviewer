use std::{env, path::PathBuf, process::Command};
use winreg::{enums::*, HKCU};

const REGISTERED_NAME: &str = "3D Viewer";
const CAP_PATH: &str = "Software\\Gallery Inc\\Gallery3DViewer\\Capabilities";
const PROG_ID: &str = "Gallery3DViewer.Model";

pub fn register_file_capabilities() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    register(&exe).map_err(|e| e.to_string())
}

fn register(exe: &PathBuf) -> std::io::Result<()> {
    let exe_q = format!("\"{}\"", exe.display());
    let command = format!("{} \"%1\"", exe_q);

    let (prog, _) = HKCU.create_subkey(format!("Software\\Classes\\{PROG_ID}"))?;
    prog.set_value("", &"3D Model")?;
    let (icon, _) = prog.create_subkey("DefaultIcon")?;
    // Resource 1 is the application icon. Resource 2 is a distinct 3D-model
    // document icon embedded in the same single EXE.
    icon.set_value("", &format!("{},-2", exe_q))?;
    let (cmd, _) = prog.create_subkey("shell\\open\\command")?;
    cmd.set_value("", &command)?;

    let (app, _) = HKCU.create_subkey(CAP_PATH)?;
    app.set_value("ApplicationName", &REGISTERED_NAME)?;
    app.set_value("ApplicationDescription", &"Native 3D model viewer, converter and 3D-print estimator")?;
    let (fa, _) = app.create_subkey("FileAssociations")?;
    for ext in crate::import::ALL_EXTENSIONS {
        fa.set_value(format!(".{ext}"), &PROG_ID)?;
        let (ext_key, _) = HKCU.create_subkey(format!("Software\\Classes\\.{ext}\\OpenWithProgids"))?;
        ext_key.set_raw_value(PROG_ID, &winreg::RegValue { bytes: Vec::new().into(), vtype: REG_NONE })?;
    }

    let (registered, _) = HKCU.create_subkey("Software\\RegisteredApplications")?;
    registered.set_value(REGISTERED_NAME, &CAP_PATH)?;
    let _ = registered.delete_value("Gallery 3D Viewer");

    let exe_name = exe.file_name().and_then(|x| x.to_str()).unwrap_or("gallery-3d-viewer.exe");
    let (supported, _) = HKCU.create_subkey(format!("Software\\Classes\\Applications\\{exe_name}\\SupportedTypes"))?;
    for ext in crate::import::ALL_EXTENSIONS { supported.set_value(format!(".{ext}"), &"")?; }
    let (app_cmd, _) = HKCU.create_subkey(format!("Software\\Classes\\Applications\\{exe_name}\\shell\\open\\command"))?;
    app_cmd.set_value("", &command)?;
    Ok(())
}

pub fn open_default_apps() -> Result<(), String> {
    let uri = "ms-settings:defaultapps?registeredAppUser=3D%20Viewer";
    Command::new("cmd").args(["/C", "start", "", uri]).spawn().map_err(|e| e.to_string())?;
    Ok(())
}
