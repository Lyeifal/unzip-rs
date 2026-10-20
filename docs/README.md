# 项目文档

更新日期：2026-10-20。当前版本为 `v0.1.26102001`（Pre-release），需手动下载安装。正式压缩记录使用 AppData 中的 SQLite；旧 JSON 可迁移、导入和导出。

## 使用与数据

| 文档 | 内容 |
|---|---|
| [项目 README](../README.md) | 功能、安装构建、CLI、配置与常见问题 |
| [版本更新日志](../CHANGELOG.md) | 当前预发布版本、文件保护与 UI 修复、验证范围和发布附件 |
| [界面与设置](./UI_REDESIGN.md) | 主题、壁纸、文字对比度、卡片透明度、共用工具和管理字段 |
| [压缩记录格式](./PACK_MAPPING.md) | 字段、哈希密码匹配、导入导出、合并与恢复 |
| [数据存储与 SQLite 迁移](./STORAGE.md) | 配置、记录和 WebView 偏好的位置；SQLite 迁移、备份及体积取舍 |
| [MCP 与 Agent Skill](../integrations/README.md) | 独立 MCP 服务、Codex / Claude Code / OpenCode 接入、skill 与分发包 |
| [JSON 示例](./examples/pack_mapping_example.json) | 展示单文件、分卷和旧字段兼容的虚构数据；不对应实际压缩文件 |

## 开发与维护

| 文档 | 内容 |
|---|---|
| [AGENTS.md](./AGENTS.md) | 架构、核心行为、文件安全约定、开发命令与测试锚点 |
| [GitHub 更新与发布](./UPDATES.md) | 版本同步、Updater 签名、Windows 构建、MCP 附件、Pre-release 与稳定版发布 |
| [实现摘要](./IMPLEMENTATION_SUMMARY.md) | 当前模块、后端命令、保存流程及已有检查范围 |
| [整体审查记录](../REVIEW_2026-10-20.md) | 2026-10-20 的问题、修复、194 项 Rust 测试及 6 组界面回归 |
| [UI 变更记录](../CHANGELOG_UI_REDESIGN.md) | 当前 UI 修复摘要与早期历史重构记录 |

历史记录中的验证结果、体积数字和“未来计划”只对应当时阶段。当前行为以使用文档和实现为准；本次文档更新未重新运行功能测试。
