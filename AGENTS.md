# 自动解压工具（Rust + Tauri 版）

原 Python 版的 Rust 重写（Python 版已退役删除）。功能对齐 Python 版（魔数识别伪装包、散卷归集、改名卷领养、密码库策略、lz4 含 legacy 链式解码、apk 产物落位、按外层目录分组），新增捆绑应急解压层；另新增 Python 版没有的能力：多形体/垃圾前缀/复合包识别与雕出（封面图+rar、完整 mp4 后追加 zip/分卷、大媒体尾部追加档带结构校验）、SFX 自解压包识别（PE overlay 判定，与游戏启动 exe 区分）、数字卷名尾标与夸克分段容忍、同名嵌套目录折叠、同 stem 双格式去重、内层解包用密码库、内层档藏子目录两层内自动解、内层档同名目录冲突自动化解（暂存位）。

## 结构

- `crates/unzip-core` — 纯逻辑库（无 UI 依赖），解压行为语义逐条对齐原 Python 版 unzip_core；模块：config / sniff / scan / extract / assemble / pipeline / lz4mini / types / pack（打包压缩，新增能力，无 Python 对应）。关键新增函数：`sniff::embedded_kind`（嵌入识别）/`embedded_offset`（雕出偏移）/`embedded_tail_offset`（尾部窗+结构校验）、`sniff::sniff_sfx`（SFX 自解压包判定）、`sniff::match_num_vol_tail`（卷名尾标与夸克形态）、`pipeline::carve_to_temp`（复合包雕出）、`pipeline::collapse_same_name_dir`（同名折叠）、`pipeline::collect_level`/`run_level`（解包作业归组/执行，UNWRAP_WALK_MAX=2 浅扫子目录、`.unwrap_` 暂存位化解同名冲突）、`pipeline::dedup_key`（同 stem 去重）、`assemble::concat_volumes`（夸克分段拼接兜底）。
- `crates/unzip-cli` — 命令行（对齐原 auto_unzip，含 --dry-run）
- `app/` — Tauri v2 + Vue 3 桌面应用（独立 cargo 工程，`src-tauri/Cargo.toml` 带空 `[workspace]` 与根 workspace 隔离）
- `assets/bundled/` — 应急层：7z.exe+7z.dll（7-Zip 26.03, LGPL）、UnRAR.exe（RARLAB, 仅解压许可）、License.txt、NOTICE.txt

## 发布组伪装手法识别速查

| 手法 | 识别机制 | 解压路径 | 测试锚点 |
|---|---|---|---|
| 纯改后缀（rar/zip 魔数在偏移 0 + 假扩展名） | 文件头魔数（与 Python 同） | 直接解 | `sniff_all_archive_magics` |
| 真封面图 + 追加 rar（多形体 jpg，如 PC15732.jpg） | 前缀 64MiB 窗找 rar 8 字节魔数 | 7z 原生支持前缀偏移，直接解 | `polyglot_cover_image_unwrapped` |
| 完整 mp4 + 追加 zip/7z/xz（如 二小姐.mp4=56MiB 可播视频 + 733MB zip(1.exe) + 尾部小档；game.mp4 内装加密 zip 套 7z 分卷） | zip 走尾部 EOCD 反推起点（**允许 ≤16MiB 拖尾**，起点须 PK 验证）；7z/xz 走前缀窗，miss 再走尾部 64MiB 窗 | 按嵌入偏移**雕出**纯压缩档再解（7z 不做 SFX 校正） | `mp4_zip_with_trailing_data_extracted`、`zip_eocd_allows_small_trailing_junk`、`mp4_prefixed_zip_with_inner_7z_volumes_full_chain` |
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

已知缺口：SFX 的压缩档结构落在 overlay 1MiB 窗之外认不出（stub 超大）；zip 后拖尾 >16MiB 的 EOCD 认不出；尾部窗外（>64MiB）的超大追加档不参与；zip 后拖着的小 rar/7z 尾巴随 zip 雕出时被带进临时档、不参与单独解（内容本体在 zip 里，可接受）；gzip/bzip2/lz4/zstd 魔数太短不参与嵌入扫描；`.txt/.md/.log` 等文档后缀与 `.exe/.msi` 不启用嵌入识别（内容可能真含魔数字符串，误判会移入失败目录）。

