# 压缩记录与管理功能实现摘要

更新日期：2026-10-16。对应当前工作区实现，正式记录已迁移到 SQLite。

## 核心流程

```text
选择源文件/目录 → 7z 打包 → 收集产物并处理单文件分卷改名
    → 分别计算产物 SHA-256 → 立即合并到 AppData SQLite 数据库
    → 集中保存失败时尝试输出目录恢复副本，并告警

扫描解压输入 → 可选 SHA-256 匹配记录密码
    → 命中密码 + 原有规则/密码库/运行时密码 → 分卷预组装与解包
```

哈希只提供密码候选，解压器继续负责校验和解包。哈希失败不会改变源文件的移动、删除或解压判定规则。

## 模块职责

| 文件 | 当前职责 |
|---|---|
| `crates/unzip-core/src/pack.rs` | 打包、分卷改名、生成带哈希的记录、逐条保存、恢复 JSON 兜底 |
| `crates/unzip-core/src/archive_hash.rs` | 使用 `sha2` 流式计算 SHA-256；1 MiB 缓冲区，按块检查取消，并检查文件大小和修改时间是否变化 |
| `crates/unzip-core/src/archive_catalog.rs` | 定位 SQLite 数据库、首次迁移旧 JSON、事务化按 ID 合并、JSON 导入导出和哈希密码索引 |
| `crates/unzip-core/src/config.rs` | `hash_match` 默认值与配置读写；密码置顶时合并最新配置 |
| `crates/unzip-core/src/pipeline.rs` | 将哈希密码候选接入预组装和正式解包；预演和直接落位产物不比对哈希 |
| `app/src-tauri/src/main.rs` | 注册管理命令，指定 AppData 路径，将任务事件传给前端 |
| `app/src/components/LibraryView.vue` | 加载集中记录、导入旧 JSON、导出备份、按条目串行保存元数据 |
| `app/src/components/GameCard.vue` | 备注、动态字段、密码可见性、产物哈希和统计展示 |
| `app/src/composables/useLibraryFields.ts` | 字段定义、稳定 ID、旧元数据迁移、密码显示偏好 |

## 桌面端命令

| 命令 | 作用 |
|---|---|
| `load_archive_library` | 返回 SQLite 路径及兼容 JSON 格式的当前记录数据 |
| `update_archive_metadata` | 根据 `record_id` 更新一个条目的元数据 |
| `import_archive_library` | 自动导入已知目录 JSON，或弹出文件选择框导入 |
| `export_archive_library` | 弹出保存对话框，导出集中记录 |
| `save_tool_paths` | 校验并仅合并共用工具路径，避免覆盖解压设置 |

旧 `load_mapping_file` / `save_mapping_file` 已由上述集中记录命令替代。

## 一致性与兼容性

- 集中记录按压缩文件绝对路径生成 ID；Windows 下统一斜杠并转为小写。同名但路径不同的条目不会合并。
- 进程 mutex 与独立 `.json.lock` 文件的 OS 排他锁覆盖“读最新记录 → 合并 → 保存”全过程。锁文件不随 JSON 替换，句柄关闭后释放锁。
- SQLite 事务负责集中记录的一致性；首次迁移旧 JSON 后保留原文件，解析失败不回退为空记录写入。
- 打包 upsert 保留备注和未知字段；元数据更新不接收整份前端记录快照。
- 哈希索引按字节数和 SHA-256 查找，本次任务按输入路径缓存结果；相同哈希对应多个不同密码时保留候选顺序并去重。
- 旧目录映射可导入；已有同路径集中记录优先。旧记录无哈希时不自动读盘补算。
- 字段定义与密码显示开关保存在 localStorage；JSON 保留字段值，隐藏密码不加密文件。

## 相关改动

- 共用工具路径移入右上角应用设置，显式保存后下一次任务生效，无需重启；正在运行的任务继续使用启动时路径。
- 游戏库扩展为通用压缩包管理，版本号迁入备注，分类等字段可自定义。
- Windows 资源管理器右键入口可将多选文件或目录加入压缩/解压列表；入口按用户设置启用。
- ZIP64 复合包识别修复，避免将包含游戏归档的视频前缀文件当作普通媒体跳过。具体识别与文件安全约定见 [AGENTS.md](./AGENTS.md)。

## 检查记录与限制

SQLite 阶段的检查结果以本次实现后的实际执行为准；历史阶段的测试数量和构建结果不能作为本阶段的验证结果。

当前管理页通过 SQLite 查询单页记录（每页 24 条），搜索、筛选和排序在数据库执行；全库统计与匹配总数不受分页影响。尚无自动修复损坏记录或完整的同路径版本历史。数据库位置与迁移边界见 [数据存储说明](./STORAGE.md#sqlite-记录实现)。

字段规格和使用方式以 [PACK_MAPPING.md](./PACK_MAPPING.md) 为准；历史 UI 变更见 [UI 变更记录](../CHANGELOG_UI_REDESIGN.md)。
