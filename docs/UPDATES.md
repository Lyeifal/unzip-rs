# GitHub 自动更新与发布

更新日期：2026-10-20。当前已发布 `v0.1.26102001`，类型为 **Pre-release**，四个附件均已上传；版本变化见 [更新日志](../CHANGELOG.md)。

应用使用 Tauri Updater，从 `https://github.com/Lyeifal/unzip-rs/releases/latest/download/latest.json` 检查稳定版。启动时只检查，不会自动下载或关闭应用；也可在“应用设置 → 应用更新”手动检查、下载并安装。检查失败不影响压缩、解压或管理记录。压缩或解压任务运行时禁止安装更新。

## 首次配置

更新公钥已写入 `app/src-tauri/tauri.conf.json`。维护者本机对应签名私钥位于 `%USERPROFILE%\.config\unzip-rs\updater.key`，不在仓库中；请在安全位置备份。当前私钥未设置密码，勿放进代码、Release 附件或日志。不要重新生成密钥替换公钥，否则已安装版本无法验证新更新。

应用名称已统一为 `unzip-rs`，更新公钥和内部 identifier 保持原值。安装器会识别旧中文产品名称的安装登记，在升级时卸载旧程序文件并保留用户数据，首次启动再复制迁移配置、SQLite 与主题偏好；默认的新安装目录为 `%LOCALAPPDATA%\unzip-rs`。

在 GitHub 仓库 `Lyeifal/unzip-rs` 的 Settings → Secrets and variables → Actions 中添加：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件的完整文本 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 当前密钥没有密码，可不设置；使用带密码密钥时填写对应密码 |

更新服务器面向公开 Releases。若仓库是私有的，应使用独立的公开更新仓库或更新服务器，不能把个人 GitHub Token 打进应用。下载能否访问取决于用户网络是否可访问 GitHub。

## 发布新版本

### 同步版本

版本采用 `0.1.YYMMDDNN` 形式，例如 `0.1.26102001`。新版本应高于前一个发布版本，同一天继续发布时递增末尾序号。Pre-release 使用 GitHub 的预发布标记，当前版本字符串仍为纯数字三段式。

| 文件 | 需要同步的位置 |
| --- | --- |
| `Cargo.toml` | 根工作区 `workspace.package.version` |
| `Cargo.lock` | `unzip-core`、`unzip-cli`、`unzip-mcp` 的版本 |
| `app/package.json`、`app/package-lock.json` | 前端包版本及锁文件根包版本 |
| `app/src-tauri/Cargo.toml`、`app/src-tauri/Cargo.lock` | 桌面包版本及其本地核心依赖版本 |
| `app/src-tauri/tauri.conf.json` | 应用与安装包版本 |

### 构建草稿

1. 完成版本同步，更新 [CHANGELOG.md](../CHANGELOG.md) 和 README 中的版本说明，提交源码。
2. 创建同版本标签，例如 `v0.1.26102001`，推送提交及标签。不要把本机日志、签名私钥或临时构建目录加入提交。
3. `.github/workflows/release.yml` 在 Windows 上运行后端和工作区测试，构建 MCP 集成包并检查 stdio 协议及实际压缩／解压往返，再构建签名 NSIS 安装包、生成清单和验证签名。
4. 流水线上传 `windows-update` 构建附件，并创建带以下四个文件的 **Release 草稿**。工作流也可在指定版本标签上手动运行；在分支上运行会被版本检查拒绝。

| Release 附件 | 用途 |
| --- | --- |
| `unzip-rs_<版本>_x64-setup.exe` | Windows x64 桌面安装包 |
| `unzip-rs_<版本>_x64-setup.exe.sig` | 对应安装包的 Updater 签名 |
| `latest.json` | 版本、下载 URL 和签名清单 |
| `unzip-rs-mcp_<版本>_windows-x86_64.zip` | MCP 可执行文件、内置工具、许可证、配套 skill 与客户端配置示例 |

### 发布 Pre-release

当前沿用预发布流程：

1. 等待流水线成功，确认标签指向目标源码，四个附件全部上传完成。
2. 核对下载文件的大小和 SHA-256，确认清单版本、下载 URL、`.sig` 与安装包一致。使用下方 `verify_update` 校验配置公钥及安装包签名；MCP zip 也应确认文件完整。
3. 编辑草稿，填入更新说明，勾选 **Pre-release**，发布时不设为 **Latest**。通过 API 发布时，对应字段为 `draft: false`、`prerelease: true`、`make_latest: "false"`。
4. 发布后核对版本标记和四个公开附件。用户从版本发布页手动下载安装；`latest.json` 即使作为预发布附件上传，也不会让稳定更新地址分发该版本。

### 发布稳定版

需要发布稳定版时，在完成验证后取消预发布标记，将该稳定版本设为 Latest。应用使用的 `/releases/latest/download/latest.json` 才会指向相应稳定版本；草稿和 Pre-release 不作为稳定更新。

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

MCP 集成包在仓库根目录单独构建：

```powershell
powershell -ExecutionPolicy Bypass -File integrations/build-bundle.ps1
```

输出为 `target/integrations/unzip-rs-mcp_<版本>_windows-x86_64.zip`。只打桌面安装包不会自动生成此 zip，接入说明见 [MCP 文档](../integrations/README.md)。

## v0.1.26102001 发布记录

- 源码提交：`e0523306c77bfc9e0dfc8f2d50e2ca08b90cbb0e`。
- 发布时间：2026-10-20 15:13（北京时间），类型为 Pre-release，未设为稳定版 Latest。
- [发布流水线](https://github.com/Lyeifal/unzip-rs/actions/runs/39383541101) 成功；发布前下载核对了四个附件的大小与 SHA-256，并验证清单、签名和 MCP zip 完整性。
- 发布地址：`https://github.com/Lyeifal/unzip-rs/releases/tag/v0.1.26102001`。

## 验证范围

浏览器回归脚本为 `app/tests/app-updater.mjs`，模拟检查、进度、安装失败和重启，不会实际安装软件。后端测试覆盖更新与任务互斥；正式构建验证生成 `.sig` 和 `latest.json`，发布前使用与 Updater 相同的 minisign 验证库确认当前公钥接受安装包并拒绝被篡改的数据。首次发布前仍需使用两个递增版本，在独立安装环境中验证一次真实的 GitHub 下载和安装流程。
