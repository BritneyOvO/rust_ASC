# HarmonyOS 移植记录

调查与集成日期：2026-11-26。第一阶段已经在 `integrate/harmonyos` 分支完成；本文保留调查基线、实现约定、提交映射和后续范围。

## 1. 仓库与固定基线

| 用途 | 本地目录 | 来源 | 当前修订 |
| --- | --- | --- | --- |
| 目标仓库基线 | `/root/rust_ASC` | `BritneyOvO/rust_ASC` | `0daddfb832b24bd0e43e76a9365eab4673db0cec` |
| Android 增强参考 | `/root/rust_ASC_ltlly` | `ltlly/rust_ASC`，`main` | `30c3fb7487a2f98d1aa3e8e62dddab2b5d45ef1a` |
| 鸿蒙实现来源 | `/root/ASC_rs` | `ReverseToolkit/ASC_rs`，本地 `main` | `bca5c2a62dee93c315067a40aa016c7c5a6ad156`，另有未提交修改 |

两份参考克隆均已初始化 `radguard` 子模块。目标仓库已配置 `ltlly` remote，并在 `integrate/harmonyos` 上以 `30c3fb7` 为 Android 增强基线；当前子模块固定为 `4e0c5ec803d46b51eecf3f6db7e12d1034a12f04`。

- BritneyOvO 基线的 `radguard`：`752dfd1106715db6df5f2746dcfea9b47bbb899e`。
- ltlly `main` 的 `radguard`：`4e0c5ec803d46b51eecf3f6db7e12d1034a12f04`，该提交已能从 MG1937 的子模块仓库取得。
- ltlly 语义分支：`feat/semantic-metadata-and-exact-unit`，比 `main` 多 `7a7875b`；其 `radguard` 为 `3ce0e9bb0090ae42e70eee561a624a91474f3363`，并切换子模块来源到 ltlly。

来源文件的哈希和工作区状态记录在 `harmony-port-sources.json`。集成前已重新比对：`ASC_rs` 仍停在 `bca5c2a`，跟踪 diff 与 5 个未跟踪 xref 文件的哈希均未变化。

## 2. 集成顺序与结果

实际从目标基线建立 `integrate/harmonyos` 分支，先接入 ltlly `main`，再增加鸿蒙提交：

```sh
git switch -c integrate/harmonyos origin/main
git merge --ff-only ltlly/main
git submodule sync --recursive
git submodule update --init --recursive
```

`origin/main` 是 `ltlly/main` 的祖先，差异为 0 / 6 个提交，因此保留了这 6 个提交的作者与历史：

1. `3b235cc`：直接 DEX 反编译优化与引擎更新。
2. `42cf043`：批量 getclass、JSON、导出过滤和 CLI/MCP 扩展。
3. `44cb039`：泛型签名与参数名恢复的引擎更新。
4. `900ebaf`：结构化输出与进程内 MCP。
5. `080d8a2`：worker 与 APK handler 的 clippy 调整。
6. `30c3fb7`：保留上游仓库 URL。

这些提交在 11 个文件中约增加 1,515 行、删除 205 行，包括子模块指针变动。鸿蒙移植不需要处理另一套 Android 实现的历史合并。

## 3. 移植范围

第一阶段已移植现有鸿蒙查询和反编译链路：

| 来源 | 移植内容 | 依赖边界 |
| --- | --- | --- |
| `crates/asc-abc` | PANDA 文件头、ABC 元数据、类与成员读取、基础查询 | 只依赖 regex / thiserror |
| `crates/asc-abc-dec` | 332 条 Ark 指令定义、反汇编、JavaScript 恢复、CFG、Oxc 优化，以及现有性能修复 | 依赖 asc-abc / Oxc / thiserror；高级引用搜索另有 regex |
| `crates/asc-hap` | HAP/HSP ZIP 读取、ABC 成员选择、module/config/pack.info、Harmony 资源解析 | 依赖两个 ABC crate 及压缩、mmap、CRC 库 |
| 旧 CLI 的鸿蒙分支 | 输入识别、source/asm、外部资源、命令适配 | 按目标 CLI 重写适配层 |
| 本地未提交的 `xref/` | 常量、整数、指令搜索，以及保守值流和调用关系 | 单独移植并修复已发现的回归 |

目标 Android 代码继续使用 `radguard` 和 ltlly 的批处理路径。旧 CLI 直接依赖旧 Android crate，不能整体复制进目标仓库。

