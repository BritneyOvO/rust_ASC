# rust_ASC

**rust_ASC is a stateless Android APK/DEX and HarmonyOS HAP/ABC analyzer with fast cross-reference search and native decompilation, written in Rust.** It queries compiled artifacts directly instead of building a global index, so there is no preprocessing step or persistent disk cache.

The Android backend is the Rust implementation of [MG1937/ASC](https://github.com/MG1937/ASC), the Python reference project, and preserves the same source layout, data structures, control flow and runtime behavior. The HarmonyOS backend is adapted from [ReverseToolkit/ASC_rs](https://github.com/ReverseToolkit/ASC_rs) and uses a native Rust Ark disassembler and JavaScript recovery pipeline adapted from [asmjmp0/xpanda](https://github.com/asmjmp0/xpanda). See [NOTICE](NOTICE) for fixed source revisions and attribution.

Presented at Black Hat Europe Arsenal: [Droid ASC: R8 Compiler Optimization as a DeCompiler Primitive](https://blackhat.com/europe/arsenal/schedule/index.html#droid-asc-r8-compiler-optimization-as-a-decompiler-primitive-54834).

Repository links:

- Python reference implementation: [MG1937/ASC](https://github.com/MG1937/ASC)
- Rust implementation: [MG1937/rust_ASC](https://github.com/MG1937/rust_ASC)
- Full-APK pseudocode exporter: [MG1937/radguard](https://github.com/MG1937/radguard)
- HarmonyOS implementation source: [ReverseToolkit/ASC_rs](https://github.com/ReverseToolkit/ASC_rs)
- Ark decompiler source: [asmjmp0/xpanda](https://github.com/asmjmp0/xpanda)

https://github.com/user-attachments/assets/f1f71a75-1eca-4647-89ce-406ed2c63c6b

## Features

- Search code references for strings, types, methods, and fields across every DEX in an APK.
- Read bare Ark `.abc` files and HarmonyOS HAP/HSP archives, including resources and module configuration.
- Recover JavaScript or Ark assembly for one or many Harmony records.
- Search Ark constants, signed integers, resolved instructions, calls, fields, types, and strings with bytecode locations.
- Locate and decompile one or many Android classes directly from an APK.
- List classes and defined Android methods, and decode Android or Harmony manifests.
- Export Android pseudocode through [MG1937/radguard](https://github.com/MG1937/radguard), or export selected Harmony records as `.js`/`.pa` files.
- Handle multi-DEX APKs and DEX 041 logical containers.
- Use memory-mapped APK access and no on-disk cache.
- Expose CLI queries as MCP tools over stdio.

## Why it is fast

- Stateless by design: no preprocessing, no global index build, no disk cache.
- APK and HAP archives are memory-mapped, and only the DEX or ABC members a query needs are inflated.
- Cross-member scans run in parallel, while each parsed DEX or ABC session is reused within the operation.
- R8 output is already highly structured. Searches jump to candidate offsets directly instead of rebuilding a bloated cross-reference database.
- On a hit, rust_ASC decompiles the class directly from the containing DEX without rebuilding it. It falls back to rebuilding a minimal self-consistent DEX in memory only when direct decompilation fails for that class; `--debug` logs which path was taken.
- Harmony queries preserve Ark bytecode locations and resolve imports conservatively, without inventing generated-source offsets.

## Build

`radguard` is a Git submodule. Clone it with the main repository:

```bash
git clone --recurse-submodules https://github.com/MG1937/rust_ASC.git
cd rust_ASC
cargo build --release
```

The `rust_ASC` binary requires Rust 1.93 because its Android manifest dependency declares that MSRV. The standalone Harmony crates (`asc-abc`, `asc-abc-dec`, and `asc-hap`) retain Rust 1.88 support.

For an existing checkout:

```bash
git submodule update --init --recursive
cargo build --release
```

MCP and HarmonyOS support are enabled by default. Build the Android-only CLI with:

```bash
cargo build --release --no-default-features
```

Build the CLI with HarmonyOS support but without MCP with:

```bash
cargo build --release --no-default-features --features harmony
```

The executable is written to:

```text
target/release/rust_ASC
```

On Windows, the executable name is `rust_ASC.exe`.

## Command Line Usage

Thread control is consistent across `getclass`, `listclass`, `listmethod`, `liststring`, `findrefs`, and `export`:

- Use only `--thread <N>`.
- Put `--thread` immediately after the subcommand.
- If omitted, the command uses the number returned by Rust's `available_parallelism()` API.

```bash
# Print version.
rust_ASC -V

# Search every code reference to a string (with optional truncation or match omission).
rust_ASC findrefs --thread 10 --debug app.apk string https://
rust_ASC findrefs app.apk string https:// --max-match 80
rust_ASC findrefs app.apk string https:// --no-match
rust_ASC findrefs --json app.apk string https://

# Search a type (accepts dotted or Dalvik form), method, or field.
rust_ASC findrefs --thread 10 app.apk type com.example.Main
rust_ASC findrefs --thread 10 app.apk method onCreate --class com.example.Main
rust_ASC findrefs --thread 10 app.apk field apiKey --class com.example.Main

# Search subclasses or interface implementors.
rust_ASC findrefs --thread 10 app.apk extends android.app.Activity
rust_ASC findrefs --thread 10 app.apk impl android.os.IInterface

# Treat --class as a fuzzy descriptor substring instead of an exact name.
rust_ASC findrefs --thread 10 app.apk method notify --class poc --fuzzy-class

# Decompile one or multiple classes from an APK, or emit structured JSON with member references.
rust_ASC getclass --thread 10 --debug app.apk com.example.Main
rust_ASC getclass --thread 10 app.apk com.example.Main com.example.Second
rust_ASC getclass --thread 10 --classes-file classes.txt app.apk
rust_ASC getclass --json --classes-file classes.txt app.apk -o classes.json

# List all classes, optionally filtered by package/class prefix or as JSON.
rust_ASC listclass --thread 10 app.apk
rust_ASC listclass --thread 10 --prefix com.example app.apk
rust_ASC listclass --json app.apk

# List method definitions without decompiling, optionally filtered by class/name or as JSON.
rust_ASC listmethod --thread 10 app.apk
rust_ASC listmethod --class com.example.Main -f onCreate app.apk
rust_ASC listmethod --json app.apk -o methods.json

# List string pool values, optionally filtered by substring, deduplicated (-u/--unique), or as JSON.
rust_ASC liststring --thread 10 app.apk
rust_ASC liststring -u -f https app.apk
rust_ASC liststring --json -u app.apk -o strings.json

# Decode AndroidManifest.xml.
rust_ASC getmanifest app.apk -o AndroidManifest.xml

# Export pseudocode for all or filtered classes to the required output directory.
rust_ASC export --thread 10 --debug app.apk output
rust_ASC export --prefix com.example app.apk output
rust_ASC export --classes-file classes.txt app.apk output

# Detect HAP/HSP/ABC by content and recover Harmony records as JavaScript or assembly.
rust_ASC listclass app.hap
rust_ASC getclass app.hap '&entry/src/main/ets/pages/Index&' --abc-output source
rust_ASC getclass modules.abc '&entry/src/main/ets/pages/Index&' --abc-output asm
rust_ASC getclass modules.abc '&entry/src/main/ets/pages/Index&' --resources resources/

# Search structured Ark constants and instructions.
rust_ASC findrefs app.hap constant 'https://.*' --json
rust_ASC findrefs app.hap integer -0x1 --json
rust_ASC findrefs modules.abc code 'lda\.str' --json

# Decode Harmony module/config metadata and export selected records.
rust_ASC getmanifest app.hap --json
rust_ASC export --abc-output source --prefix entry/src app.hap output
```

Input type is detected from the PANDA magic or ZIP contents rather than the filename extension. `getclass` accepts one or more class/record names and/or `--classes-file <file>` (one name per line, `#` comments allowed). Android JSON includes `dex` and `member_references`; Harmony JSON includes `backend`, `language`, and the source ABC `member`. A batch keeps successful output when another requested item fails and returns a nonzero status for the partial failure.

`findrefs` takes `string`, `type`, `extends`, `impl`, `method`, or `field` for Android input. Harmony input supports `string`, `type`, `method`, `field`, `constant`, `integer`, and `code`; `extends`, `impl`, and `--fuzzy-class` are rejected because they do not have equivalent Ark semantics. Harmony JSON records include the ABC member, caller, bytecode `pc`, opcode, hit kind, resolution state, and matched value. For `string`, `--max-match <N>` truncates long displayed constants and `--no-match` omits the matched value. `-o` also writes the output to a file.

`listmethod` prints one Dalvik method descriptor per line, for example `Lcom/example/Main;->onCreate(Landroid/os/Bundle;)V`. It lists only methods defined in class data, in DEX/class/declaration order with duplicates preserved. `--class` takes an exact dotted name or descriptor; `-f` is a case insensitive name substring; `--json` emits a JSON array. `-o` writes to a file instead of stdout.

`liststring` prints one string pool value per line, straight from the DEX string table, including literals nothing references. Order is DEX central-directory then string index; duplicates across DEX files are preserved unless `-u` / `--unique` is passed. `-f` is a case insensitive substring filter, while `findrefs string` is case sensitive. `--json` emits a JSON array. `-o` writes to a file instead of stdout.

`listmethod` and `liststring` are DEX-specific and return a capability error for HAP/ABC input.

`getmanifest` decodes `AndroidManifest.xml` for APK input. For HAP/HSP input it emits `module.json`, legacy `config.json`, and `pack.info` when present, with Harmony resource references resolved; `--json` returns an object keyed by member name.

`export` writes one `.java` pseudocode file per selected Android class, or one `.js`/`.pa` file per selected Harmony record. HAP exports use a member namespace so equal record names from different ABC files cannot overwrite each other. `--prefix` and `--classes-file` filter before decompilation. Missing requested items, unreadable members, or an empty filter result cause a nonzero exit while preserving files already produced.

## MCP

`rust_ASC mcp` runs a stdio MCP server that exposes the CLI commands as tools: `getclass`, `listclass`, `listmethod`, `liststring`, `getmanifest`, `findrefs`, and `export`. Calls execute in-process through the same dispatch path as the CLI. APK and HAP readers reuse bounded memory-mapped archive caches across requests.

- Every tool accepts `apk_path`; `input_path` is an alias. Harmony-capable tools accept HAP/HSP/ABC when the `harmony` feature is built.
- `getclass` accepts optional `class_name`, `classes`, `classes_file`, `json`, `thread`, `output`, `abc_output`, and `resources`.
- `listclass` accepts `apk_path`, optional `prefix`, `json`, `thread`, and `output`.
- `listmethod` accepts `apk_path`, optional `class_name`, `filter`, `json`, `thread`, and `output`.
- `liststring` accepts `apk_path`, optional `filter`, `unique`, `json`, `thread`, and `output`.
- `findrefs` accepts the Android query kinds plus Harmony `constant`, `integer`, and `code`, with optional `json`, `thread`, `output`, and `resources`. `string` queries also accept `max_match` and `no_match`.
- `getmanifest` accepts optional `output` and Harmony `json`.
- `export` accepts `output`, optional `prefix`, `classes_file`, `thread`, `abc_output`, and `resources`.

Configure it in your MCP client:

```json
{
  "mcpServers": {
    "asc": {
      "command": "/absolute/path/to/rust_ASC",
      "args": ["mcp"]
    }
  }
}
```

## Benchmark

### rust_ASC vs JADX

Same workloads. JADX figures come from the ASC benchmark suite.

**Global string cross-reference search**

| | TelegramX (59 MB) | WhatsApp (130 MB) | Grab (228 MB) | TencentQQ (318 MB) |
| --- | ---: | ---: | ---: | ---: |
| rust_ASC | 55 ms | 67 ms | 118 ms | 136 ms |
| JADX | 20 s | 32 s | 2m14s | 8m2s |

rust_ASC is 364x to 3,544x faster than JADX for global string xref search.

**Single-class decompile**

| | TelegramX (59 MB) | WhatsApp (130 MB) | Grab (228 MB) | TencentQQ (318 MB) |
| --- | ---: | ---: | ---: | ---: |
| rust_ASC | 79 ms | 70 ms | 111 ms | 118 ms |
| JADX | 6 s | 15 s | 37 s | 1m32s |

rust_ASC is 76x to 780x faster than JADX for single-class decompile.

**CLI peak memory**

| | TelegramX (59 MB) | WhatsApp (130 MB) | Grab (228 MB) | TencentQQ (318 MB) |
| --- | ---: | ---: | ---: | ---: |
| rust_ASC | 32 MB | 116 MB | 358 MB | 443 MB |
| JADX | 1,016 MB | 2.1 GB | 7.1 GB | 13.2 GB |

rust_ASC uses 19x to 32x less memory than JADX for CLI operation.

**Disk cache**

| | TelegramX | WhatsApp | Grab | TencentQQ |
| --- | ---: | ---: | ---: | ---: |
| rust_ASC | 0 MB | 0 MB | 0 MB | 0 MB |
| JADX | 119 MB | 30 MB | 127 MB | 322 MB |

rust_ASC stays fully in memory with zero disk cache, while JADX writes up to 322 MB.

Because every query is stateless, an agent can analyze hundreds of APKs at once, even on weak machines.

### rust_ASC CLI details

The following measurements use CLI commands with no cache, no preprocessing, and direct searches of every code reference to the string `https://`.

| APK | APK size | CLI string-reference search | CLI single-class decompile | CLI RSS peak | Persistent cache |
| --- | ---: | ---: | ---: | ---: | ---: |
| TelegramX | 59 MB | 55 ms | 79 ms — `com.coremedia.iso.boxes.AlbumBox` | 32 MB | 0 MB |
| WhatsApp | 130 MB | 67 ms | 70 ms — `com.whatsapp.apollo.Apollo` | 116 MB | 0 MB |
| Grab | 228 MB | 118 ms | 111 ms — `com.grab.arrears.view.ArrearsPickUpDropOffView` | 358 MB | 0 MB |
| TencentQQ | 318 MB | 136 ms | 118 ms — `com.tencent.icgame.game.trtc.roomservice.voiceroom.a.b` | 443 MB | 0 MB |

- **CLI string-reference search**: search all code references to `https://` directly from the APK, without cache or preprocessing.
- **CLI single-class decompile**: locate and decompile the named class directly from the APK.
- **Persistent cache**: disk cache written by the tool after command completion.

## Full APK Pseudocode Export Benchmark

Full-APK pseudocode export is provided through [MG1937/radguard](https://github.com/MG1937/radguard)'s public `export_to_directory` API and uses its shared parallel APK export pipeline. The following benchmark fully decompiles **TencentQQ.apk** (383 MB, 41 DEX files).

| Workers | Wall time | Speedup vs. 8 workers | Notes |
| ---: | ---: | ---: | --- |
| 8 | 7.4 s | 1.00× | 725% CPU, 827,840 KiB max RSS |
| 16 | 4.17 s | 1.78× | Reported run |
| 32 | 2.56 s | 2.89× | 2166% CPU, 2,639,588 KiB max RSS |

The worker count is a global export budget. The exporter overlaps APK/DEX processing with class decompilation while bounding per-DEX concurrent class work.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
