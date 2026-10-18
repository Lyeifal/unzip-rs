# unzip-rs（Rust + Tauri 版）

自动解压 + 打包压缩工具（早期仅用脚本快速验证过原型，现行为以本文档与代码为准）。核心能力：魔数识别伪装包、散卷归集、改名卷领养、密码库策略、lz4 含 legacy 链式解码、apk 产物落位、按外层目录分组，捆绑应急解压层；扩展能力：多形体/垃圾前缀/复合包识别与雕出（封面图+rar、完整 mp4 后追加 zip/分卷、大媒体尾部追加档带结构校验）、SFX 自解压包识别（PE overlay 判定，与游戏启动 exe 区分）、数字卷名尾标与夸克分段容忍、同名嵌套目录折叠、同 stem 双格式去重、内层解包用密码库、内层档藏子目录两层内自动解、内层档同名目录冲突自动化解（暂存位）。

## 结构

当前用户文档入口见 [README.md](./README.md)，集中记录与 JSON 兼容格式见 [PACK_MAPPING.md](./PACK_MAPPING.md)，保存位置与 SQLite 迁移见 [STORAGE.md](./STORAGE.md)。当前正式记录采用 SQLite；历史测试数量不能作为新阶段的验证结果。

记录模块 `archive_hash` 负责流式 SHA-256，`archive_catalog` 负责 SQLite 记录、旧 JSON 迁移、元数据合并和密码索引；它们与 `pack`、`pipeline` 共享同一份记录，不依赖 GUI。

- `crates/unzip-core` — 纯逻辑库（无 UI 依赖）；模块：config / sniff / scan / extract / assemble / pipeline / lz4mini / types / pack（打包压缩）。关键新增函数：`sniff::embedded_kind`（嵌入识别）/`embedded_offset`（雕出偏移）/`embedded_tail_offset`（尾部窗+结构校验）、`sniff::sniff_sfx`（SFX 自解压包判定）、`sniff::match_num_vol_tail`（卷名尾标与夸克形态）、`pipeline::carve_to_temp`（复合包雕出）、`pipeline::collapse_same_name_dir`（同名折叠）、`pipeline::collect_level`/`run_level`（解包作业归组/执行，UNWRAP_WALK_MAX=2 浅扫子目录、`.unwrap_` 暂存位化解同名冲突）、`pipeline::dedup_key`（同 stem 去重）、`assemble::concat_volumes`（夸克分段拼接兜底）。
- `crates/unzip-cli` — 命令行（含 --dry-run）
- `app/` — Tauri v2 + Vue 3 桌面应用（独立 cargo 工程，`src-tauri/Cargo.toml` 带空 `[workspace]` 与根 workspace 隔离）
- `assets/bundled/` — 应急层：7z.exe+7z.dll（7-Zip 26.03, LGPL）、UnRAR.exe（RARLAB, 仅解压许可）、License.txt、NOTICE.txt

## 发布组伪装手法识别速查

