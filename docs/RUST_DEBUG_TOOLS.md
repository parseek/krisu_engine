# Rust 调试与分析工具链（Windows 实测）

> 本文记录**本机（Windows + MSVC 工具链）实际装好并验证过**的 Rust 调试 / 性能分析工具，
> 以及每个工具的真实可用性与用法。凡是"未验证"的地方都单独标注，不写成结论。
>
> 断点调试的**推荐路径只有一条**：CodeLLDB + DAP MCP（§3）。
> ferroscope 与 mcp-debugger 的 Rust 路径在本机**不可用**，原因见 §3.1。

---

## 0. 快速开始（TL;DR）

| 我想…… | 用什么 | 命令 |
|---|---|---|
| 打断点、看变量、单步 | CodeLLDB via DAP MCP | 见 §3.2 的 5 步工作流 |
| 找热点 / 画火焰图 | `cargo flamegraph`（**管理员**终端） | §4.1 |
| 看二进制里谁最占体积 | `cargo bloat` | §5.1 |
| 找单态化爆炸点 | `cargo llvm-lines` | §5.2 |
| 更快的测试 | `cargo nextest run` | §5.3 |
| 看宏展开成什么 | `cargo expand` | §5.4 |
| 覆盖率 | `cargo llvm-cov` | §5.5 |
| 改代码自动重跑 | `bacon` | §5.6 |

---

## 1. 本机环境事实（实测）

| 项 | 值 |
|---|---|
| 工具链 | `rustc 1.97.1` / `stable-x86_64-pc-windows-msvc` |
| **`CARGO_TARGET_DIR`** | **`C:\rust-targets\`** ← 仓库根目录下的 `target/` 是空的，别去那里找产物 |
| rustup 组件 | 新增 `llvm-tools-x86_64-pc-windows-msvc`（cargo-llvm-cov 依赖） |
| CodeLLDB | `C:\Users\35451\.vscode\extensions\vadimcn.vscode-lldb-1.12.3\adapter\codelldb.exe` |
| lldb / gdb | **不在 PATH 上**（所以 ferroscope 用不了，见 §3.1） |
| Node / npm | v24.19.0 / 11.17.0（MCP 类工具用） |
| WSL | Ubuntu（已安装，默认 Stopped） |

> `CARGO_TARGET_DIR` 是全局环境变量，构建产物统一落在 `C:\rust-targets\`。
> 这一点和 §3.3 的"PDB 找不到"事故直接相关。

---

## 2. 已安装清单与 Windows 适用性矩阵

### 2.1 cargo 工具（`~/.cargo/bin`）

| 工具 | 版本 | 用途 | Windows 实测 |
|---|---|---|---|
| `cargo-flamegraph` | 0.6.14 | 火焰图 | ⚠️ **需管理员**（§4.1） |
| `ferroscope` | 1.1.0 | LLDB/GDB 断点调试 MCP | ❌ 上游不支持 Windows（§3.1） |
| `cargo-bloat` | 0.12.1 | 各函数占多少体积 | ✅ 已验证 |
| `cargo-llvm-lines` | 0.4.48 | 每个泛型/`impl` 生成多少 LLVM IR 行 | ✅ 已验证 |
| `cargo-nextest` | 0.9.145 | 更快的测试执行器 | ✅ 已验证 |
| `cargo-expand` | 1.0.126 | 宏 / derive 展开 | ✅ 已验证（**stable 可用**） |
| `cargo-asm` | 0.1.16 | 查看某个函数的汇编 | ⚠️ 本机报错，见 §5.7 |
| `bacon` | 3.25.0 | 后台 `cargo check/test` 看板 | ✅ 已验证 |
| `cargo-llvm-cov` | 0.9.1 | 覆盖率（LLVM source-based） | ✅ 已验证 |
| `cargo-miri` | 随 stable 附带 | UB 检查 | ❌ 需 nightly，本机无 nightly 工具链 |
| `rust-gdb` / `rust-lldb` | 随 rustup | 命令行调试器包装 | ❌ MSVC 下 `rust-lldb` 直接报 "not applicable" |

### 2.2 MCP 调试服务

| 包 | 版本 | 结论 |
|---|---|---|
| `@flydut/debug-adapter-protocol-mcp` | 0.2.0 | ✅ **本仓库正在用，Rust 断点调试实测通过**（工具名 `mcp__dap__debug`） |
| `@debugmcp/mcp-debugger` | 0.17.0 | ⚠️ 已装，但 **Rust 适配器未发布到 npm**（§3.1） |
| `debugmcp`（Microsoft DebugMCP） | 0.1.0 | ⚠️ 已装（`npm i -g debugmcp`），只吃 **stdio 型** DAP 适配器；CodeLLDB 是 `--port` 型，本机未打通 |
| `ferroscope` | 1.1.0 | ❌ 见 §3.1 |

---

## 3. 断点调试

### 3.1 为什么不用 ferroscope / mcp-debugger（实测证据）

**ferroscope（已装，但本机不可用）**
- 上游 README 的 Requirements 写死：`LLDB (macOS) or GDB (Linux)`，Limitations 明确写 **"Windows is not supported (WinDbg integration planned)"**。
- 实测：MCP 握手成功、10 个工具都能列出，但 `debug_run` 直接失败：
  `{"error":{"code":-32602,"message":"Tool execution failed: program not found"}}`
  —— 因为它要 spawn `lldb`/`gdb`，而本机 PATH 上两者都没有。
- 结论：**保留安装，但 Windows 上不要用**。

**mcp-debugger（已装，Rust 路径不可用）**
- 包本身可用：`create_debug_session` / `set_breakpoint` / `get_local_variables` 等 17 个工具正常。
- 调用 `list_supported_languages` 的真实返回：
  ```json
  {"installed":["javascript","python","mock"],
   "available":[..., {"language":"rust","package":"@debugmcp/adapter-rust",
                      "installed":false,"description":"Rust debugger using CodeLLDB"}]}
  ```
  Rust 被列为 `installed: false`。
- 原因：`@debugmcp/adapter-rust` **从未发布到 npm**（`npm view @debugmcp/adapter-rust` → 404），
  它只存在于 monorepo 的 workspace 里；而动态加载还需要 `MCP_CONTAINER=true`。
- 附带发现：npm 包里的 `vendor/codelldb/` **只有 `linux-x64`**，没有 `win32-x64`。
- 结论：**JS/Python 可用；Rust 在 Windows 上走不通**。

### 3.2 推荐路径：CodeLLDB + DAP MCP（✅ 本机实测通过）

本仓库的 DSH `web` profile 已注册好 DAP MCP（`~/.dsh/profiles/web/cordis.patch.yml` 的 `mcp-dap` 条目），
模型侧看到的就是 `mcp__dap__debug`。**不需要额外配置**，直接按下面 5 步走：

```
1. launch            adapter = C:\...\vadimcn.vscode-lldb-1.12.3\adapter\codelldb.exe
                     program = C:\rust-targets\debug\<crate>.exe
                     cwd     = <crate 根目录>
                     dapArguments = { "stopOnEntry": true }   ← 关键：先停下来，否则程序会直接跑完
