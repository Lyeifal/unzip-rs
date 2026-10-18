# 压缩记录与映射格式

更新日期：2026-10-16。本文描述当前记录格式以及 SQLite 与 JSON 导入导出的兼容约定。

## 保存位置与时机

Windows 默认正式记录文件为 `%APPDATA%\unzip-rs\pack_mapping.sqlite3`。每成功打包一个条目，立即合并保存其名称、密码、大小和各产物哈希；正常情况下，输出目录只保存压缩文件。首次使用数据库时，若同目录存在旧 `pack_mapping.json`，会在事务中导入并保留原文件。

集中保存失败会显示警告，并尝试在打包输出目录写入 `pack_mapping.json` 恢复副本。两处都失败时，应保存打包结果中的密码。哈希计算取消或读取失败不会删除已产出的压缩文件，记录中可能只有部分哈希。

CLI 可通过 `UNZIP_CATALOG_PATH` 指定旧 JSON 路径及同名 `.sqlite3` 数据库位置；GUI 启动时使用自己的应用数据目录。配置、记录与界面偏好的位置见 [数据存储说明](./STORAGE.md)。

## JSON 导入导出结构

以下是带备注的旧记录导入后的示意，没有哈希。完整的单文件、分卷和旧字段示例见 [示例文件](./examples/pack_mapping_example.json)。

示例文件带有 `example_only: true` 标记，路径、大小、密码及重复数字组成的哈希均为展示数据，不对应真实压缩文件，不可用于实际完整性验证。

```json
{
  "schema_version": 2,
  "generated_at": "2026-10-16",
  "updated_at_unix": 1792108800,
  "total_count": 1,
  "mappings": [
    {
      "record_id": "d:/archives/documents.zip",
      "archive_path": "D:\\Archives\\Documents.zip",
      "output_dir": "D:\\Archives",
      "source_name": "项目文档",
      "archive_name": "Documents.zip",
      "password": null,
      "source_size": { "bytes": 1024, "mb": "0.00", "gb": "0.000", "human": "1024 B" },
      "compressed_size": { "bytes": 512, "mb": "0.00", "gb": "0.000", "human": "512 B" },
      "compression_ratio": "50.0%",
      "is_volume": false,
      "volume_count": 1,
      "created_at": "2026-10-16",
      "metadata": {
        "schema_version": 1,
        "notes": "工作文档备份",
        "custom_fields": { "category": "文档", "organized": true }
      }
    }
  ]
}
```

### 顶层字段

| 字段 | 类型 | 含义 |
|---|---|---|
| `schema_version` | number | 集中记录格式版本，当前为 `2` |
| `generated_at` | string | 最近一次保存日期，`YYYY-MM-DD`；尚未保存的空库可为空字符串 |
| `updated_at_unix` | number，可选 | 最近一次保存的 Unix 时间戳，单位为秒；旧记录和尚未保存的空库可缺省 |
| `total_count` | number | 条目数量，一组分卷计为一个条目 |
| `mappings` | array | 压缩记录；文件中的顺序不保证按名称排序，管理页负责排序 |

### 每条压缩记录

| 字段 | 类型 | 含义 |
|---|---|---|
| `record_id` | string | 根据压缩文件绝对路径生成的标识；Windows 下统一斜杠并转为小写 |
| `archive_path` | string | 打包时压缩文件的绝对路径；分卷为首卷路径 |
| `output_dir` | string | 压缩文件所在目录 |
| `source_name` | string | 源文件或目录名称 |
| `archive_name` | string | 压缩文件名；分卷为首卷完整名称，如 `G002.7z.001` |
| `password` | string 或 null | 解压密码；`null` 表示无密码 |
| `source_size` | object | 源内容大小：`bytes` 为整数，`mb/gb/human` 为显示字符串 |
| `compressed_size` | object | 压缩产物大小，分卷时为所有分卷之和 |
| `compression_ratio` | string | 压缩后大小 / 源大小，例如 `70.0%`；超过 100% 也是有效结果 |
| `is_volume` | boolean | 实际产物是否多于一个文件 |
| `volume_count` | number | 实际产物文件数量 |
| `created_at` | string | 条目打包日期，更新备注不会修改此字段 |
| `hash_algorithm` | string，可选 | 新打包记录为 `sha256` |
| `output_hashes` | array，可选 | 各产物的 `file_name`、`size_bytes` 和 64 位十六进制 `sha256` |
| `metadata` | object，可选 | 备注与用户自定义字段；未编辑的条目可能没有此字段 |

