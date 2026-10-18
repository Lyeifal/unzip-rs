use std::{error::Error, fs, path::PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;

fn main() -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(std::env::args_os().nth(1).ok_or("Expected update directory")?);
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json"))?;
    let manifest: Value = serde_json::from_slice(&fs::read(directory.join("latest.json"))?)?;
    let version = manifest["version"].as_str().ok_or("Missing version")?;
    if config["version"] != version { return Err("Manifest version differs from app version".into()); }
    let entry = &manifest["platforms"]["windows-x86_64"];
    let encoded_key = config["plugins"]["updater"]["pubkey"].as_str().ok_or("Missing public key")?;
    let public_key = PublicKey::decode(&String::from_utf8(STANDARD.decode(encoded_key)?)?)?;
    let signature = Signature::decode(&String::from_utf8(STANDARD.decode(
        entry["signature"].as_str().ok_or("Missing signature")?,
    )?)?)?;
    let installer = directory.join(format!("unzip-rs_{version}_x64-setup.exe"));
    let mut bytes = fs::read(&installer)?;
    public_key.verify(&bytes, &signature, true)?;
    let signed_version = signature.trusted_comment().split('\t')
        .find_map(|field| field.strip_prefix("version:"));
    if signed_version != Some(version) {
        return Err("Installer's signed version differs from manifest version".into());
    }
    if let Some(byte) = bytes.first_mut() { *byte ^= 1; }
    if public_key.verify(&bytes, &signature, true).is_ok() {
        return Err("Tampered package unexpectedly passed signature verification".into());
    }
    println!("Update {version}: configured public key accepts installer and rejects tampered bytes");
    Ok(())
}