| 手法 | 识别机制 | 解压路径 | 测试锚点 |
|---|---|---|---|
| 纯改后缀（rar/zip 魔数在偏移 0 + 假扩展名） | 文件头魔数 | 直接解 | `sniff_all_archive_magics` |
| 真封面图 + 追加 rar（多形体 jpg，如 PC15732.jpg） | 前缀 64MiB 窗找 rar 8 字节魔数 | 7z 原生支持前缀偏移，直接解 | `polyglot_cover_image_unwrapped` |
| 完整 mp4 + 追加 zip/7z/xz（如 二小姐.mp4=56MiB 可播视频 + 733MB zip(1.exe) + 尾部小档；game.mp4 内装加密 zip 套 7z 分卷；荒野独居 151/152=同一 875MB 视频头 + 数 GB zip 拖残破 EOCD 到文件尾） | zip 走尾部 EOCD 反推起点（**允许 ≤16MiB 拖尾**，起点须 PK 验证）；EOCD 被故意损坏时兜底走**本地头链识别**（沿 PK\x03\x04 链跟随，命中中央目录或连续 2 环即坐实；实测发布组为防网盘扫描损毁 EOCD，荒野独居 151/152 由此解开）；7z/xz 走前缀窗，miss 再走尾部 64MiB 窗 | 按嵌入偏移**雕出**纯压缩档再解（7z 不做 SFX 校正） | `mp4_zip_with_trailing_data_extracted`、`zip_eocd_allows_small_trailing_junk`、`mp4_prefixed_zip_with_inner_7z_volumes_full_chain`、`zip_broken_eocd_found_by_lfh_chain` |
| 大媒体后追加 rar/7z/xz 到文件尾（超过前缀窗） | 尾部 64MiB 窗找长魔数，**必须过结构校验**（rar4/5 头类型+头长、7z StartHeaderCRC32、xz 流标志 CRC32；实测 12.6GB 真视频尾部有巧合 rar4 魔数，不校验会误移入失败目录） | 按偏移雕出（rar 前缀命中仍走原生不雕） | `tail_magic_validates_archive_structure`、`tail_magic_validates_7z_and_xz_headers` |
| 垃圾前缀 + rar/7z/xz | 同上嵌入扫描 | 7z/xz 雕出；rar 原生 | `sniff_polyglot_cover_image_with_appended_archive` |
| 数字卷名带尾标（`x.7z.001删`/`002旧`） | `match_num_vol_tail` 容忍 1-4 字节非 ASCII 尾标 | 正常分卷归集 | `renamed_first_volume_with_marker_reunited` |
| 夸克分卷名（`x.7z-<hex>.001`） | 同上，hex 6-16 位形态 | 正常分卷归集 | `parse_volume_name_rename_marks` |
| 夸克按段下载单档（任意字节切 x.7z.001/.002…） | 分卷归集后 7z 按多卷打不开 → **拼接兜底** | `concat_volumes` 按序拼成单档 | `quark_byte_segmented_single_archive_concat_fallback` |
| 压缩包内再嵌规范分卷（zip 里装 x.7z.001~00N） | `collect_level` 按 (族,stem) 归组 | 只解首卷，7z/unrar 原生多卷拼接，成功整族删除 | 同上全链路用例 |
| 包内顶层目录与包同名（out/X/X） | `collapse_same_name_dir` | 内容上移一层 | `nested_same_name_top_dir_collapsed` |
| SFX 自解压包（PE overlay 带压缩档，如 1.exe 内装 7z 分卷，双击解压型） | PE 节表算 overlay，起点/后 1MiB 窗内须带校验通过的压缩档结构（7z StartHeaderCRC/rar 头/zip 本地头）；游戏启动 exe 无此结构，不误伤 | 识别为普通压缩档，7z 原生读 SFX，不雕出；加密 SFX 走密码库自动尝试（真实案例命中） | `sfx_exe_with_archive_overlay_detected`、`sfx_exe_full_chain_unwrapped` |
| 内层档藏在包内单层/两层子目录（fcm 真实结构：zip→png(实为zip)→153…/{1.zip,存档.rar}） | 本层无可解对象时 `unwrap_folder` 浅扫子目录两层（UNWRAP_WALK_MAX=2），解出后重扫 | 解到所在子目录；**第 3 层以下不碰**（游戏自带存档备份/Mod 压缩包通常在更深层，解了就是过度解压） | `nested_archives_in_shallow_subdirs_unwrapped_but_deep_ones_untouched` |
| 内层档与档内顶层目录同名（B3 真实结构：zip 里装无扩展名档 B3，其内含 B3/ 目录） | 解内层档前先挪进 `.unwrap_` 暂存位腾名，成功随 TempDir 删除、失败挪回告警 | 化解 7z「当文件已存在时，无法创建该文件」假成功 | `inner_same_name_dir_collision_unwrapped_via_staging` |
| 同 stem 双格式（a.zip + a.rar） | `dedup_key` 去重，只解第一个 | 跳过的记 skipped | `same_stem_dual_format_extracted_once` |
| 下载中半成品（`.qkdownloading`） | DOWNLOADING_SUFFIXES 跳过 | 不碰，源文件不动 | `pdf_is_ordinary_file_and_qkdownloading_skipped` |
| 随包广告 PDF | `.pdf` 入黑名单 →「普通文件」 | 不告警（伪装 pdf 仍走魔数识别，**不排除**） | `pdf_named_disguised_archive_still_extracted` |

