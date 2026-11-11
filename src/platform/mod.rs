#[cfg(windows)] mod windows;

#[cfg(windows)] pub use windows::*;

#[cfg(not(windows))]
pub fn register_file_capabilities() -> Result<(), String> { Ok(()) }
#[cfg(not(windows))]
pub fn open_default_apps() -> Result<(), String> { Err("Default Apps integration is Windows-only".into()) }
