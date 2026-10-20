# 项目实现摘要

更新日期：2026-10-20。对应 `v0.1.26102001`（Pre-release），正式记录采用 SQLite。版本变化见 [更新日志](../CHANGELOG.md)。

## 核心流程

```text
选择源文件/目录 → 严格配置与路径校验 → 暂存源内容及独占创建混淆说明
    → 在输出目录的私有暂存目录执行 7z，要求返回码 0
    → 收集数字分卷并处理单文件分卷改名 → 独占发布，不覆盖旧包
    → 分别计算产物 SHA-256 → 立即合并到 AppData SQLite 数据库
    → 集中保存失败时尝试输出目录恢复副本，并告警

扫描解压输入 → 可选 SHA-256 匹配记录密码
    → 命中密码 + 原有规则/密码库/运行时密码 → 分卷预组装与解包
```

哈希只提供密码候选，解压器继续负责校验和解包。哈希失败不会改变源文件的移动、删除或解压判定规则。

## 模块职责

| 文件 | 当前职责 |
|---|---|
| `crates/unzip-core/src/pack.rs` | 命名避让、检查暂存错误、私有目录压缩、独占发布、数字分卷排序、哈希记录和恢复 JSON 兜底 |
| `crates/unzip-core/src/archive_hash.rs` | 使用 `sha2` 流式计算 SHA-256；1 MiB 缓冲区，按块检查取消，并检查文件大小和修改时间是否变化 |
| `crates/unzip-core/src/archive_catalog.rs` | 定位 SQLite 数据库、首次迁移旧 JSON、事务化按 ID 合并、JSON 导入导出和哈希密码索引 |
| `crates/unzip-core/src/config.rs` | 严格配置读取与兼容读取接口、进程内串行合并更新；损坏配置不被部分设置更新覆盖 |
| `crates/unzip-core/src/pipeline.rs` | 将哈希密码候选接入预组装和正式解包；预演和直接落位产物不比对哈希 |
| `crates/unzip-mcp/src/lib.rs`、`jobs.rs` | stdio MCP 工具、后台任务、进度与取消、密码响应隐藏及任务异常恢复 |
| `app/src-tauri/src/main.rs` | AppData 路径和命令入口、任务互斥、统一后台线程及完成／异常事件、管理记录命令 |
| `app/src-tauri/src/shell_integration.rs` | 右键子菜单、单实例转发和直接任务队列 |
| `app/src/components/LibraryView.vue` | SQLite 分页查询、导入导出、元数据保存队列、失败草稿与重试、编辑结束后的刷新 |
| `app/src/components/GameCard.vue` | 备注、动态字段、密码可见性、产物哈希和统计展示 |
| `app/src/composables/useLibraryFields.ts` | 字段定义、稳定 ID、旧元数据迁移、密码显示偏好 |
| `app/src/composables/useTaskLog.ts` | 两页共享日志，保留最近 5000 行并合并滚动更新 |
| `app/src/composables/useShellQueue.ts` | 直接任务排队和两页运行状态协调 |
| `app/src/components/StyledSelect.vue`、`ThemeSettings.vue` | 下拉键盘交互、菜单定位、设置栏目和弹窗焦点管理 |
| `.github/workflows/release.yml`、`app/scripts/prepare-release.mjs` | Windows 签名构建、MCP 集成、更新清单与 Release 草稿 |

## 桌面端命令

| 命令 | 作用 |
|---|---|
| `load_archive_library` | 返回 SQLite 路径及兼容 JSON 格式的当前记录数据 |
| `query_archive_library` | SQLite 分页搜索、字段筛选、排序、全库统计和筛选选项 |
| `update_archive_metadata` | 根据 `record_id` 更新一个条目的元数据 |
| `import_archive_library` | 自动导入已知目录 JSON，或弹出文件选择框导入 |
| `export_archive_library` | 弹出保存对话框，导出集中记录 |
| `save_tool_paths` | 校验并仅合并共用工具路径，避免覆盖解压设置 |
| `load_config`、`save_config` | 严格读取配置，合并保存解压设置并保留最新共用工具及系统集成偏好 |
| `start_run`、`start_pack`、`retry_unpack_password` | 校验输入后占用共享任务槽，启动后台工作并发送完成事件 |

表中为主要命令；完整注册列表见 `app/src-tauri/src/main.rs`。旧 `load_mapping_file` / `save_mapping_file` 已由集中记录命令替代。

## 一致性与兼容性