顶层 `schema_version: 2` 与 `metadata.schema_version: 1` 分别控制记录格式和旧字段迁移，含义不同。

## 合并与同名文件

- 集中记录按 `record_id` 合并，同名但不同输出路径的压缩包分别保存。
- 打包记录更新会保留该条目已有的备注、元数据和未知属性。
- 编辑备注只提交条目 ID 与元数据，不回写整份前端缓存，避免覆盖同时产生的新记录。
- 压缩文件实际移动后，已有记录的路径不会自动更新；内容哈希仍可用于查找密码。
- 记录集合只保存每个路径的当前条目，不是同一路径下所有历史内容的完整版本库。

## 哈希匹配密码

1. 打包完成且单文件分卷改名结束后，流式计算每个产物的 SHA-256。
2. 解压任务启用 `hash_match` 时，按输入文件大小查询数据库中的哈希索引行，并缓存本次任务查过的大小；该大小存在密码候选时才计算文件哈希。
3. 输入文件大小有候选时才计算哈希。命中后将记录密码放在规则、密码库和运行时追加密码之前尝试；解压器仍保留原有无密码探测。
4. 未命中、记录不可读、哈希无效或密码错误时，继续使用原有密码流程。

分卷通过待处理首卷查找密码，**首卷命中不等于整组分卷完整**，后续仍由解压器检查。关闭 `hash_match`、执行预演或处理 APK 等直接落位的产物时，不计算解压输入哈希。没有哈希的旧记录仍可整理，但不会自动补算哈希。

## 管理字段与密码显示

“应用设置 → 压缩包管理”可添加文本、下拉和勾选字段，也可控制密码是否显示，默认隐藏。备注保存在 `metadata.notes`；字段值保存在 `metadata.custom_fields`，按稳定字段 ID 对应。

旧 `version` 会在迁移时并入备注；旧 `engine` 和 APK 标记会迁入自定义字段，原始属性保留。移除字段定义或选项不会删除记录中的值。字段定义保存在本机 WebView localStorage，记录 JSON 只保存字段值。

密码显示开关仅控制管理卡片。JSON、备份、打包结果和日志可能包含明文密码。

## 导入、导出与备份

- 管理页首次挂载时尝试导入上次管理目录和打包输出目录中的旧 `pack_mapping.json`。其他目录可通过“导入旧 JSON”选择。
- 旧记录根据原 JSON 所在目录和文件名补齐路径及 ID；包含有效绝对 `archive_path` 的记录沿用该路径。
- 同路径已有集中记录优先，重复导入不会覆盖集中记录中的新备注。导入保留原 JSON，也不搬动压缩文件。
- “导出备份”另存完整记录，导出位置应在应用记录目录之外。导出不包含主题或自定义字段定义。
- 集中保存使用 SQLite 事务。首次迁移遇到无效旧 JSON 时会报错并保留原文件，不会用空记录覆盖它；迁移完成后数据库是正式记录。

恢复时，可将导出的 JSON 导入到空数据库；已有同路径记录会被保留。旧版留下的 `.bak` 是旧 JSON 的上一份快照，不是当前数据库的自动备份。需要从旧 JSON 重新迁移时，先退出 GUI 和 CLI，并单独保留现有数据库与原文件，再进行恢复。

## 只读脚本示例

PowerShell 可读取管理页“导出备份”生成的 JSON，只输出名称和哈希，不显示密码：

```powershell
$catalogPath = 'D:\Backups\pack_mapping_backup.json' # 改为实际导出路径
$catalog = Get-Content -LiteralPath $catalogPath -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($entry in $catalog.mappings) {
    Write-Output "$($entry.source_name) -> $($entry.archive_path)"
    foreach ($hash in $entry.output_hashes) {
        Write-Output "  $($hash.file_name): $($hash.sha256)"
    }
}
```

应用运行时应通过管理页修改记录，不要用外部脚本直接修改数据库。
