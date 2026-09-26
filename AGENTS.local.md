# 本地开发规则（AGENTS.local.md）

## Token 节约
- 所有可能产生大量输出的命令，必须进行字节上限截断。
- 本文件行数应保持在 120 行以内，超出时优先精简低频规则。

## Rust 图形程序调试
- 断点调试使用 `cargo build`（debug 模式），性能分析使用 `cargo build --release`。
- 在 `device.create_*` 等关键调用周围使用 `push_error_scope` 包裹，并将错误输出到日志。
- 修改任何涉及 `unsafe` buffer 转换的结构体后，必须添加编译期布局断言。

## Rust 调试工具链（Windows，详见 `docs/RUST_DEBUG_TOOLS.md`）
- **产物不在 `./target`**：全局 `CARGO_TARGET_DIR=C:\rust-targets\`，debug 产物在
  `C:\rust-targets\debug\`，PDB 可能在 `...\debug\deps\`。
- **断点调试唯一推荐路径**：CodeLLDB + DAP MCP（工具 `mcp__dap__debug`）。顺序固定：
  `launch(dapArguments.stopOnEntry=true)` → `set_breakpoint(sessionId,file,line)` →
  `continue(sessionId,threadId)` → `stack_trace`/`scopes`/`variables`/`step_*`。
  每次 `launch` 生成**新** session id，断点不跨会话；第一停常是 ntdll 异常，`continue` 即可。
- 调试器路径：
  `C:\Users\35451\.vscode\extensions\vadimcn.vscode-lldb-1.12.3\adapter\codelldb.exe`。
- **变量显示 `<variable not available>` 先查 PDB**：
  `Copy-Item C:\rust-targets\debug\deps\<crate>.pdb C:\rust-targets\debug\<crate>.pdb -Force`。
- **不要**用 ferroscope 调 Windows（上游不支持，需 lldb/gdb）；**不要**指望 mcp-debugger 调 Rust
  （`@debugmcp/adapter-rust` 未发布到 npm，`list_supported_languages` 里恒为 `installed:false`）。
- 性能分析：`cargo flamegraph` 在 Windows 走 blondie 后端，**必须管理员终端**；release 需要
  `[profile.release] debug = 1` 才有符号。非管理员时改用 WSL2 + `perf`，或 Tracy / puffin。
- 常用分析命令：`cargo bloat --release -n 20`（体积）、`cargo llvm-lines`（单态化）、
  `cargo nextest run`（测试）、`cargo expand`（宏展开，stable 可用）、`cargo llvm-cov`（覆盖率）、
  `bacon`（持续检查）。
- 不可用：`cargo-miri`（需 nightly）、`cargo-asm`（本机定位不到函数）、`rust-lldb`（MSVC 不适用）。
- GPU 帧问题用 RenderDoc（`mcp__renderdoc__*`），不要用 CPU 火焰图解释"没画面"。

## 架构治理
- 修改公开 API 后，必须同步更新 `docs/ARCHITECTURE.md`。
- 每个非平凡模块顶部必须编写“维护者笔记”。
- 提交信息格式为 `type(scope): 为什么改`。
- 每次功能完成后，输出 3-5 句话的“架构影响说明”。

## 注意
- 对于 ui 模块，请使用简洁的 egUI 作为测试器，不再使用难以维护的 eg260818UI。

## -≤- 目标

> Easy to use but powerful.  
> Clean and easy to maintain **by hands**.  
> Quick but customizable.  
> -- krisuRJW

## **重要**
- 请使用简体中文进行思考，并用简体中文像用户解释每一步在执行什么，执行的理由和结果（如文件更改部分等）。
- 让用户有足够的决策空间，使得人机协同开发变得可控且可靠。