# 打包映射文件说明

## 概述

从本版本开始，批量压缩功能会在输出目录自动生成 `pack_mapping.json` 文件，记录源文件/目录与生成的压缩包之间的对应关系。

**新特性：**
- ✨ **增量更新**：重复打包会自动合并，按压缩包名去重
- 📊 **详细统计**：记录压缩前后大小（bytes/MB/GB）和压缩率
- 💾 **配置记忆**：GUI 自动记住上次使用的输出目录、密码和模板文案

## 文件位置

- 文件名：`pack_mapping.json`
- 位置：与压缩包相同的输出目录
- 更新方式：增量合并（按 archive_name 去重）

## 文件结构

```json
{
  "generated_at": "2026-10-14",           // 最后更新日期
  "total_count": 3,                       // 总记录数（含历史）
  "mappings": [                           // 映射列表（按包名排序）
    {
      "source_name": "游戏本体",          // 源文件/目录名称
      "archive_name": "G001.7z",          // 压缩包名称
      "password": "aB3dEfGh5jKmNpQr",     // 密码（null 表示无密码）
      
      "source_size": {                    // 压缩前大小
        "bytes": 524288000,
        "mb": "500.00",
        "gb": "0.488",
        "human": "500.0 MB"
      },
      
      "compressed_size": {                // 压缩后大小
        "bytes": 456123456,
        "mb": "435.07",
        "gb": "0.425",
        "human": "435.1 MB"
      },
      
      "compression_ratio": "87.0%",       // 压缩率
      "is_volume": false,                 // 是否分卷
      "volume_count": 1,                  // 分卷数
      "created_at": "2026-10-14"          // 创建日期
    }
  ]
}
```

## 字段说明

### 顶层字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `generated_at` | string | 最后更新日期，格式 YYYY-MM-DD |
| `total_count` | number | 总记录数（含历史打包） |
| `mappings` | array | 映射关系数组（按 archive_name 排序） |

### mappings 数组元素

| 字段 | 类型 | 说明 |
|------|------|------|
| `source_name` | string | 源文件或目录的名称 |
| `archive_name` | string | 生成的压缩包名称。单卷时为 `G001.7z`；分卷时为首卷完整文件名 `G002.7z.001` |
| `password` | string \| null | 压缩包密码。`null` 表示无密码 |
| `source_size` | object | 压缩前大小（bytes / mb / gb / human） |
| `compressed_size` | object | 压缩后大小（所有分卷总和） |
| `compression_ratio` | string | 压缩率（如 "87.0%"） |
| `is_volume` | boolean | 是否为分卷压缩包 |
| `volume_count` | number | 分卷数量。单卷为 1，多卷为实际分卷数 |
| `created_at` | string | 创建/更新日期 |

## 增量更新机制

**自动去重：**
- 同一输出目录多次打包会自动合并记录
- 按 `archive_name` 去重：同名压缩包自动覆盖旧记录
- 不同压缩包累积保存，形成完整历史

**使用场景：**
```bash
# 第一批打包
打包 → G001.7z, G002.7z
结果：pack_mapping.json 包含 2 条记录

# 第二批打包（同一输出目录）
打包 → G003.7z, G004.7z
结果：pack_mapping.json 包含 4 条记录（累积）

# 重新打包 G001（同名覆盖）
打包 → G001.7z
结果：pack_mapping.json 包含 4 条记录（G001 信息已更新）
```

## 使用场景

1. **批量管理**：快速查找某个源文件对应的压缩包名称和密码
2. **压缩分析**：查看压缩率，评估哪些文件适合压缩
3. **自动化脚本**：解析 JSON 文件进行后续处理（上传、重命名等）
4. **档案管理**：记录打包历史，方便日后查询
5. **密码管理**：集中存储随机生成的密码，避免丢失
6. **增量备份**：同一目录多次打包自动累积记录

## GUI 配置记忆

打包页会自动记住上次成功打包时的配置：
- ✅ 输出目录
- ✅ 压缩密码（统一密码模式）
- ✅ 模板文案
- ✅ 其他所有打包选项

配置存储在浏览器 localStorage，重启应用后自动恢复。

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
        print(f"压缩前：{mapping['source_size']['human']}")
        print(f"压缩后：{mapping['compressed_size']['human']}")
        print(f"压缩率：{mapping['compression_ratio']}")
        break

# 分析压缩效果
for m in data['mappings']:
    ratio = float(m['compression_ratio'].rstrip('%'))
    if ratio < 50:
        print(f"{m['archive_name']} 压缩效果显著（{m['compression_ratio']}）")
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

1. **增量更新**：同一输出目录重复打包会合并记录，不会覆盖整个文件
2. **去重规则**：按 `archive_name` 去重，同名压缩包会更新记录
3. **压缩后大小**：为所有分卷文件的总和（多卷时需全部下载完才能解压）
4. **生成时机**：只有成功打包的项目会记录，失败的不会写入
5. **映射文件丢失**：下次打包会创建新文件，历史记录无法恢复
6. **分卷识别**：`archive_name` 显示首卷完整文件名（如 `G002.7z.001`），其他分卷按序递增
7. **压缩率异常**：压缩后比原文件大（>100%）是正常现象，常见于已压缩的文件（图片、视频、已压缩包）
8. **配置记忆**：GUI 的配置记忆存储在浏览器，清除浏览器数据会丢失

## 完整示例

参见项目 `docs/examples/pack_mapping_example.json` 文件。