已知缺口：SFX 的压缩档结构落在 overlay 1MiB 窗之外认不出（stub 超大）；本地头链兜底只覆盖 zip（rar/7z 无等价廉价结构验证）；尾部窗外（>64MiB）的超大追加档不参与；zip 后拖着的小 rar/7z 尾巴随 zip 雕出时被带进临时档、不参与单独解（内容本体在 zip 里，可接受）；gzip/bzip2/lz4/zstd 魔数太短不参与嵌入扫描；`.txt/.md/.log` 等文档后缀与 `.exe/.msi` 不启用嵌入识别（内容可能真含魔数字符串，误判会移入失败目录）。

## 常用命令

- Windows 桌面入口在 debug/release 均使用 `windows_subsystem=windows`，不能仅 release 启用；右键常注册到开发版，控制台子系统会在进入 main/单实例转发前闪出终端。7z/UnRAR/lz4 子进程继续使用 `CREATE_NO_WINDOW`。只有用户显式开启 `shell_show_console` 时，`task_console` 通过 `AllocConsole` 创建应用日志终端，结束后保留至退出应用或下次右键任务关闭该选项；禁用终端关闭菜单，防止关闭终端直接终止任务。验证终端闪现时不得给桌面 exe 启动器额外加 `CREATE_NO_WINDOW`，否则会掩盖入口错误。