HAP 文件比较、签名和 native 差异属于第二阶段，见第 9 节；它们位于 `asc-diff` / `asc-so`，不是三个鸿蒙核心 crate 的依赖。

## 4. Cargo 与模块边界

保留目标根 package、`src/asc_client`、`src/asc_core`、release profile 和二进制名 `rust_ASC`。为三个鸿蒙库添加 workspace：

```text
rust_ASC/
  Cargo.toml                 根 package + workspace
  radguard/                  独立 Git 子模块
  crates/
    asc-abc/
    asc-abc-dec/
    asc-hap/
  src/
    input.rs                 输入识别与 ArtifactKind
    harmony/
      mod.rs
      handler.rs             鸿蒙命令与批处理适配
      output.rs              结构化结果转换
    cli.rs                   现有 clap 参数和命令分派
    mcp/                     复用相同命令执行路径
```

- workspace 显式列出根 package 和三个 crate，`default-members = ["."]`、`resolver = "3"`；`radguard` 从 workspace 排除，保留其独立管理方式。
- 导入 crate 保留 edition 2021；根 package 保留 edition 2024。三个 Harmony crate 已用 Rust 1.88 验证；根二进制因 `apk-info-axml 1.0.13` / `apk-info-xml 1.0.13` 明确要求 Rust 1.93，所以根 package 的 MSRV 为 1.93。
- 统一三个导入 crate 的版本、许可、MSRV 和内部路径依赖。保持 Oxc `=0.75.0` 及当前必要 feature，首次移植不同时升级 AST 引擎。
- 使用目标现有的 flate2 `zlib-rs` 后端，避免同时从旧 manifest 引入另一套压缩 feature。依赖版本以现有 manifests / lockfile 为依据。
- 根 package 新增 `harmony` feature，关联三个可选路径依赖；接通 CLI 时将默认 feature 设为 `mcp + harmony`。只构建 Android 时可以关闭默认 feature。
- 增量更新目标 `Cargo.lock`；旧锁文件作为版本参考，不整份替换目标锁文件。
- 新增库继续使用各自的 thiserror 错误枚举；CLI 边界转换到目标既有错误返回方式，保留路径、ABC 成员、pc 和 opcode 信息。

依赖方向为根 package → asc-hap / asc-abc-dec → asc-abc。ABC 库不依赖 CLI、MCP 或 Android 后端。

## 5. 命令和结果约定

### 输入识别

先识别 `PANDA\0\0\0`，再依据 ZIP 中的鸿蒙标记判断 HAP/HSP。文件扩展名只作提示。`assets/.../config.json` 单独存在时不能将 APK 识别成 HAP；损坏的中央目录应报告读取失败。

在对类名进行 Java/Dalvik 规范化之前选择后端。鸿蒙 record 名可能带 `&entry/...&` 等编码，不能全部送进 Android 的类名转换函数。

### 对齐现有命令

| 命令 | 鸿蒙行为 |
| --- | --- |
| `getclass` | 单个或多个 ABC record；支持 classes-file、JSON、source/asm |
| `listclass` | 保留 ABC/HAP 顺序和包前缀规则；JSON 输出描述符数组 |
| `getmanifest` | HAP 的 module.json，缺失时 config.json，并包含 pack.info 与资源解析 |
| `export` | HAP/ABC 导出 `.js` 或 `.pa`；支持 prefix / classes-file 过滤 |
| `findrefs` | string / type / method / field；接通新查询引擎后增加 constant / integer / code |

新增 `--abc-output source|asm`，默认 source；`--resources` 用于裸 ABC 的资源目录或资源 HAP。HAP 使用自身资源。保留目标的 `--thread` 参数约定。

第一阶段 `listmethod` / `liststring` 仍提供其现有 Android 功能；对鸿蒙输入给出明确的能力错误。ABC 字符串来源与 DEX 全局字符串池不同，不能把引用搜索结果冒充完整字符串枚举。`extends` / `impl` 的鸿蒙实现也不由 DEX 语义直接推导。

### JSON、错误与文件输出

