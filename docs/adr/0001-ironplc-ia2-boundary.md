# ADR-0001: ironplc / IA2 职责边界

状态：已采纳（2026-06-13）

## 背景

IA2 通过 vendored submodule 使用 ironplc（parser / analyzer / codegen /
container / vm / dsl）。盘点结论：

- `crates/ironplc-bridge` 是**唯一**直接 import ironplc 的 crate（lsp-launcher
  仅调 `ironplc-cli::lsp::start()`）。server / cli / runtime 只见 bridge 的
  `Container` / `ProgramHandle` / `VarSnapshot` 等类型，无泄漏。
- vendored ironplc 此前为**未修改的上游**（pinned 31c40c69, v0.212.0 线）。
- 但 bridge 在四处 paper over 上游缺口，且从未把"谁负责什么"写成决策：
  1. codegen 不填 `container.task_table` → VM `next_due_us()` 永远 None，
     bridge 自己以 tasks.toml 的 interval sleep 调度（单一节拍）；
  2. VM `find_program()` 只执行容器里第一个 PROGRAM → server 拒绝
     多 PROGRAM 的 tasks.toml；
  3. codegen 丢失 `VAR RETAIN` 限定符 → bridge 在 codegen 前走 AST 提取
     retain 变量名；
  4. VM 写 API 仅 `write_variable(i32)` → LREAL 输入映射被跳过、RETAIN
     对 64 位类型截断。

## 决策：边界原则

**ironplc 负责"语言"：IEC 61131-3 文本 → 可执行单元的一个扫描周期。**
**IA2 负责"工程"：把 N 个可执行单元编排成一座工厂的控制层。**

| 能力 | 归属 | 形态 |
|---|---|---|
| 解析 / 语义分析 / 问题码 + RST 文档 | ironplc | bridge 透传 `CheckDiagnostic` |
| 字节码容器 + 调试段 | ironplc | bridge 只读（`build_var_debug_map`） |
| VM：执行一个容器的一个 scan（`run_round`）、变量读写 | ironplc | bridge 持有 `VmRunning` |
| LSP server（语法/符号/语义着色） | ironplc | lsp-launcher 拉起；**诊断不走 LSP**（单文件视角），走 IA2 项目感知 `/api/check` |
| CONFIGURATION 合成（tasks.toml → IEC 文本） | IA2 bridge | `synthesize_configuration` |
| 任务调度（多任务节拍、多 PROGRAM 编排） | **IA2 bridge** | 见下"多 PROGRAM 设计" |
| RETAIN 提取 + 持久化 + 恢复 | IA2 bridge | AST 提取 + `retain.rs` 盘上格式 |
| I/O：设备、通道、映射、failsafe、watchdog | IA2（iocore/iomap-*） | VM 无感知 |
| 工程模型：项目/库/Edge/部署/IDE/HTTP API | IA2 | — |

判据：凡 IEC 61131-3 标准文本定义的语义（语法、类型、单 POU 执行）归
ironplc；凡"标准之外让它成为产品"的（调度策略、持久化格式、硬件、
多项目、IDE）归 IA2。**不把工程概念塞进 vendor，也不在 IA2 重新实现
语言。**

## 决策：vendor 策略（fork + 最小补丁注册表）

submodule 指向 fork `supcon-international/ironplc`，分支 `ia2-patches`，
基于上游 pinned commit。补丁准入：**只接受"上游理应提供的窄 API"**，
不接受任何 IA2 业务语义。每个补丁登记于下表并以 PR 形式回馈上游；
上游合并后 rebase 掉对应补丁。

| # | 补丁 | 动机 | 上游 PR |
|---|---|---|---|
| 1 | `vm: write_variable_raw(VarIndex, u64)`（d06a646c） | `read_variable_raw` 有 u64 读但无对称写；RETAIN 恢复与 64 位 I/O 映射需要无截断写入 | 待提 |

升级流程：`git fetch upstream && git rebase upstream/main ia2-patches`，
补丁冲突即重新评估是否仍需要。

## 决策：多 PROGRAM / 多任务在 IA2 侧实现

不等上游 task_table codegen。bridge 改为 **每 PROGRAM 实例一个
Container + 一个 VM，单 scan 线程轮转调度**：

- 编译：每个 `tasks.toml` program 条目单独 `compile_isolated_source`
  （基建已存在）；每容器独立 debug 段 → 多实例变量名缺失问题
  （上游 debug_section 只命名第一个实例）随之消失。
- 调度：每任务独立 `next_due` 锚点；线程每轮执行所有到期任务绑定的
  VM 的 `run_round`，再睡到最近 due。优先级 = 同刻到期时的执行顺序。
- I/O 路由：`Mapping.application` 字段（已存在）选择目标 VM；设备仍由
  scan 线程独占，多 VM 同线程轮转，无并发问题。
- 快照：变量名带实例前缀 `instance.variable`，跨 VM 合并。
- 约束：多 PROGRAM 模式下不支持跨 PROGRAM 的 GLOBAL VAR 共享
  （分容器隔离了地址空间）；`/api/project/validate` 检测并报错。
- 硬件控制权不变：server 全局同时只允许一个项目运行。

若上游未来落地 task_table + 多 PROGRAM 容器语义，bridge 可把"轮转多
VM"退化为"单容器多任务"，对上层 API 无感。

## 后续（上游候选）

1. PR：`write_variable_raw`（补丁 #1）。
2. Issue/PR：codegen 填充 `container.task_table`（VM 的 scheduler.rs
   骨架已在）。
3. Issue：debug_section 为每个 PROGRAM 实例命名变量。
