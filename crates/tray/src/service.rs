#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::start;
#[cfg(windows)]
pub use windows::{process_id, start};