- Android 的既有 JSON 字段和行为保留。
- 鸿蒙 getclass 同样返回数组，每项包含 class、status、source，并增加 backend、language、ABC 成员来源等必要字段。ABC 成员不用 `dex` 字段表示。
- `status` 区分成功、未找到和解析/反编译失败。没有源码位置数据时省略 `member_references`，不能填造假的源码跨度。
- 鸿蒙 Manifest 的文本形式保留可读文件分隔；如果请求 JSON，则从原始文件构造对象，不解析已经加过分隔标题的文本。
- 引用 JSON 从结构化命中生成，包含成员来源、caller、pc、opcode、kind、resolution、target/value；文本输出由同一结构格式化，不再反向拆分文本行。
- 保留目标现有 `-o` 规则，并为新增鸿蒙路径测试相同的 stdout 行为；旧 ASC_rs 的规则不自动覆盖目标规则。
- 批量操作保留成功结果和每项失败状态；出现失败时 CLI 返回非零。MCP 返回错误状态时仍保留已经生成的批量结果。
- 不支持的指令返回有位置的错误。source 失败时允许用户显式改用 asm。

### 批量与缓存

每次请求复用中央目录、每个命中 ABC 的解压 buffer 和 Decompiler；筛选目标后再反编译。导出同样每个 ABC 解压一次。

跨请求缓存只存拥有所有权的 mmap / bytes 与元数据。借用 bytes 的 Decompiler / References 在请求作用域构造，避免把自引用对象放进缓存。缓存有数量与字节预算；完整值流图只在需要相应查询时构建，不在打开包时预建。

多 ABC 的导出保留成员来源命名空间，避免不同 ABC 中同名 record 相互覆盖。路径净化、中央目录重复成员、大小写冲突和 `.js` / `.pa` 后缀都需要确定性规则及回归检查。

## 6. 已落地的提交

每个功能提交携带对应回归测试；源码导入同时补齐了必要许可说明。前八项和一个 Android 解压修正已提交，最后一项在本文档、README 与 CI 验证完成后提交。

| 顺序 | 提交 | 改动与验收 |
| --- | --- | --- |
| 1 | `1be1ba8 feat(abc): import checked Ark metadata reader` | workspace 基础、asc-abc、PANDA/类名/成员/字面量读取与原有测试；保留来源修订和许可 |
| 2 | `86ce32c feat(abc): port native disassembly and JavaScript recovery` | asc-abc-dec、ISA、优化器和四份样本；反编译集成测试及优化器测试通过 |
| 3 | `b7e723f feat(hap): add Harmony archives and resource resolution` | ZIP64、损坏目录、解压限额、资源替换、Manifest 与导出行为 |
| 4 | `54cd49e feat(harmony): route CLI queries to HAP and ABC backends` | 输入识别、getclass/listclass/getmanifest、基础 findrefs、source/asm/resources、JSON |
| 5 | `011c26a feat(harmony): batch decompilation and filtered exports` | 多 record / classes-file、共享 ABC 会话、来源命名空间、过滤与部分失败 |
| 6 | `2b405a1 feat(abc): add instruction references and conservative value flow` | 常量、整数、指令、值流和调用关系；包含 normalized/相对模块路径及歧义解析回归 |
| 7 | `cf92245 feat(harmony): expose structured constant and code searches` | constant/integer/code CLI、结构化命中、资源解析及批量查询复用 |
| 8 | `0bde53a feat(mcp): expose Harmony analysis through shared CLI dispatch` | CLI/MCP 共用分派，新增 ABC 输出、资源参数与 `input_path` 别名 |
| 修正 | `80ac390 fix(apk): initialize buffers before libdeflate decompression` | 修复 Rust 1.93 对未初始化解压 buffer 的拒绝，同时保持 Android 回归通过 |
| 9 | `test(harmony): verify feature matrix and document provenance` | CI feature 矩阵、README、来源记录、脱离 checkout 的 Harmony 二进制回归和最终验证 |

MCP 保留 `apk_path` 参数兼容性，并可以提供 `input_path` 的输入别名；说明改为对应支持的 APK/HAP/ABC 输入。CLI 与 MCP 不能各维护一份输入识别或查询逻辑。

## 7. 调查时问题与修复结果

调查阶段执行的基线检查：

- ltlly `main`：`cargo test --locked` 通过，共 95 项测试；包含 MCP、批量 getclass、DEX 041、Manifest/引用和源码元数据回归。现有编译警告已记录，未顺手修改。
- 当前本地 asc-abc：2 项单元测试和 13 项集成测试通过。
- 当前本地 asc-hap：10 项单元测试和 2 项集成测试通过。
- 当前本地 asc-abc-dec：17 项反编译集成测试通过；库测试 27 项通过、1 项失败。

当时唯一失败项是 `xref::tests::modern_literal_buffers_and_cross_record_calls_are_searchable`。能找到 AppLog 内部调用，但未得到 GamePage 对 AppLog 的已解析调用。

