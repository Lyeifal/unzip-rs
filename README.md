# 自动解压工具（Rust + Tauri）

自动解压（魔数识别伪装包、散卷归集、密码库自动尝试、lz4 解码）+ 打包压缩（命名递增、自动密码、阈值分卷、混淆 txt）的 Windows 桌面工具。Rust 核心库，Tauri v2 + Vue 3 界面。

## 目录结构

```
unzip-rs/
├── crates/
│   ├── unzip-core/      # 核心库：解压 pipeline + 打包 pack 模块（纯逻辑，无 UI 依赖）
│   └── unzip-cli/       # 命令行（解压 + pack 子命令）
├── app/                 # Tauri 桌面应用（Vue 3 前端在 app/src，Rust 外壳在 app/src-tauri）
│   ├── src/components/  #   UnpackView.vue（解压页）、PackView.vue（打包页），App.vue 为页签壳
│   ├── src-tauri/       #   Tauri 外壳（命令、事件、配置迁移）
│   └── sync-bundled.bat #   把应急组件同步进打包资源（构建安装包前必跑）
├── assets/bundled/      # 内置 7z.exe/7z.dll（7-Zip 26.03）、UnRAR.exe 及许可证（git 跟踪）
└── AGENTS.md            # 给协作者/AI 的详细约定，改代码前必读
```

## 环境要求

- Windows + [Rust toolchain](https://rustup.rs/)（rustup 默认装即可）
- [Node.js](https://nodejs.org/) + npm
- 首次开发：`cd app && npm install`

## 开发模式（改代码热更新）

```powershell
cd app
npm run tauri dev
```

- 前端改动即时热更新；改了 `app/src-tauri/` 里的 Rust 会自动重编译重启
- devUrl 端口 1520（1420 被本机 Hyper-V 保留，已在配置里固定）

## 构建安装包（发布用）

```powershell
cd app
sync-bundled.bat          # 必跑：同步 assets\bundled → 打包资源（7z/UnRAR 应急组件）
npm run tauri build
```

产物：

```
app\src-tauri\target\release\bundle\nsis\自动解压工具_0.1.0_x64-setup.exe   ← 安装包（当前用户权限，双击即装）
app\src-tauri\target\release\unzip-app.exe                                   ← 便携版主程序
app\src-tauri\target\release\resources\                                      ← 便携版必须与 exe 同目录拷贝
```

## 命令行（不装 GUI 也能用）

```powershell
# 解压（在仓库根目录跑）
cargo run --release -p unzip-cli -- "D:\压缩包目录" -o "D:\解压输出" [--dry-run]

# 打包（每个源一个包，命名递增，超 3G 分卷 2G/个，自动密码，混入混淆 txt）
cargo run --release -p unzip-cli -- pack "D:\游戏A" "D:\游戏B" -o "D:\发布" --name G198
cargo run --release -p unzip-cli -- pack --help     # 全部参数
```

exe 在 `target\release\unzip-cli.exe`，可单独拷走；旁边放 `config.json` 可自定义 7z 路径和密码库。

## 测试

```powershell
cargo test --workspace    # 70 个用例；用到真实 7z/UnRAR 的集成用例在工具缺失时自动跳过
```

## 常见问题

**构建报 `failed to remove file ... unzip-app.exe` / 拒绝访问？**
应用在运行中，文件被占用。托盘退出或任务管理器结束 `unzip-app.exe` 后重新构建；命令行可用：
```powershell
taskkill /F /IM unzip-app.exe
```

**dev 模式起不来 / 端口被占？**
检查 1520 端口是否被占（`netstat -ano | findstr 1520`），杀掉占用进程即可。

**装好的应用配置在哪？密码库从哪来？**
`%APPDATA%\com.unzip.app\config.json`。首次启动会自动从 `D:\Code\unzip\config.json`（Python 旧版）迁移密码库与设置；应急 7z 组件随应用安装，无需另装 7-Zip。

**改了 `assets/bundled/` 里的组件后打包？**
重跑 `sync-bundled.bat` 再 `npm run tauri build`。

**打包的分卷在网盘上叫什么？**
超阈值自动产出 `G198.7z.001 / .002 …`（zip 为 `.zip.001`），下载者把同族分卷放一起用 7-Zip 打开 `.001` 即可。
