use std::path::PathBuf;

use clap::Parser;
use rmcp::ServiceExt;
use unzip_mcp::ArchiveServer;

#[derive(Parser)]
#[command(version, about = "unzip-rs local stdio MCP archive server")]
struct Args {
    /// Absolute directory containing config.json and pack_mapping.sqlite3.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Absolute directory containing bundled 7z.exe/7z.dll and UnRAR.exe.
    #[arg(long)]
    bundled_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if let Some(dir) = args.data_dir {
        if !dir.is_absolute() {
            return Err("--data-dir requires an absolute path".into());
        }
        std::env::set_var("UNZIP_CONFIG_PATH", dir.join("config.json"));
        std::env::set_var("UNZIP_CATALOG_PATH", dir.join("pack_mapping.json"));
    } else if std::env::var_os("UNZIP_CONFIG_PATH").is_none() {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            // Match desktop migration before resolving a possibly empty new directory.
            let base = PathBuf::from(appdata);
            unzip_core::storage::migrate_directory(
                &base.join(unzip_core::storage::LEGACY_APP_DIR_NAME),
                &base.join(unzip_core::storage::APP_DIR_NAME),
                true,
            )?;
            std::env::set_var("UNZIP_CONFIG_PATH", base.join("unzip-rs/config.json"));
        }
    }
    if let Some(dir) = args.bundled_dir {
        if !dir.is_absolute() || !dir.is_dir() {
            return Err("--bundled-dir requires an existing absolute directory".into());
        }
        std::env::set_var("UNZIP_BUNDLED_DIR", dir);
    } else if std::env::var_os("UNZIP_BUNDLED_DIR").is_none() {
        let exe_dir = unzip_core::config::exe_dir();
        let mut candidates = vec![exe_dir.join("resources/bundled"), exe_dir.join("bundled")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local).join("unzip-rs/resources/bundled"));
        }
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/bundled"));
        if let Some(dir) = candidates.into_iter().find(|p| p.join("7z.exe").is_file()) {
            std::env::set_var("UNZIP_BUNDLED_DIR", dir);
        }
    }
    let server = ArchiveServer::new();
    let service = server.clone().serve(rmcp::transport::stdio()).await?;
    let result = service.waiting().await;
    server.shutdown().await;
    result?;
    Ok(())
}