根因确认是样本导入路径 `@normalized:N&&&entry/src/main/ets/common/AppLog&` 被旧 `record()` 按首个 `&` 截断。当前实现保留 `&&&` 后的真实路径，并继续对缺失模块和同名模块保持保守解析。原失败测试与新增的 normalized/相对路径/歧义测试均已通过。

旧来源存在 6 个已跟踪文件的未提交修改，外加 `xref/` 下 5 个未跟踪文件。稳定移植以 bca5c2a 的 Git 内容为准；第 6、7 个提交单独吸收工作区增强。移植准备不在旧仓库执行 reset、stash 或自动提交。

## 8. 最终验证

最终执行：

```sh
cargo clippy --workspace --all-targets --all-features --locked
cargo +1.88.0 check -p asc-abc -p asc-abc-dec -p asc-hap --locked
cargo test --workspace --all-features --locked
cargo test -p rust_ASC --no-default-features --locked
cargo test -p rust_ASC --no-default-features --features harmony --locked
cargo test -p rust_ASC --no-default-features --features mcp --locked
cargo build --release --locked
```

只对本轮改动文件执行 rustfmt，避免给既有 Android 代码或 `radguard` 子模块制造整库格式 diff。根包已有 15 项 warning 由 Android 基线继承；本轮 Harmony 文件不新增 warning。

覆盖以下实际边界：

- APK 与 HAP 输入识别，含无扩展名、误导扩展名和 APK assets/config.json。
- 裸 ABC、HAP/HSP、多 ABC、store/deflate、ZIP64 和损坏输入。
- Ark 13/24 样本、所有指令定义、异常边界、闭包、异步和生成器。
- 裸 ABC 的外部资源、HAP 默认资源、非 ASCII 和资源转义。
- 单类/多类、缺失目标、无匹配过滤、重复 record、路径冲突、部分失败。
- 常量、负整数、十六进制/大整数、指令查询、跨 record 导入与调用、闭包作用域。
- CLI 与 MCP 的结果、输出文件和错误一致；协议 stdout 不混入日志。
- 二进制复制到空目录后仍可完成鸿蒙查询，运行时不依赖 Node 或旧仓库路径。

Node 仅用作可选测试解释器；CI 可安装 Node 执行行为回归，产品运行无需 Node。保留既有 Windows/Linux/macOS 和 GNU/musl 发布矩阵，验证导入 Oxc 后的目标兼容性。

## 9. 后续独立阶段

### ltlly 的语义元数据分支

`7a7875b` 增加精确 DEX 单元定位、嵌套条目、输入/工具修订、变量与符号元数据、重命名和渲染选项，10 个文件约增加 944 行、删除 124 行。

该分支已拉取但本轮未检出、未编译验证。建议先在独立分支验证并保留它的原始提交，再与鸿蒙主线集成。特别检查：

- radguard 的 ltlly 来源与固定 SHA 能从全新 checkout 取得。
- `source_unit` 是 DEX 专用模型；鸿蒙 origin 使用独立类型或带 backend 的明确区分。
- ABC 的 pc 不等于生成源码位置；没有实际映射前不宣称支持变量重命名或语义编辑。
- JSON 与批量结果接口的共同改动在独立适配提交中解决。

### Windows 静态 CRT

ltlly 的 `ci/windows-static-crt` 是验证用途的分支，整体替换了发布 workflow，删去了 Linux/macOS 和发布步骤。后续若需要静态 CRT，只单独移植 Windows 编译选项与检查，保留完整矩阵。

### HAP diff / 签名 / native

旧 asc-diff 依赖 asc-apk、asc-axml 和 asc-so。HAP 相关内容包括文件分组、HAP 签名块证书、文件导出，以及 libs 下 ELF 比较。

这部分单独拆为两组提交：先接入独立 asc-so 的 ELF/反汇编能力，再移植 HAP 的目录比较和签名适配。资源和 XML 转换通过明确的后端接口处理，避免为了 HAP diff 拉入旧 Android 反编译链。沿用 SHA、X.509 与 ELF 的现有测试并补充目标 CLI/MCP 适配。

第一阶段已达到完成标准：目标基于 ltlly 的稳定增强，具备 HAP/ABC 查询、JavaScript/Ark 汇编、资源与批量导出；高级引用搜索的已知回归已修复；CLI/MCP 共用实现，功能矩阵通过且来源可追溯。