- 测试：`cargo test --workspace`（集成测试用真实 7z/UnRAR/lz4，外部工具缺失时对应用例自动 skip）
- CLI 解压：`cargo run --release -p unzip-cli -- <目录> [-o 输出] [--dry-run]`
- CLI 打包：`cargo run --release -p unzip-cli -- pack <文件/目录…> -o 输出 [--name G198] [--step 1] [--format 7z|zip] [--random-uniform | --password 密码] [--volume-threshold 3 --volume-size 2] [--no-txt] [--txt-template "文案 {date}"] [--mx 5] [--dry-run]`
- GUI 开发：`cd app && npm install && npm run tauri dev`（devUrl 端口 1520，1420 被本机 Hyper-V 保留）
- GUI exe 重建（改了 unzip-core 后）：`cd app && npx tauri build --no-bundle`（只出 exe）；**勿裸 `cargo build --release`**——不经 tauri CLI 前端资源不嵌入，打开即「无法访问此页面」。便携版 exe 需与 `resources\` 同拷。
- 打包：`cd app && sync-bundled.bat && npm run tauri build`（NSIS 安装包；应急组件在 `app/src-tauri/resources/bundled/`，由 sync-bundled.bat 从 `assets/bundled/` 同步——resources 必须位于 src-tauri 内，写 `../` 相对路径会被 NSIS 编成 `_up_` 段装到安装目录之外）

## 关键约定

- **config 位置**：CLI 默认 exe 旁 `config.json`；Tauri 启动时设 `UNZIP_CONFIG_PATH` 指向应用数据目录（`%APPDATA%\unzip-rs\config.json`）。
- **名称与迁移**：产品名/默认安装目录为 `unzip-rs`；`appDirectoriesOverride` 将配置与记录定向到 `%APPDATA%\unzip-rs`，WebView 到 `%LOCALAPPDATA%\unzip-rs\webview`。窗口 `create=false`，setup 完成 `storage::migrate_directory` 后再创建窗口，避免 WebView 提前生成目标文件遮蔽旧偏好。迁移先完整复制到临时目录，再原子发布；旧文件保留，新目录非空时不合并旧文件，SQLite 必须包含 WAL 快照。`installer-hooks.nsh` 在产品改名时识别并卸载旧程序文件，保留用户数据。单实例标识及更新公钥不变。标题栏 `titlebarBackground` 默认 false，图标可用 `python app/scripts/make_icon.py` 生成透明底源图后用 Tauri icon 生成各平台尺寸。
- **共用工具设置**：`ToolPathsSettings.vue` 位于应用设置中，显式保存 `seven_zip/unrar/lz4`，调用 `save_tool_paths` 校验并只合并这三项；解压页不再编辑工具路径。任务启动等待进行中的路径保存，两种任务均从配置文件读取最新工具。解压页 `save_config` 只保存其余配置，保留已保存的共用路径；返回 false 时不得继续启动任务。配置写入和合并由 core `update_config_to` 在同进程 writer mutex 内串行完成，自动密码置顶只合并密码，不用旧任务配置覆盖新设置。测试锚点：`password_promotion_preserves_settings_saved_after_task_started`、`simultaneous_config_updates_do_not_lose_changes_or_share_temp_writes`、Tauri `tool_settings::tests`、`app/tests/tool-path-settings.mjs`。
- **捆绑工具回退链**：用户配置的外部 7z/UnRAR 路径优先；不存在时用 `UNZIP_BUNDLED_DIR`（或 exe 旁 `bundled/`）里的内置组件。lz4 解码内置 Rust 实现（`lz4mini`）优先，外部 lz4.exe 仅回退。
- **用户可见文案**（日志/警告/错误）保持既定措辞；新增文案（如"同组同名包已处理，疑似重复格式，已跳过"、"同名目录折叠失败"）属有意扩展，改语义时必须同步更新本文件的语义清单。
- **文件安全**：源目录文件只读（失败移目录是显式设计）；测试只允许写 tempfile 目录；批量改名/删除逻辑改动后必须跑集成测试。
- 卷名/相似度匹配/去重键/折叠判定一律在 ASCII 折叠后的字节上进行（中文等多字节文件名绝不允许按 Unicode lower() 后的字符串索引切片——曾因此 panic）。
- 嵌入识别后缀规则：`.exe/.msi/.dll/.sys` 与文档脚本类（.txt/.md/.nfo/.log/.ini/.json/.url/.html/.htm/.bat/.cmd/.py）不启用嵌入扫描（内容可能真含魔数字符串，误判会被移入失败目录）；但 PE 文件（MZ）另走 SFX 判定（sniff_sfx）：overlay 带校验通过的压缩档结构即自解压包，识别为压缩档。**`.pdf` 必须启用**（伪装成 pdf 的压缩包要识别），真 PDF 靠黑名单按「普通文件」跳过。
- 跳过路径（领养跳过/去重跳过）也必须推进 `on_progress`，否则 GUI 进度条永远到不了 total。
- 雕出/拼接产生的临时目录用 TempDir 持有、函数出口 drop 清理；失败包的目标目录删除后源文件才移入失败目录。

## 行为语义（设计决策）

（各手法的快速索引见上文「发布组伪装手法识别速查」表，本清单是完整语义。）

- 扫描两遍化（scan.rs）：第一遍只收分卷，第二遍主卷补位——`stem.rar` 是否算卷族首卷不依赖目录枚举顺序（确定性）。
- 改名卷候选排序用 jaro_winkler，只影响尝试顺序。
- rar 加密头探测空密码显式 `-p` 防交互卡死。
- 多形体/垃圾前缀/尾部追加识别（sniff.rs embedded_kind）：文件头无魔数但媒体头或未知内容时，前缀 64MiB 窗扫 rar/7z/xz 长魔数（≥6 字节；gzip/bzip2/lz4/zstd 魔数太短不参与），miss 再扫尾部 64MiB 窗（完整媒体后「追加到文件尾」）；尾部窗命中必须过结构校验（rar4/5 头类型+头长、7z StartHeaderCRC32、xz 流标志 CRC32，公式对着真实档案实测）——视频数据里会出现巧合魔数（实测 12.6GB 真 mp4 尾部有假 rar4 魔数），不校验会被误移入失败目录；前缀窗维持纯魔数判定（历史行为）。zip 走尾部 EOCD 反推起点：从文件尾倒序找 PK\x05\x06，允许 ≤16MiB 拖尾（三连体：zip 后还拖着小 rar/7z），eocd_pos − cd_offset − cd_size 反推的起点必须真是 zip 本地头。`.exe/.msi/.dll/.sys` 与文档脚本类后缀（.txt/.md/.log/.json/.bat…内容可能真含魔数字符串）不启用嵌入扫描——但 PE（MZ）另走 SFX 判定（见速查表）；`.pdf` 启用（伪装 pdf 必须识别）。scan_sources 对分类结果按路径备忘，两遍扫描只嗅探一次（嵌入扫描有大窗口 I/O）。
- 复合包雕出（pipeline.rs carve_to_temp）：识别 kind 与文件头魔数不符（mp4/图片前缀或媒体后追加 zip/7z/xz/rar）时，按嵌入偏移雕出纯压缩档到临时目录再解——7z 对带前缀的 zip/7z/xz 不做 SFX 偏移校正（实测打不开）；rar 前缀命中原生支持不雕，尾部命中必须雕（原生工具扫不到文件尾）。zip 偏移由 EOCD 反推，7z/xz 取前/尾部窗首个校验通过的命中。SFX 例外：sniff_head 已按 overlay 结构判出 kind，视为正常压缩档不雕出，7z 原生读（含密码库自动尝试）。发布组典型结构「mp4 头 + 追加 zip（内装 7z.001~00N 分卷）」「完整 mp4 + 追加 zip(1.exe) + 尾部小档（二小姐.mp4）」「mp4 + 875MB 后接 1.66GB zip（荒野独居 153）」由此全链路解开。
- ZIP64 复合包（sniff.rs）：存在 ZIP64 locator 时，即使经典 EOCD 的 32 位大小/偏移未达到哨兵值，也从 ZIP64 EOCD 与 locator 反推起点，验证单盘、记录边界和目录偏移一致，再检查起点魔数；ZIP64 可扩展记录搜索上限 1MiB。经典字段是哨兵值但无有效 locator 时不走普通 EOCD 公式。本地头链可解析 0x0001 扩展中的 64 位压缩大小，拒绝截断和溢出。真实案例 `13.公主协同效应/二小姐.mp4`：59524992 字节视频前缀 + ZIP64(1.exe) + 尾部小档；旧版误跳过为真实媒体，实际未处理游戏程序。测试锚点：`embedded_zip64_with_trailer_and_optional_sentinels`、`zip64_locator_checks_records_offsets_and_64_bit_values`、`zip64_local_extra_supports_chain_recovery_and_rejects_truncation`、`embedded_zip64_extracts_and_preserves_game_program`。
- 数字卷名容忍（sniff.rs match_num_vol_tail）：`x.7z.###`/`x.zip.###` 后允许 1-4 字节非 ASCII 尾标（「删」「旧」——下载器/手改痕迹，如 `深渊迷宮.7z.001删`）；`.7z-<hex 6-16>.###` 夸克分卷形态。archive_stem/parse_volume_name 共用。
- 夸克分段拼接兜底（assemble.rs concat_volumes）：文件名像分卷但切点在任意字节（夸克按段下载单档）时，规范名归集后 7z 按多卷打不开 → 按 idx 序拼接成单档再探（1..=max 必须连续无缺卷）；失败原因仍报「分卷组装后校验未通过（可能缺卷或卷内容不对）」。判定依据：真 7z 多卷首卷的 NextHeaderOffset 指向全集末尾（实测 2.95GB 集声明 2950182584），与拼接总长比对即知缺多少尾数据。
- 解包分卷族归组（unwrap_folder::collect_level）：目录内规范命名分卷（x.7z.001+N 等）按 (族,stem) 归组，首卷在场即只解首卷（7z/unrar 原生多卷自动拼接），成功后整族删除；缺首卷的散卷保持原样。
- 同名嵌套目录折叠（pipeline.rs collapse_same_name_dir）：包内单一顶层目录名等于包名（或包名去压缩后缀，ASCII 折叠）时内容上移一层，避免 out/X/X（有意设计，非默认行为）；extract 成功后与 unwrap 每轮递归后各执行一次；上移中途失败回滚并记警告。产物（apk 落位）不走 unique_dir，避免与同组同名压缩档输出目录互挤。
- 密码合并（extract_package）：规则+库+运行时密码合成一份 pw_list，lz4 链、正式解压、内层解包共用一份——内层加密档不再缺密码。
- 同组同名去重（run() dedup_key）：键 = 组+包名+首卷子路径（ASCII 折叠；不同子目录同名包是不同版本，产物不参与），a.zip + a.rar 这类同 stem 双格式只解第一个（按路径排序，首个成功后才登记、失败则允许后续同名格式兜底），跳过的记 skipped 并推进 on_progress，源文件不动。alive_count 按去重键计数（幸存包落位 out/<组> 而非三层嵌套）。被去重跳过的卷族不做预组装（不领养孤儿、不白烧组装子进程；首个失败时主循环内联组装兜底）。
- unwrap_folder 目录混入照常解包：dest 含子目录、或非压缩包杂项文件（readme/产物/广告图等）时不放弃内层解包——真实发布组结构「加密 game.7z + 全CG存档/」混装很常见，杂项不阻断。测试锚点：`inner_archive_alongside_dirs_and_stray_files_still_unwrapped`。
- unwrap_folder 浅扫子目录两层：本层无可解对象时扫子目录两层（UNWRAP_WALK_MAX）找内层档并解到所在子目录，解出后重扫（fcm 真实结构「zip→png(实为zip)→153…/{1.zip,存档.rar}」由此解开）；第 3 层以下不碰——游戏自带的存档备份/Mod 压缩包通常在更深层（实测第 5 层），解了就是过度解压。测试锚点：`nested_archives_in_shallow_subdirs_unwrapped_but_deep_ones_untouched`。
- 解内层档先挪 `.unwrap_` 暂存位：腾出「档名同名目录」的名字再解，成功随 TempDir 删除、失败挪回并告警——B3 真实结构（档 B3 内含 B3/ 目录）旧逻辑 7z 建目录报「当文件已存在时，无法创建该文件」，内层失败但外层假成功；暂存位化解。测试锚点：`inner_same_name_dir_collision_unwrapped_via_staging`。
- unwrap_folder 无进展不递归：一轮解包（含浅扫）无一成功即停，失败的压缩包留在原地——避免同密码逐层重试造成子进程风暴。
- `.pdf` 入 ORPHAN_BLACKLIST（随包广告 PDF 按「普通文件」跳过，不产生「无法识别」告警）；`.qkdownloading`（夸克下载中临时文件）入 DOWNLOADING_SUFFIXES（有魔数也不许碰，防止半成品被移入失败目录、打断正在进行的下载）。