2. set_breakpoint    sessionId（上一步返回的 id）, file = 绝对路径, line = N
3. continue          sessionId, threadId（上一步快照里的 threadId）
4. stack_trace / scopes / variables / evaluate
5. step_over / step_into / step_out / terminate
```

**实测记录（一次完整的调试会话）**

```
launch(stopOnEntry)  → session codelldb-4, status=stopped,
                       停在 ntdll!RtlGetReturnAddressHijackTarget（正常，见下方坑 2）
set_breakpoint(main.rs:31) → {"line":31,"verified":true,"message":"Resolved locations: 1"}
continue             → stopReason="breakpoint", frameName="rustdbg_demo::main", line=31
stack_trace          → 完整 Rust 符号栈：
                        rustdbg_demo::main
                        <fn() as core::ops::function::FnOnce<()>>::call_once
                        std::sys::backtrace::__rust_begin_short_backtrace
                        std::rt::lang_start ... BaseThreadInitThunk
scopes(frameId)      → Local / Static / Global / Registers
variables(Local)     → particles : alloc::vec::Vec<rustdbg_demo::Particle, Global> = {len:2}
                        frame     : int = 0
                        iter      : core::slice::iter::IterMut<rustdbg_demo::Particle>
step_over(29→30)     → 生效
```

**两个必须知道的坑**

1. **PDB 必须在 exe 旁边。**
   本机 `CARGO_TARGET_DIR=C:\rust-targets\` 时，cargo 有可能**只**把 PDB 写在
   `C:\rust-targets\debug\deps\<crate>.pdb`，而没有复制/硬链接到
   `C:\rust-targets\debug\<crate>.pdb`。此时 LLDB 能解析类型，但**所有变量值都显示
   `<variable not available>`**（本次实测就是这样）。
   修复：
   ```powershell
   Copy-Item C:\rust-targets\debug\deps\<crate>.pdb C:\rust-targets\debug\<crate>.pdb -Force
   ```
   拷贝后重跑同一条断点，`particles` 立刻正确显示为 `{len:2}`。
   → 等价做法：直接调试 `C:\rust-targets\debug\deps\<crate>.exe`（PDB 就在旁边）。

2. **`stopOnEntry: true` 的第一停通常是 ntdll 异常**，不是 `main`。
   这是 Windows 上 LLDB 在进程启动早期的正常停靠；直接 `continue` 到你的断点即可。
   想少一次操作，也可以不传 `stopOnEntry`，改为"先 launch 让程序跑完 → 再 launch 新会话"是**无效的**
   （每次 launch 会新建 session id，断点不跨会话）；请沿用上面的顺序。

3. `evaluate` 对已被 `&mut` 借走的值会失败：
   `for p in &mut particles { p.step(..) }` 循环体内 `evaluate("particles.len()")` 报语法错误、
   `evaluate("frame")` 返回 `<variable not available>` —— **这不是工具坏了，是借用检查期的语义**；
   在循环体**之外**的行下断点即可正常读取（见上面的实测）。

### 3.3 其它（本机未打通，仅记录）

- **`debugmcp`（Microsoft DebugMCP）**：`npm i -g debugmcp` 已装。它只通过 **stdio** 与 DAP 适配器通信
  （`debugmcp adapter add <name> --command "<cmd>" --extensions .rs`）。CodeLLDB 只提供 `--port` 模式
  （`codelldb.exe --port <N>`），因此没有现成组合；要打通需额外的 stdio↔TCP 桥。
- **`lldb-dap`**：LLVM 官方的 stdio DAP 适配器，是 `debugmcp` 的正解，但本机没装 LLVM，未验证。

---

## 4. 性能分析

### 4.1 `cargo flamegraph`（本机：需管理员）

实测（非管理员终端）：

```
WARNING: profiling without debuginfo. Enable symbol information by adding:
[profile.release]
debug = true
Error: could not find dtrace and could not profile using blondie: NotAnAdmin
```

结论：
- Windows 上 cargo-flamegraph 走 **`blondie`** 采样后端，而 `blondie` **需要管理员权限**。
  → 用**管理员 PowerShell** 重跑即可（本机无法自动提权，故未做端到端验证）。
- 火焰图要有符号，release 也必须带调试信息：
  ```toml
  [profile.release]
  debug = 1        # 或 true；只影响符号，不影响优化
  ```
  或临时：`$env:CARGO_PROFILE_RELEASE_DEBUG=1`。
- 常用参数：
  ```powershell
  cargo flamegraph --bin <example> -o flame.svg -- --arg1
  cargo flamegraph --pid <PID> -F 997            # 附加到运行中的进程
  cargo flamegraph --inverted                    # 倒置（看调用者）
  cargo flamegraph --flamechart                  # 保留时间顺序，不合并栈
  cargo flamegraph --perfdata capture.perf       # 分析已有 perf 数据（Linux/WSL 产物）
  ```
- 备选：**WSL2 + `perf`**（Linux 侧采样最稳），或 `samply`（仅 macOS/Linux）。
  本机 WSL Ubuntu 存在但本次未启动验证。

### 4.2 图形程序的帧级分析（本仓库更推荐）

`cargo flamegraph` 是 CPU 采样，对"一帧里 GPU/CPU 各花了多少"帮助有限。这个 wgpu 引擎更适合：

- **Tracy**（`tracing-tracy` / `tracy-client`）：跨平台帧剖析器，带 GPU zone 支持，Windows 原生。
- **puffin + puffin_viewer**：纯 Rust 帧剖析器，`puffin_http` 起服务、`puffin_viewer` 连上看实时帧树，适合即时模式 UI。
- **RenderDoc**：抓帧看 draw call / 纹理 / 像素（本会话已有 `mcp__renderdoc__*`）。
- 先看 `Ui::debug_dump()` / `Frame` 事实（见 `docs/DEBUGGING.md` §1），再决定要不要上剖析器。

---

## 5. 其它已装工具用法

### 5.1 `cargo bloat` —— 谁最占体积

```powershell
cargo bloat --release -n 20            # 按体积列出前 20 个函数
cargo bloat --release --crates         # 按 crate 汇总
cargo bloat --release -p rjw_ui        # 只看某个包
```
实测输出（小 demo）：`.text section size 106.0KiB / file 148.0KiB`，并列出 `format_shortest` 等热点。

### 5.2 `cargo llvm-lines` —— 单态化爆炸

```powershell
cargo llvm-lines | Select-Object -First 20
```
输出 `Lines`（LLVM IR 行数）、`Copies`（被单态化出多少份）、函数名。
**编译慢的元凶通常是 Copies 高的泛型**（实测 demo 里 `RawVecInner::current_memory` 被复制 1 份/总计 52 行）。
对泛型大户（`rjw_typed_registry`、`rjw_2d_render` 的实例化路径）尤其有用。

### 5.3 `cargo nextest` —— 测试

```powershell
cargo nextest run                     # 全工作区
cargo nextest run -p rjw_ui           # 单包
cargo nextest run --no-fail-fast      # 跑完所有
cargo nextest run -E 'test(ui)'       # 过滤表达式
```
注意：`nextest` 不支持 doctest，doctest 仍需 `cargo test --doc`。

### 5.4 `cargo expand` —— 宏展开

```powershell
cargo expand                          # 默认 target
cargo expand -p rjw_krusie            # 指定包
cargo expand rjw_ui::widget           # 指定模块
```
**已实测在 stable 1.97.1 上可用**（内部用 `RUSTC_BOOTSTRAP`，无需切 nightly）。
写 proc-macro / derive 时配合 `cargo expand > out.rs` 再搜索最有效。

### 5.5 `cargo llvm-cov` —— 覆盖率

```powershell
cargo llvm-cov --html --open          # HTML 报告
cargo llvm-cov --summary-only         # 只看汇总
cargo llvm-cov --lcov --output-path lcov.info
```
依赖已装好的 `llvm-tools-x86_64-pc-windows-msvc` 组件。

### 5.6 `bacon` —— 后台检查看板

```powershell
bacon            # 等价 cargo check 的持续看板
bacon test       # 持续测试
bacon clippy     # 持续 clippy
```
在项目根目录跑，改文件即刷新；`q` 退出。

### 5.7 `cargo asm` —— 本机不可用

```powershell
cargo asm rustdbg_demo::total_energy
# [ERROR]: could not find function at path "..." in the generated assembly.
```
本机（MSVC + PDB 调试信息）下 cargo-asm 定位不到函数。替代：
`cargo rustc --release -- --emit asm` 后自己看，或用 VS / `dumpbin /disasm`。

---

## 6. 与本仓库约定的配合

- 断点调试用 **debug** 构建；性能分析用 **release**（已写入 `AGENTS.local.md`）。
- release 火焰图/剖析需要符号：在 workspace `Cargo.toml` 的 `[profile.release]` 里加 `debug = 1`。
- wgpu 相关失败优先看验证层：`device.create_*` 用 `push_error_scope` 包住并落日志。
- 产物路径永远记着 `CARGO_TARGET_DIR=C:\rust-targets\`。
- 调试 GPU 帧优先用 RenderDoc（`mcp__renderdoc__*`），不要用 CPU 火焰图去解释"没画面"。

---

## 7. 未完成 / 不可用清单

| 项 | 状态 |
|---|---|
| ferroscope 在 Windows 上调试 | ❌ 上游不支持；需 WinDbg 集成或 WSL/Linux |
| mcp-debugger 的 Rust 适配器 | ❌ `@debugmcp/adapter-rust` 未发布到 npm |
| debugmcp + CodeLLDB | ⚠️ 需 stdio 型适配器（如 `lldb-dap`），未打通 |
| cargo flamegraph 端到端 | ⚠️ 需管理员终端；本次仅验证到权限报错 |
| cargo-miri | ❌ 需 nightly（本机只有 stable + esp） |
| cargo-asm | ❌ 本机无法定位函数 |
| WSL2 + perf 火焰图 | ⚠️ 未验证（WSL Ubuntu 已安装） |