## 常用命令

- 测试：`cargo test --workspace`（集成测试用真实 7z/UnRAR/lz4，外部工具缺失时对应用例自动 skip）
- CLI 解压：`cargo run --release -p unzip-cli -- <目录> [-o 输出] [--dry-run]`
- CLI 打包：`cargo run --release -p unzip-cli -- pack <文件/目录…> -o 输出 [--name G198] [--step 1] [--format 7z|zip] [--random-uniform | --password 密码] [--volume-threshold 3 --volume-size 2] [--no-txt] [--txt-template "文案 {date}"] [--mx 5] [--dry-run]`
- GUI 开发：`cd app && npm install && npm run tauri dev`（devUrl 端口 1520，1420 被本机 Hyper-V 保留）
- GUI exe 重建（改了 unzip-core 后）：`cd app && npx tauri build --no-bundle`（只出 exe）；**勿裸 `cargo build --release`**——不经 tauri CLI 前端资源不嵌入，打开即「无法访问此页面」。便携版 exe 需与 `resources\` 同拷。
- 打包：`cd app && sync-bundled.bat && npm run tauri build`（NSIS 安装包；应急组件在 `app/src-tauri/resources/bundled/`，由 sync-bundled.bat 从 `assets/bundled/` 同步——resources 必须位于 src-tauri 内，写 `../` 相对路径会被 NSIS 编成 `_up_` 段装到安装目录之外）

## 关键约定

- **config 位置**：CLI 默认 exe 旁 `config.json`；Tauri 启动时设 `UNZIP_CONFIG_PATH` 指向应用数据目录（`%APPDATA%\com.unzip.app\config.json`）。
- **捆绑工具回退链**：用户配置的外部 7z/UnRAR 路径优先；不存在时用 `UNZIP_BUNDLED_DIR`（或 exe 旁 `bundled/`）里的内置组件。lz4 解码内置 Rust 实现（`lz4mini`）优先，外部 lz4.exe 仅回退。
- **用户可见文案**（日志/警告/错误）与 Python 版逐字一致；新增文案（如"同组同名包已处理，疑似重复格式，已跳过"、"同名目录折叠失败"）属有意扩展，改语义时必须同步对照 Python 版与本文件差异清单。
- **文件安全**：源目录文件只读（失败移目录是显式设计）；测试只允许写 tempfile 目录；批量改名/删除逻辑改动后必须跑集成测试。
- 卷名/相似度匹配/去重键/折叠判定一律在 ASCII 折叠后的字节上进行（中文等多字节文件名绝不允许按 Unicode lower() 后的字符串索引切片——曾因此 panic）。
- 嵌入识别后缀规则：`.exe/.msi/.dll/.sys` 与文档脚本类（.txt/.md/.nfo/.log/.ini/.json/.url/.html/.htm/.bat/.cmd/.py）不启用嵌入扫描（内容可能真含魔数字符串，误判会被移入失败目录）；但 PE 文件（MZ）另走 SFX 判定（sniff_sfx）：overlay 带校验通过的压缩档结构即自解压包，识别为压缩档。**`.pdf` 必须启用**（伪装成 pdf 的压缩包要识别），真 PDF 靠黑名单按「普通文件」跳过。
- 跳过路径（领养跳过/去重跳过）也必须推进 `on_progress`，否则 GUI 进度条永远到不了 total。
- 雕出/拼接产生的临时目录用 TempDir 持有、函数出口 drop 清理；失败包的目标目录删除后源文件才移入失败目录。

## 已知与 Python 版的差异（有意）

（各手法的快速索引见上文「发布组伪装手法识别速查」表，本清单是完整语义。）

- 扫描两遍化：`stem.rar` 是否算卷族首卷不再依赖目录枚举顺序（确定性，见 scan.rs）。
- lz4 解码顺序：内置优先（Python 是 pip lz4 → 内置 → exe）。
- 改名卷候选排序用 jaro_winkler（Python 是 difflib ratio），只影响尝试顺序。
- rar 加密头探测空密码显式 `-p` 防交互卡死（语义同 Python）。
- 多形体/垃圾前缀/尾部追加识别（sniff.rs embedded_kind）：文件头无魔数但媒体头或未知内容时，前缀 64MiB 窗扫 rar/7z/xz 长魔数（≥6 字节；gzip/bzip2/lz4/zstd 魔数太短不参与），miss 再扫尾部 64MiB 窗（完整媒体后「追加到文件尾」）；尾部窗命中必须过结构校验（rar4/5 头类型+头长、7z StartHeaderCRC32、xz 流标志 CRC32，公式对着真实档案实测）——视频数据里会出现巧合魔数（实测 12.6GB 真 mp4 尾部有假 rar4 魔数），不校验会被误移入失败目录；前缀窗维持纯魔数判定（历史行为）。zip 走尾部 EOCD 反推起点：从文件尾倒序找 PK\x05\x06，允许 ≤16MiB 拖尾（三连体：zip 后还拖着小 rar/7z），eocd_pos − cd_offset − cd_size 反推的起点必须真是 zip 本地头。`.exe/.msi/.dll/.sys` 与文档脚本类后缀（.txt/.md/.log/.json/.bat…内容可能真含魔数字符串）不启用嵌入扫描——但 PE（MZ）另走 SFX 判定（见速查表）；`.pdf` 启用（伪装 pdf 必须识别）。Python 版只认偏移 0 魔数。scan_sources 对分类结果按路径备忘，两遍扫描只嗅探一次（嵌入扫描有大窗口 I/O）。
- 复合包雕出（pipeline.rs carve_to_temp）：识别 kind 与文件头魔数不符（mp4/图片前缀或媒体后追加 zip/7z/xz/rar）时，按嵌入偏移雕出纯压缩档到临时目录再解——7z 对带前缀的 zip/7z/xz 不做 SFX 偏移校正（实测打不开）；rar 前缀命中原生支持不雕，尾部命中必须雕（原生工具扫不到文件尾）。zip 偏移由 EOCD 反推，7z/xz 取前/尾部窗首个校验通过的命中。SFX 例外：sniff_head 已按 overlay 结构判出 kind，视为正常压缩档不雕出，7z 原生读（含密码库自动尝试）。发布组典型结构「mp4 头 + 追加 zip（内装 7z.001~00N 分卷）」「完整 mp4 + 追加 zip(1.exe) + 尾部小档（二小姐.mp4）」「mp4 + 875MB 后接 1.66GB zip（荒野独居 153）」由此全链路解开。
- 数字卷名容忍（sniff.rs match_num_vol_tail）：`x.7z.###`/`x.zip.###` 后允许 1-4 字节非 ASCII 尾标（「删」「旧」——下载器/手改痕迹，如 `深渊迷宮.7z.001删`）；`.7z-<hex 6-16>.###` 夸克分卷形态。archive_stem/parse_volume_name 共用。
- 夸克分段拼接兜底（assemble.rs concat_volumes）：文件名像分卷但切点在任意字节（夸克按段下载单档）时，规范名归集后 7z 按多卷打不开 → 按 idx 序拼接成单档再探（1..=max 必须连续无缺卷）；失败原因仍报「分卷组装后校验未通过（可能缺卷或卷内容不对）」。判定依据：真 7z 多卷首卷的 NextHeaderOffset 指向全集末尾（实测 2.95GB 集声明 2950182584），与拼接总长比对即知缺多少尾数据。
- 解包分卷族归组（unwrap_folder::collect_level）：目录内规范命名分卷（x.7z.001+N 等）按 (族,stem) 归组，首卷在场即只解首卷（7z/unrar 原生多卷自动拼接），成功后整族删除；缺首卷的散卷保持原样。
- 同名嵌套目录折叠（pipeline.rs collapse_same_name_dir）：包内单一顶层目录名等于包名（或包名去压缩后缀，ASCII 折叠）时内容上移一层，避免 out/X/X；extract 成功后与 unwrap 每轮递归后各执行一次；上移中途失败回滚并记警告。产物（apk 落位）不走 unique_dir，避免与同组同名压缩档输出目录互挤。
- 密码合并（extract_package）：规则+库+运行时密码合成一份 pw_list，lz4 链、正式解压、内层解包共用（Python 内层只看运行时密码，GUI 下内层加密档必失败）。
- 同组同名去重（run() dedup_key）：键 = 组+包名+首卷子路径（ASCII 折叠；不同子目录同名包是不同版本，产物不参与），a.zip + a.rar 这类同 stem 双格式只解第一个（按路径排序，首个成功后才登记、失败则允许后续同名格式兜底），跳过的记 skipped 并推进 on_progress，源文件不动。alive_count 按去重键计数（幸存包落位 out/<组> 而非三层嵌套）。被去重跳过的卷族不做预组装（不领养孤儿、不白烧组装子进程；首个失败时主循环内联组装兜底）。Python 两份都解。
- unwrap_folder 目录混入照常解包：Python 版在 dest 含子目录、或非压缩包杂项文件（readme/产物/广告图等）时直接放弃内层解包（`if dirs or len(archives)!=len(files)`），真实发布组结构「加密 game.7z + 全CG存档/」会因此把游戏包残留在输出目录（用户实测踩坑）；Rust 版只要有可解的内层压缩档/分卷族就继续解，杂项不阻断。测试锚点：`inner_archive_alongside_dirs_and_stray_files_still_unwrapped`。
- unwrap_folder 浅扫子目录两层：本层无可解对象时扫子目录两层（UNWRAP_WALK_MAX）找内层档并解到所在子目录，解出后重扫（fcm 真实结构「zip→png(实为zip)→153…/{1.zip,存档.rar}」由此解开）；第 3 层以下不碰——游戏自带的存档备份/Mod 压缩包通常在更深层（实测第 5 层），解了就是过度解压。测试锚点：`nested_archives_in_shallow_subdirs_unwrapped_but_deep_ones_untouched`。
- 解内层档先挪 `.unwrap_` 暂存位：腾出「档名同名目录」的名字再解，成功随 TempDir 删除、失败挪回并告警——B3 真实结构（档 B3 内含 B3/ 目录）旧逻辑 7z 建目录报「当文件已存在时，无法创建该文件」，内层失败但外层假成功；暂存位化解。测试锚点：`inner_same_name_dir_collision_unwrapped_via_staging`。
- unwrap_folder 无进展不递归：一轮解包（含浅扫）无一成功即停，失败的压缩包留在原地，不同密码逐层重试（Python 会按 max_depth 反复重试同一批失败档）。
- `.pdf` 入 ORPHAN_BLACKLIST（随包广告 PDF 按「普通文件」跳过，不产生「无法识别」告警）；`.qkdownloading`（夸克下载中临时文件）入 DOWNLOADING_SUFFIXES（有魔数也不许碰，防止半成品被移入失败目录、打断正在进行的下载）。