## 打包压缩（pack.rs）

- 入口 `run_pack(sources, opts, cfg, cb)`：逐源一个包；核心 `name_mode=legacy` 仍兼容 CLI 的 `name_start` 末尾数字递增。桌面端固定传 `name_mode=parts`，不显示旧起始包名及命名方式选项；旧 localStorage 配置加载时迁移前缀与数字起点。parts 使用可独立启停的前缀、纯数字序号和后缀，宽度保留且溢出加宽；`auto_number` 从 SQLite 中跨目录匹配相同前后缀的 `.7z/.zip/.rar` 包名（含数字卷尾），从最大序号 +step 与用户起点中较大者开始，预览和实际打包共用解析路径，实际开始前重新计算。记录不可读时明确报错；输出撞名仍自动 " (2)" 避让，绝不覆盖旧包。
- 打包走 `7z a`（Extractor 的 7z 探测与回退链与解压共用）；产物路径必须绝对（7z 的 current_dir 是 temp 暂存目录）。
- 资源管理器快捷压缩入口 `run_quick_pack`：每个选中项在其父目录创建一个包，文件使用 stem（`文件.txt` → `文件.7z`），文件夹保留完整名称。`Config.quick_pack_format` 默认 `7z`，设置可选 `zip`，`set_quick_pack_format` 校验后只合并该字段；`save_unpack_config` 保留磁盘最新值，避免旧快照覆盖。快捷压缩独立构造选项，`password_mode=none`、不混入 txt、不分卷，沿用集中记录和撞名避让，不修改常规打包表单或 localStorage 偏好。测试：core `quick_pack_*`、Tauri `quick_pack_format_defaults_validates_and_survives_stale_unpack_save`、`app/tests/shell-integration.mjs`。
- 加密：7z 格式 `-p密码 -mhe=on`（连文件名一起加密），zip 格式 `-p密码 -mem=AES256`；密码模式 random_per_pack / random_uniform / manual，随机密码 16 位字母数字（剔除 0/O、1/I/l 易混淆字符）。
- 分卷：源内容总大小（压缩后不可预知）> 阈值（默认 3G）→ `-v{大小}g`（默认 2g）。**单文件分卷优化**：启用分卷参数但压缩后只产生一个文件时，自动重命名去除 `.001` 后缀（Test001.7z.001 → Test001.7z），避免假分卷误导。
- 混淆 txt：输入先硬链接镜像（`link_or_copy`，不占磁盘）到 temp 暂存目录，混入 `资源说明.txt`（模板替换 `{date}` + 8 位随机 hex），cd 进暂存目录打包 `.`——txt 落在压缩包根，同批每包内容唯一 → hash 必然不同（防网盘按 hash 比对），下载者解压可见来源与日期。
- **集中映射记录**（`archive_catalog`）：每个条目打包后立即 upsert 到 `%APPDATA%\unzip-rs\pack_mapping.sqlite3`（CLI 可用 `UNZIP_CATALOG_PATH` 指定旧 JSON 路径及数据库基名，GUI 启动时指定 AppData 路径）；`record_id` 基于压缩文件绝对路径，同名不同目录不合并。首次使用时在事务中导入旧 JSON 并保留原文件；无效 JSON 不得用空数据库遮蔽。元数据命令只修改指定条目，不接受前端整文件快照覆盖。集中保存失败时，`write_mapping_json` 仅作为输出目录恢复副本兜底并告警。
- **压缩内容哈希**（`archive_hash`）：单文件分卷完成改名后，逐产物流式计算 SHA-256，`output_hashes` 保存每个文件的 file_name / size_bytes / sha256。按块检查取消，不读取整个大文件入内存；哈希失败或取消仍保留产物与密码记录，并提示哈希不完整。
- **哈希密码策略**：`Config.hash_match` 默认 true。`HashPasswords` 从集中记录按大小和 SHA-256 建索引，只有大小有候选时才读文件，包内同一路径缓存结果。优先密码同时用于预组装和正式解包，后接规则/密码库/运行时密码；无效记录或未命中不妨碍原流程。分卷仅以首卷查找密码，不能视为整组完整性验证。关闭或 dry-run 不读输入哈希，产物不比对。
- CLI 无子命令时仍是解压；`pack` 为子命令（目录恰好叫 pack 时会被优先当子命令解析，已知小歧义）。
- 取消在包间及哈希读取块之间检查 `should_cancel`，不杀 7z 子进程。已完成的压缩产物会保留；哈希取消导致记录不完整时须告警。
- GUI：`App.vue` 为 Tab 壳（KeepAlive 三页状态互不丢），解压页 `components/UnpackView.vue`、打包页 `components/PackView.vue`、管理页 `components/LibraryView.vue`；公共样式入口是 `styles/themes.css`、`styles/glassmorphism.css` 和 `styles/animations.css`。Tauri 事件解压 `uz-*`、打包 `pack-*`（前缀隔离不串扰）。
- 更新：`useAppUpdater.ts` 在发布构建启动后检查 GitHub 稳定版，只提示；`UpdateSettings.vue` 提供手动检查及下载安装。`begin_update_install` 与压缩/解压共用 busy，安装失败时只释放更新自身占用。正式构建合并 `tauri.release.conf.json` 生成签名；`prepare-release.mjs` 生成 Windows x64 清单，`verify_update` 使用 minisign 验证包及签名版本，工作流创建 Release 草稿。密钥位于仓库外，禁止提交私钥或任意替换现有公钥；详见 [UPDATES.md](./UPDATES.md)。

