# 打包映射文件说明

## 概述

从本版本开始，批量压缩功能会在输出目录自动生成 `pack_mapping.json` 文件，记录源文件/目录与生成的压缩包之间的对应关系。

## 文件位置

- 文件名：`pack_mapping.json`
- 位置：与压缩包相同的输出目录

## 文件结构

```json
{
  "generated_at": "2026-10-14",           // 生成日期（YYYY-MM-DD）
  "total_count": 3,                       // 成功打包的总数
  "mappings": [                           // 映射列表
    {
      "source_name": "游戏本体",          // 源文件/目录名称
      "archive_name": "G001.7z",          // 压缩包名称（单卷）
      "password": "aB3dEfGh5jKmNpQr",     // 密码（null 表示无密码）
      "size_bytes": 524288000,            // 源内容字节数
      "size_human": "500.0 MB",           // 人类可读大小
      "is_volume": false,                 // 是否分卷
      "volume_count": 1                   // 分卷数（单卷为 1）
    },
    {
      "source_name": "DLC合集",
      "archive_name": "G002.7z.001",      // 分卷时显示首卷完整文件名
      "password": "xY9zAbCdEfGhJkMn",
      "size_bytes": 3221225472,
      "size_human": "3.0 GB",
      "is_volume": true,                  // 分卷标识
      "volume_count": 2                   // 该包共 2 个分卷
    },
    {
      "source_name": "MOD整合包",
      "archive_name": "G003.zip",
      "password": null,                   // 无密码
      "size_bytes": 157286400,
      "size_human": "150.0 MB",
      "is_volume": false,
      "volume_count": 1
    }
  ]
}
```

## 字段说明

### 顶层字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `generated_at` | string | 生成日期，格式 YYYY-MM-DD |
| `total_count` | number | 成功打包的压缩包总数 |
| `mappings` | array | 映射关系数组 |

### mappings 数组元素

| 字段 | 类型 | 说明 |
|------|------|------|
| `source_name` | string | 源文件或目录的名称 |
| `archive_name` | string | 生成的压缩包名称。单卷时为 `G001.7z`；分卷时为首卷完整文件名 `G002.7z.001` |
| `password` | string \| null | 压缩包密码。`null` 表示无密码 |
| `size_bytes` | number | 源内容的总字节数（压缩前） |
| `size_human` | string | 人类可读的大小格式（如 "3.2 GB"、"500.0 MB"） |
| `is_volume` | boolean | 是否为分卷压缩包 |
| `volume_count` | number | 分卷数量。单卷为 1，多卷为实际分卷数 |

## 使用场景

1. **批量管理**：快速查找某个源文件对应的压缩包名称和密码
2. **自动化脚本**：解析 JSON 文件进行后续处理（上传、重命名等）
3. **档案管理**：记录打包历史，方便日后查询
4. **密码管理**：集中存储随机生成的密码，避免丢失

## 示例用法

### Python 读取示例

```python
import json

with open('pack_mapping.json', 'r', encoding='utf-8') as f:
    data = json.load(f)

# 查找特定源文件对应的压缩包
source = "游戏本体"
for mapping in data['mappings']:
    if mapping['source_name'] == source:
        print(f"压缩包：{mapping['archive_name']}")
        print(f"密码：{mapping['password']}")
        print(f"大小：{mapping['size_human']}")
        break
```

### PowerShell 读取示例

```powershell
$json = Get-Content pack_mapping.json -Raw | ConvertFrom-Json

# 列出所有映射
foreach ($mapping in $json.mappings) {
    Write-Host "$($mapping.source_name) -> $($mapping.archive_name)"
    if ($mapping.password) {
        Write-Host "  密码: $($mapping.password)"
    }
}
```

## 注意事项

1. 只有成功打包的项目会出现在映射文件中，失败的不会记录
2. 如果没有任何成功的打包，不会生成映射文件
3. 映射文件生成失败不会影响打包流程，只会产生警告
4. 每次打包会覆盖旧的 `pack_mapping.json` 文件
5. 分卷时 `archive_name` 字段显示首卷完整文件名（如 `G002.7z.001`），其他分卷按序递增（`G002.7z.002`、`G002.7z.003`...）

## 完整示例

参见项目 `docs/examples/pack_mapping_example.json` 文件。
