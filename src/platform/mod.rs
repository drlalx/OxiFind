//! Platform abstraction layer for desktop actions, elevation, and system integration.

pub mod desktop;
pub mod elevation;
pub mod mounts;

#[cfg_attr(not(windows), allow(unused_imports))]
pub use desktop::explorer_argument;
pub use desktop::{OpenAction, open_path};
pub use elevation::{is_elevated, restart_elevated};
#[allow(unused_imports)]
pub use mounts::{default_local_roots, default_system_exclusions, is_path_excluded};
