//! unzip-core — 自动解压核心库。

pub mod assemble;
pub mod config;
pub mod extract;
pub mod lz4mini;
pub mod pack;
pub mod pipeline;
pub mod scan;
pub mod sniff;
pub mod types;

pub use config::{
    config_path, default_output_dir, exe_dir, format_rule, load_config, load_config_from,
    promote_password, save_config, save_config_to, Config, PasswordRule,
};
pub use pack::{run_pack, PackOptions, PackResult, PackSummary};
pub use pipeline::run;
pub use types::{
    ArchiveKind, LogLevel, NullCallback, Package, PkgKind, RunCallback, ScanResult, Summary,
    VolFamily, VolumeRef,
};
