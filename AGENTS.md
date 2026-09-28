# 自动解压工具（Rust + Tauri 版）

`D:\Code\unzip` 的 Rust 重写。功能对齐 Python 版（魔数识别伪装包、散卷归集、改名卷领养、密码库策略、lz4 含 legacy 链式解码、apk 产物落位、按外层目录分组），新增捆绑应急解压层。

## 结构

- `crates/unzip-core` — 纯逻辑库（无 UI 依赖），全部行为语义以 `D:\Code\zip-rs` 不存在、以 `D:\Code\unzip\unzip_core.py` 为基准逐条对齐；模块：config / sniff / scan / extract / assemble / pipeline / lz4mini / types
- `crates/unzip-cli` — 命令行（对齐 `auto_unzip.py`，含 --dry-run）
- `app/` — Tauri v2 + Vue 3 桌面应用（独立 cargo 工程，`src-tauri/Cargo.toml` 带空 `[workspace]` 与根 workspace 隔离）
- `assets/bundled/` — 应急层：7z.exe+7z.dll（7-Zip 26.03, LGPL）、UnRAR.exe（RARLAB, 仅解压许可）、License.txt、NOTICE.txt

## 常用命令

- 测试：`cargo test --workspace`（集成测试用真实 7z/UnRAR/lz4，外部工具缺失时对应用例自动 skip）
- CLI：`cargo run --release -p unzip-cli -- <目录> [-o 输出] [--dry-run]`
- GUI 开发：`cd app && npm install && npm run tauri dev`（devUrl 端口 1520，1420 被本机 Hyper-V 保留）
- 打包：`cd app && sync-bundled.bat && npm run tauri build`（NSIS 安装包；应急组件在 `app/src-tauri/resources/bundled/`，由 sync-bundled.bat 从 `assets/bundled/` 同步——resources 必须位于 src-tauri 内，写 `../` 相对路径会被 NSIS 编成 `_up_` 段装到安装目录之外）

## 关键约定

- **config 位置**：CLI 默认 exe 旁 `config.json`；Tauri 启动时设 `UNZIP_CONFIG_PATH` 指向应用数据目录，并从 Python 版 `D:\Code\unzip\config.json` 首启迁移。
- **捆绑工具回退链**：用户配置的外部 7z/UnRAR 路径优先；不存在时用 `UNZIP_BUNDLED_DIR`（或 exe 旁 `bundled/`）里的内置组件。lz4 解码内置 Rust 实现（`lz4mini`）优先，外部 lz4.exe 仅回退。
- **用户可见文案**（日志/警告/错误）与 Python 版逐字一致；改语义时必须同步对照 Python 版。
- **文件安全**：源目录文件只读（失败移目录是显式设计）；测试只允许写 tempfile 目录；批量改名/删除逻辑改动后必须跑集成测试。
- 卷名/相似度匹配一律在 ASCII 折叠后的字节上切片（中文等多字节文件名绝不允许按 lower() 后的字符串索引切片——曾因此 panic）。

## 已知与 Python 版的差异（有意）

- 扫描两遍化：`stem.rar` 是否算卷族首卷不再依赖目录枚举顺序（确定性，见 scan.rs）。
- lz4 解码顺序：内置优先（Python 是 pip lz4 → 内置 → exe）。
- 改名卷候选排序用 jaro_winkler（Python 是 difflib ratio），只影响尝试顺序。
- rar 加密头探测空密码显式 `-p` 防交互卡死（语义同 Python）。