- 集中记录按压缩文件绝对路径生成 ID；Windows 下统一斜杠并转为小写。同名但路径不同的条目不会合并。
- 集中记录由进程 mutex 和 SQLite 事务协调读写；跨进程数据库竞争由 SQLite 处理，连接设置 30 秒 busy timeout。旧 `.json.lock` 不再承担 SQLite 数据库的锁定。
- 首次迁移旧 JSON 在事务中完成，保留原文件，解析失败不回退为空记录写入。
- 打包 upsert 保留备注和未知字段；元数据更新不接收整份前端记录快照。
- 哈希索引按字节数和 SHA-256 查找，本次任务按输入路径缓存结果；相同哈希对应多个不同密码时保留候选顺序并去重。
- 旧目录映射可导入；已有同路径集中记录优先。旧记录无哈希时不自动读盘补算。
- 字段定义与密码显示开关保存在 localStorage；JSON 保留字段值，隐藏密码不加密文件。
- 元数据按条目和快照排队，查询结果叠加待保存内容；编辑框失焦后才执行保存触发的刷新。失败草稿只保留在会话内，存在失败项时阻止导出。
- GUI 手动压缩、解压、重试及右键直接任务使用同一个 busy 状态；后台线程创建失败或 panic 均释放占用，完成事件在释放后发送。MCP 的任务槽属于独立进程，不与桌面任务槽互锁。
- 打包暂存只读源文件，硬链接失败回退独占创建并复制；写入混淆说明时不复用源路径。压缩成功后才发布，发布冲突不覆盖已有文件，失败时尝试回滚本次已发布文件。

## 相关改动

- 共用工具路径移入右上角应用设置，显式保存后下一次任务生效，无需重启；正在运行的任务继续使用启动时路径。
- 游戏库扩展为通用压缩包管理，版本号迁入备注，分类等字段可自定义。
- Windows 资源管理器右键入口可将多选文件或目录加入压缩/解压列表；入口按用户设置启用。
- ZIP64 复合包识别修复，避免将包含游戏归档的视频前缀文件当作普通媒体跳过。具体识别与文件安全约定见 [AGENTS.md](./AGENTS.md)。

## 检查记录与限制

2026-10-20 审查阶段通过：

| 检查 | 结果及范围 |
| --- | --- |
| Rust 工作区 | 核心 173、MCP 7，共 180 项通过 |
| Tauri 主程序 | 14 项通过；Rust 总计 194 项 |
| 前端 | `npm run build`、`npx tsc --noEmit` 通过；后者不等同于完整 `vue-tsc` 检查 |
| 浏览器回归 | 以下 6 组通过，使用 Playwright、Edge 和模拟 Tauri IPC |

从仓库根目录执行浏览器脚本；可用 `PLAYWRIGHT_MODULE` 指定已有 Playwright `index.mjs` 的绝对路径或文件 URL：

| 命令 | 覆盖范围 |
| --- | --- |
| `node app/tests/archive-library-pagination.mjs` | 分页、连续编辑、失败保存与重试、筛选、下拉键盘交互、设置入口及窄／宽布局 |
| `node app/tests/pack-naming.mjs` | 前后缀、数字起点、自动续号及命名预览 |
| `node app/tests/shell-integration.mjs` | 右键设置和队列、任务互斥、运行中参数禁用 |
| `node app/tests/unpack-settings.mjs` | 解压来源、密码和规则设置、输出目录交互 |
| `node app/tests/tool-path-settings.mjs` | 共用工具路径保存及失败处理 |
| `node app/tests/app-updater.mjs` | 更新提示、模拟下载、安装失败与重启交互 |

发布流水线另执行了后端及工作区测试、MCP stdio 与实际压缩／解压往返、签名构建和签名验证。发布前下载核对了远端四个附件，具体记录见 [更新与发布](./UPDATES.md#v0126102001-发布记录)。

以上检查不等于真实安装升级及原生资源管理器端到端验证，也未覆盖数十 GB 压力或突然断电。多分卷逐个发布不是一次原子事务；取消等待当前外部工具返回。完整审查范围见 [审查记录](../REVIEW_2026-10-20.md)。本次文档同步不重新执行功能测试。

当前管理页通过 SQLite 查询单页记录（每页 24 条），搜索、筛选和排序在数据库执行；全库统计与匹配总数不受分页影响。尚无自动修复损坏记录或完整的同路径版本历史。数据库位置与迁移边界见 [数据存储说明](./STORAGE.md#sqlite-记录实现)。

字段规格和使用方式以 [PACK_MAPPING.md](./PACK_MAPPING.md) 为准；历史 UI 变更见 [UI 变更记录](../CHANGELOG_UI_REDESIGN.md)。