## 通用压缩包管理与资源管理器入口

- 单实例插件必须最先注册。直接右键动作由 `is_direct_request` 识别，每次从磁盘加载显示偏好：`shell_show_progress` 默认 true（旧配置也默认开启），`shell_show_console` 默认 false；任一开启时 show/unminimize/focus，并取消后台自动退出标志，完成后保留结果。两者均关闭时冷启动隐藏，热启动只转发，不抢焦点。普通启动或添加到列表始终显示应用。`set_shell_display` 只合并这两个字段，`save_unpack_config` 必须保留磁盘最新值，不能被旧解压快照覆盖。`TaskActivity.vue` 显示当前文件、准备状态和实际已用时间，现有外部工具返回前不得虚构包内百分比。App.vue 协调两页队列和 active 状态，队列排空后调用 `finish_shell_requests`，后端再次确认 busy 与待分发队列均空才退出静默后台实例；失败/警告/启动错误仍显示窗口，保留结果和密码重试。用户正常打开的实例不自动退出。`sibling_output` 复用非链接空目录，仍对含内容目录或文件碰撞递增避让，各文件以独占创建方式避免覆盖。测试：Tauri `shell_display_defaults_persist_without_stale_unpack_overwrite`、`app/tests/shell-integration.mjs`；原生测试须覆盖默认显示、保留完成结果、可选终端及下一次右键任务关闭终端。

