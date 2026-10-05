# 自动解压工具（Rust + Tauri）

Windows 桌面工具，解决两个下载党日常痛点：

1. **带密码的压缩包不用到处翻密码**。平时把常用密码存进密码库（发布页看到的随手加进去），下载完成后一键解压——工具自动按密码库逐个尝试，命中的还会置顶记忆，下次更快。再也不用在「文件提供者发布页 → 聊天记录 → 便签」之间来回找密码。
2. **打包发布一站搞定**。批量把文件/目录打成加密压缩包，命名从 `G198` 这类起点自动递增（宽度保留，Z999→Z1000），超过阈值自动分卷，每个包混入内容唯一的 `资源说明.txt`（防网盘按 hash 秒传比对），方便用包名快速索引。

技术上是个 Rust 核心库（`unzip-core`，纯逻辑无 UI）+ 命令行（`unzip-cli`）+ Tauri v2 + Vue 3 桌面应用。核心能力是**按内容魔数识别文件**，所以发布组各种伪装手法都能认出来直接解。

## 能处理的发布组伪装手法

- **纯改后缀**：rar/zip/7z 内容改成 .jpg/.mp4/.pdf 等任意扩展名，按文件头魔数识别
- **真封面图 + 追加压缩档**（多形体）：真 jpg/png 后紧跟 rar/7z，整包识别并解出
- **完整 mp4 + 追加压缩档**：可正常播放的视频后追加 zip/rar/7z/xz（常见为「zip 装 1.exe + 尾部小档」，或加密 zip 内装 7z 分卷），自动雕出纯压缩档全链路解开；尾部命中带结构校验，真视频不会被误判
- **SFX 自解压包**：「双击开始解压」的 exe（PE 里藏着 7z/rar/zip），与游戏启动 exe 区分识别并解压
- **垃圾前缀包**：压缩档前面垫任意字节
- **改名/带标记分卷**：`x.7z.001删`、`x.7z.002旧`、夸克 `x.7z-<哈希>.001` 均归族
- **夸克分段下载**：单档被下载器按任意字节切成 `.001/.002/.003`，归集失败时自动拼接兜底
- **套娃分卷**：压缩包内再装规范命名分卷（zip 里装 7z.001~00N），解包阶段按族归组、只解首卷自动拼接
- **内层套娃**：包里（含单层/两层子目录里）还有压缩包会继续自动解到底；两层以下的游戏数据包（存档备份/Mod 包）不碰
- **同 stem 双格式**：`a.zip` + `a.rar` 同内容只解一份，绝不覆盖旧包
- **同名嵌套**：包内顶层目录与包同名时自动折叠，不再 out/X/X
- **下载中文件**：`.qkdownloading`（夸克）/`.downloading`（百度）半成品不碰，不会打断下载；随包广告 PDF 按普通文件跳过

密码相关：加密 zip/7z/rar（含加密头）、SFX、lz4 链式套娃统一走密码库自动尝试，命中即置顶。

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
└── AGENTS.md            # 给协作者/AI 的详细约定（含伪装手法识别速查表与全部测试锚点），改代码前必读
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

## 配置

`config.json` 放在 CLI exe 旁（GUI 则在 `%APPDATA%\com.unzip.app\config.json`）：

| 字段 | 说明 |
|---|---|
| `passwords` | 全局密码库，按序尝试，命中可自动置顶（`recent_first`） |
| `password_rules` | 按后缀/关键字给特定包前置专属密码（如所有 `.lz4` 先试某两个） |
| `password_strategy` | `recent_first`（命中置顶）/ `list_order` |
| `seven_zip` / `unrar` / `lz4` | 外部工具路径；缺省时回退 exe 旁内置组件（lz4 还有内置 Rust 解码器） |
| `output_dir` / `failed_dir` | 输出 / 失败包移入目录 |
| `product_exts` | 产物扩展名（.apk 等），只落位不解压 |
| `max_depth` | 内层解包递归深度上限 |

## 测试

```powershell
cargo test --workspace    # 99 个用例；用到真实 7z/UnRAR/lz4/7z.sfx 的集成用例在工具缺失时自动跳过
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
`%APPDATA%\com.unzip.app\config.json`。应急 7z 组件随应用安装，无需另装 7-Zip。

**改了 Rust 核心后用 GUI 测试没变化？或直接 cargo 编的 GUI exe 打开白屏/「无法访问此页面」？**
GUI 是独立 cargo 工程（`app/src-tauri`），且前端资源由 **tauri CLI** 在构建时嵌入——裸 `cargo build --release` 编出的 exe **不嵌前端**，必须走 tauri 构建：
```powershell
cd app
npx tauri build --no-bundle                    # 快速：只出 exe（app\src-tauri\target\release\unzip-app.exe）
sync-bundled.bat && npm run tauri build        # 完整：exe + NSIS 安装包
```
便携版 exe 需与 `resources\` 目录同拷（内置 7z/UnRAR 应急组件）。

**分卷报「缺少 x.7z.00N / 分卷组装后校验未通过」？**
这是真缺卷：7z 首卷头部声明的元数据位置（NextHeaderOffset+32）指向全集末尾，与已有分卷合计大小一比就知道缺多少尾数据。回分享页把缺的卷/分段下完放回同目录即可，命名随意（带「删」尾标、夸克哈希名都认）。

**改了 `assets/bundled/` 里的应急组件后打包？**
重跑 `sync-bundled.bat` 再 `npm run tauri build`。

**压缩包解出了一层还是压缩包？**
正常：套娃结构（如 mp4→zip→SFX exe→7z 分卷）会逐层自动解到底，失败的层会留在原地并记警告，不会误删。

**打包的分卷在网盘上叫什么？**
超阈值自动产出 `G198.7z.001 / .002 …`（zip 为 `.zip.001`），下载者把同族分卷放一起用 7-Zip 打开 `.001` 即可。

## 开源协议与致谢

- 本项目：MIT License（见 LICENSE）
- 内置应急组件：7-Zip 26.03（LGPL，见 `assets/bundled/License.txt`）、UnRAR（RARLAB，仅解压许可，见 `assets/bundled/NOTICE.txt`），许可证文件随发布包分发