## 打包压缩（pack.rs，新增能力，无 Python 对应）

- 入口 `run_pack(sources, opts, cfg, cb)`：逐源一个包；命名从 `name_start` 起末尾数字 +step 递增（宽度保留，G198→G199，Z999→Z1000）；输出撞名自动 " (2)" 避让，绝不覆盖旧包。
- 打包走 `7z a`（Extractor 的 7z 探测与回退链与解压共用）；产物路径必须绝对（7z 的 current_dir 是 temp 暂存目录）。
- 加密：7z 格式 `-p密码 -mhe=on`（连文件名一起加密），zip 格式 `-p密码 -mem=AES256`；密码模式 random_per_pack / random_uniform / manual，随机密码 16 位字母数字（剔除 0/O、1/I/l 易混淆字符）。
- 分卷：源内容总大小（压缩后不可预知）> 阈值（默认 3G）→ `-v{大小}g`（默认 2g）。
- 混淆 txt：输入先硬链接镜像（`link_or_copy`，不占磁盘）到 temp 暂存目录，混入 `资源说明.txt`（模板替换 `{date}` + 8 位随机 hex），cd 进暂存目录打包 `.`——txt 落在压缩包根，同批每包内容唯一 → hash 必然不同（防网盘按 hash 比对），下载者解压可见来源与日期。
- CLI 无子命令时仍是解压；`pack` 为子命令（目录恰好叫 pack 时会被优先当子命令解析，已知小歧义）。
- 取消粒度为包间检查 `should_cancel`（与解压侧对齐，不杀 7z 子进程）。
- GUI：`App.vue` 为 Tab 壳（KeepAlive 两页状态互不丢），解压页 `components/UnpackView.vue`、打包页 `components/PackView.vue`，公共样式 `styles.css`；Tauri 事件解压 `uz-*`、打包 `pack-*`（前缀隔离不串扰）。