- `LibraryView.vue` 直接展示集中打包记录，`GameCard.vue` 为保留旧文件名的通用卡片。备注与 `metadata.custom_fields` 保存在集中 SQLite 数据库，字段定义及密码可见性由 `useLibraryFields.ts` 存在本机 localStorage。密码默认隐藏，隐藏时不得将明文放入 DOM 或 title；该设置不加密数据库或 JSON 导出。旧 `version` 只在 `schema_version < 1` 时并入备注，引擎/APK 标记迁入稳定字段 ID，原始属性保留；移除定义不删除字段值。
- 管理页使用 `query_archive_library` 从 SQLite 加载 24 条一页的记录，数据库负责搜索、字段筛选、排序、匹配计数与全库统计；切换页码或条件时旧请求结果不得覆盖新请求。JSON 备份仍走完整 `load`/`export`，不受分页截断。
- 字段类型为文本、下拉、勾选。字段 ID 不随名称改变；保存队列捕获条目 ID 和元数据快照，只调用 `update_archive_metadata`。旧目录 JSON 自动尝试导入已知的管理/打包目录，也提供手动导入和备份导出；同路径已有集中记录优先，原文件不修改。切回管理页会刷新集中记录。
- Windows 右键菜单由 `shell_integration.rs` 按用户设置写入 HKCU `Software\Classes\*\shell\UnzipRs` / `Directory\shell\UnzipRs`，`MUIVerb=unzip-rs`、空 `SubCommands` 与内部 `shell` 构成二级菜单。启停只清理本应用新键和旧 `UnzipRs.Pack` / `UnzipRs.Unpack`。五个入口：`--shell-extract-here` / `--shell-extract-folder` 直接解压，`--shell-pack-same-name` 直接快捷压缩，`--shell-unpack` / `--shell-pack` 仅加入列表。直接 exe + 引号参数，不经 cmd/PowerShell。单实例转发与冷启动队列交给 App.vue 分发，UnpackView / PackView 注册事件后自动运行；忙时保留排队，停止清空相应页面待执行直接任务。传统 `MultiSelectModel=Player` 最多 100 项，Win11 通常在“显示更多选项”。
- `start_shell_run` 从磁盘读取已保存 Config，共用密码策略、hash 匹配、failed_dir_enabled 与 busy 互斥，不保存临时源/输出目录。核心 `run_with_layout` 保留默认 Grouped 流程，Here / NamedFolder 在每个包的父目录内暂存，复用完整解压及递归流程，成功后 `sibling_output` 原子预留不重名目录/文件，拒绝覆盖与符号链接；落位异常保留剩余暂存内容并告警。直接模式按实际父路径区分同名包。`PendingRetry` 保留原始输出布局与目录，失败包移入 failed 后重试也回原始目标，且只尝试新密码。测试：`explorer_*`、`sibling_output::tests`、注册表临时键迁移测试、`app/tests/shell-integration.mjs`。
- `start_run` 的可选 `files` 非空时仅用指定文件/目录作为源根，空数组报错；未传时保留源目录扫描。核心 `scan_sources` 对显式文件只扫描所选文件，不枚举父目录；同目录选中文件合入同一两遍扫描组。显式文件可位于输出目录内，重复根只处理一次。测试锚点：`explicit_files_do_not_scan_unselected_neighbors_and_use_parent_root`、`explicit_main_rar_and_companion_are_grouped_in_both_orders`。
