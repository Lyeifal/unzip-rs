# GitHub 自动更新与发布

应用使用 Tauri Updater，从 `https://github.com/Lyeifal/unzip-rs/releases/latest/download/latest.json` 检查稳定版。启动时只检查，不会自动下载或关闭应用；也可在“应用设置 → 应用更新”手动检查、下载并安装。检查失败不影响压缩、解压或管理记录。压缩或解压任务运行时禁止安装更新。

## 首次配置

更新公钥已写入 `app/src-tauri/tauri.conf.json`。对应签名私钥位于本机 `C:\Users\akxls\.config\unzip-rs\updater.key`，不在仓库中；请在安全位置备份。当前私钥未设置密码，勿放进代码、Release 附件或日志。不要重新生成密钥替换公钥，否则已安装版本无法验证新更新。

应用名称已统一为 `unzip-rs`，更新公钥和内部 identifier 保持原值。安装器会识别旧中文产品名称的安装登记，在升级时卸载旧程序文件并保留用户数据，首次启动再复制迁移配置、SQLite 与主题偏好；默认的新安装目录为 `%LOCALAPPDATA%\unzip-rs`。

在 GitHub 仓库 `Lyeifal/unzip-rs` 的 Settings → Secrets and variables → Actions 中添加：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件的完整文本 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 当前密钥没有密码，可不设置；使用带密码密钥时填写对应密码 |

更新服务器面向公开 Releases。若仓库是私有的，应使用独立的公开更新仓库或更新服务器，不能把个人 GitHub Token 打进应用。下载能否访问取决于用户网络是否可访问 GitHub。

## 发布新版本

1. 修改 `app/src-tauri/tauri.conf.json`、`app/src-tauri/Cargo.toml` 和 `app/package.json` 的版本号，更新相应锁文件；版本必须高于已发布稳定版。
2. 提交并推送代码，创建和推送同版本标签，例如配置版本为 `0.1.1` 时使用 `v0.1.1`。
3. `.github/workflows/release.yml` 在 Windows 上测试、构建 NSIS、签名，并生成 Release 草稿。工作流也可在指定版本标签上手动运行。
4. 检查草稿内含 `unzip-rs_<版本>_x64-setup.exe`、同名 `.sig` 和 `latest.json`，确认后发布为稳定版，并将其设为 Latest。草稿或预发布版本不会通过稳定更新地址分发。

已存在相同标签的 Release 时，创建草稿会失败，避免无意覆盖发布文件；可从工作流附件下载构建产物后人工处理。

首次使用更新功能需要手动安装这个已接入 Updater 的版本。旧的、不含 Updater 的安装包不会自行获得更新能力。应用当前只提供 Windows x64 更新。

## 本机构建签名更新

普通 `npx tauri build` 仍可在没有密钥时构建安装包。正式签名更新使用独立的发布配置：

```powershell
cd app
.\sync-bundled.bat
$env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $env:USERPROFILE '.config\unzip-rs\updater.key'
npx tauri build --config src-tauri/tauri.release.conf.json
node scripts/prepare-release.mjs
cargo run --manifest-path src-tauri/Cargo.toml --example verify_update --locked -- src-tauri/target/release/update
```

构建脚本输出在 `app/src-tauri/target/release/update/`，包含安装包、签名和更新清单。本地脚本只生成文件，不上传和发布；发布新版本时，`latest.json` 的版本、签名和下载 URL 必须对应同一份安装包。签名验证通过后才会运行安装器。Windows 会关闭应用，执行更新安装并重新打开，AppData 中的配置与 SQLite 数据不会被更新包清空。

## 验证范围

浏览器回归脚本为 `app/tests/app-updater.mjs`，模拟检查、进度、安装失败和重启，不会实际安装软件。后端测试覆盖更新与任务互斥；正式构建验证生成 `.sig` 和 `latest.json`，发布前使用与 Updater 相同的 minisign 验证库确认当前公钥接受安装包并拒绝被篡改的数据。首次发布前仍需使用两个递增版本，在独立安装环境中验证一次真实的 GitHub 下载和安装流程。
