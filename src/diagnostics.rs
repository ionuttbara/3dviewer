use std::{
    fs::{create_dir_all, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

fn log_path() -> &'static PathBuf {
    LOG_PATH.get_or_init(|| {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("Gallery Inc").join("3D Viewer");
        let _ = create_dir_all(&dir);
        dir.join("runtime.log")
    })
}

fn working_log_path() -> Option<PathBuf> {
    std::env::current_dir().ok().map(|dir| dir.join("runtime.log"))
}

fn append_line(path: &PathBuf, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

pub fn install() {
    let primary = log_path().clone();
    let _ = std::fs::write(&primary, "");
    if let Some(cwd) = working_log_path() {
        if cwd != primary {
            let _ = std::fs::write(cwd, "");
        }
    }
    log("3D Viewer 0.2.2 startup diagnostics initialized");
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|v| format!("{}:{}:{}", v.file(), v.line(), v.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let payload = if let Some(v) = info.payload().downcast_ref::<&str>() {
            (*v).to_string()
        } else if let Some(v) = info.payload().downcast_ref::<String>() {
            v.clone()
        } else {
            "non-string panic payload".to_string()
        };
        log(format!("PANIC at {location}: {payload}"));
    }));
}

pub fn log(message: impl AsRef<str>) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0);
    let line = format!("[{stamp}] {}", message.as_ref());
    let primary = log_path();
    append_line(primary, &line);
    if let Some(cwd) = working_log_path() {
        if &cwd != primary {
            append_line(&cwd, &line);
        }
    }
}

pub fn display_path() -> String {
    match working_log_path() {
        Some(cwd) if &cwd != log_path() => format!("{}\n{}", log_path().display(), cwd.display()),
        _ => log_path().display().to_string(),
    }
}
