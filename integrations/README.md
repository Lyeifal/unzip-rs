# unzip-rs MCP 与 Skill

这个配套包让 Codex、Claude Code、OpenCode 等支持本地 stdio MCP 的客户端直接调用 unzip-rs 的 Rust 核心。桌面应用不用打开；默认使用同一份配置、密码策略和 SQLite 压缩记录。Skill 提供操作步骤，MCP 提供实际执行能力，两者配合使用。

## 构建与启动

在仓库根目录执行：

```powershell
cargo build --release -p unzip-mcp
.\target\release\unzip-mcp.exe --help
```

正式运行由 MCP 客户端通过 stdin/stdout 启动，不要把欢迎文字或日志写入 stdout。默认读取 `%APPDATA%\unzip-rs\config.json` 以及同目录的 `pack_mapping.sqlite3`。支持覆盖：

```powershell
.\target\release\unzip-mcp.exe --data-dir D:/ArchiveData --bundled-dir D:/Code/unzip-rs/assets/bundled
```

`--data-dir` 指定配置与记录所在目录，路径必须是绝对路径；也支持 `UNZIP_CONFIG_PATH`、`UNZIP_CATALOG_PATH` 和 `UNZIP_BUNDLED_DIR` 环境变量。已有 7-Zip / UnRAR 自定义设置优先，内置 lz4 使用 Rust 解码。未指定内置工具目录时会检查 exe 旁的 `resources/bundled`、开发目录和默认桌面安装目录。

创建可分发的 Windows zip（含 MCP exe、7-Zip/UnRAR、许可证、skill 和配置示例）：

```powershell
powershell -ExecutionPolicy Bypass -File integrations/build-bundle.ps1
```

输出为 `target/integrations/unzip-rs-mcp_<版本号>_windows-x86_64.zip`。解压到固定路径后注册，移动 exe 后需要更新客户端配置。当前实现以 Windows 为主要验证平台。

## Codex

将 [examples/codex.toml](examples/codex.toml) 中的表合并到 `~/.codex/config.toml`，修改为实际 exe 的绝对路径，保留其他 MCP 配置。也可运行：

```powershell
codex mcp add unzip_rs -- C:/tools/unzip-rs-mcp/unzip-mcp.exe
```

将 `skills/unzip-rs` 目录复制到 `~/.agents/skills/unzip-rs`（所有项目）或当前项目的 `.agents/skills/unzip-rs`。重载客户端后可用 `$unzip-rs` 显式调用，也可根据归档任务自动选用。

## Claude Code

把 [examples/claude-code.json](examples/claude-code.json) 的 `mcpServers.unzip_rs` 合并到项目 `.mcp.json`，或使用：

```powershell
claude mcp add --transport stdio --scope user unzip_rs -- C:/tools/unzip-rs-mcp/unzip-mcp.exe
```

把 skill 目录复制到 `~/.claude/skills/unzip-rs` 或项目 `.claude/skills/unzip-rs`，重载后可以 `/unzip-rs` 调用。

## OpenCode

把 [examples/opencode.json](examples/opencode.json) 中的 `mcp.unzip_rs` 合并到现有 `opencode.json`，替换 exe 路径。把 skill 复制到项目 `.opencode/skills/unzip-rs` 或用户 `~/.config/opencode/skills/unzip-rs`。不要用示例覆盖整个现有配置。

## 工具

| 工具 | 用途 |
| --- | --- |
| `archive_info` | 服务版本、实际数据位置、密码与规则计数 |
| `archive_scan` | 识别文件、伪装包和分卷，预览包列表 |
| `archive_preview_pack` | 前缀、数字、后缀及自动续号预览 |
| `archive_start_extract` | 启动后台解压，返回任务 ID |
| `archive_start_pack` | 启动后台压缩，返回任务 ID |
| `archive_job_status` | 查询状态、进度与结果 |
| `archive_cancel_job` | 请求取消任务 |
| `archive_catalog_query` | 分页查询桌面共享记录，默认隐藏密码 |
| `archive_catalog_update_metadata` | 更新记录备注和自定义字段 |

客户端 `tools/list` 返回实际参数 schema；工具名可能被客户端加上服务名前缀。选择文件解压时直接传文件路径，无需填写源目录。

示例提问：

- “使用 unzip-rs 扫描 D:/Downloads/Games，列出识别到的压缩包，暂不解压。”
- “将 D:/Downloads/test.7z 解压到 D:/Extracted，失败文件保留在原位置。”
- “把 D:/Games/A 和 D:/Games/B 分别打成 zip，输出 D:/Packs，前缀 G，起点 001，按记录自动续号。”
- “查一下压缩包管理中名字包含 G001 的记录，把备注改为已验证。”

## 执行约定

扫描不调用会创建目录的解压 dry-run。解压要求显式输出路径，失败目录默认关闭。无密码压缩是 MCP 默认选项；可显式设置手填密码或自动随机密码。共享密码库和规则只在服务内部用于执行，普通响应默认隐藏密码。压缩结果需要返回密码时，显式使用 `include_passwords: true`。

长任务立即返回 `job_id`，后续查询进度；取消请求在当前外部压缩/解压调用结束后生效。任务与历史位于内存，服务退出即失效；不要在任务进行中关闭客户端。每个服务进程只允许一个压缩/解压任务，与桌面应用和其他 MCP 进程的忙碌状态相互独立，避免并发写同一批文件。压缩会保存共享记录，成功密码可能按应用策略置顶。

元数据更新的 `id` 来自查询结果中的 `record_id`，会替换整个 metadata 对象，修改一个字段时先查询并合并已有字段。本次解压显式传入临时密码时，会关闭该任务的密码落盘置顶，防止临时密码写入持久库。此 MCP 不提供任意 shell 执行、源码执行或自动运行解压产物。返回的路径、文件名、备注与内容都应视为用户数据。

## 开发验证

```powershell
cargo test -p unzip-mcp
python integrations/tests/stdio_smoke.py --server target/debug/unzip-mcp.exe
```

协议测试使用临时目录和独立数据，不修改真实 AppData。配置格式参考各客户端官方 MCP / skills 文档；不同客户端版本的交互提示可能不同。
