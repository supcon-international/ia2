# process-control — IEC 61131-3 ST 过程控制 FB 库

面向 IA2（vendored ironplc 方言）的常用过程控制功能块，全部为纯 ST、
无标准 FB 依赖（计时一律用 `dt_s` 手工累加，离线可测、跨方言可移植）。
每个文件均通过 `cs check` 静态检查，整库通过 `cs project check` 全量编译
（含 codegen）验证。

## FB 索引（一行一个）

| 文件 | FB | 一句话说明 |
|---|---|---|
| `pous/fb_scale.st` | `FB_SCALE` | 原始计数 → 工程单位线性变换，防除零，越界 >5% 出 NE43 风格断线提示，可选钳位 |
| `pous/fb_lag.st` | `FB_LAG` | 一阶惯性滤波 PT1（PT1/LAG），t_s=0 直通，reset/首扫描对中 |
| `pous/fb_leadlag.st` | `FB_LEADLAG` | 超前-滞后补偿（前馈动态整形，lead-lag），内部 PT1 + 直通混合离散近似 |
| `pous/fb_ramp.st` | `FB_RAMP` | 设定值斜坡发生器：朝 target 限速逼近，上/下行速率独立，0=跳变，track 无扰对中，ramping 指示 |
| `pous/fb_rate_limit.st` | `FB_RATE_LIMIT` | 变化率限幅器（velocity limiter）：跟随连续变化的输入，只钳 |Δ|/s；与 RAMP 的区别是无固定目标 |
| `pous/fb_sqrt_flow.st` | `FB_SQRT_FLOW` | 差压流量开方提取：flow_max*SQRT(dp/100)，小流量切除（默认 6.25% 差压 ↔ 25% 流量） |
| `pous/fb_char.st` | `FB_CHAR` | 折线特性化（piecewise-linear characterizer）：2..8 点分段线性插值，两端钳位；标量点对形式（FB 内数组不能 codegen，见坑 7） |
| `pous/fb_pid.st` | `FB_PID` | 增量式（速度型）PID：条件积分+回算抗饱和、手/自动无扰，新增 ff 前馈、track 输出跟踪、sp_rate 设定值斜坡、dev 偏差输出（全带默认值，向后兼容；典型场景：流量 PID 驱动变频泵，输出限幅即频率上下限） |
| `pous/fb_ratio.st` | `FB_RATIO` | 比值站（ratio station）：sp = clamp(ratio)*wild_flow + bias，接下游 FB_PID 的 sp |
| `pous/fb_select_hl.st` | `FB_SELECT_HL` | 高/低选择器（超驰控制 >/< 选择），a_selected 指示选中通道 |
| `pous/fb_split_range.st` | `FB_SPLIT_RANGE` | 分程输出：u 0..100 按 split 分到两阀，A 段 [0,split] 0→100，B 段 [split,100] 0→100 或反转（reverse_b），全程钳位连续 |
| `pous/fb_pwm.st` | `FB_PWM` | 时间比例输出（time-proportioning）：0..100% → 周期占空比通断，带最小通/断时间钳位，停用清相位 |
| `pous/fb_alarm_hl.st` | `FB_ALARM_HL` | H/L/HH/LL 四级报警，死区 + 延时 + 简化 ISA-18.2 确认锁存，附原始越限位 |
| `pous/fb_alarm_dev.st` | `FB_ALARM_DEV` | 偏差报警（pv-sp 上/下偏差，deviation alarm），死区+延时+确认锁存（范式同 FB_ALARM_HL） |
| `pous/fb_alarm_roc.st` | `FB_ALARM_ROC` | 变化率报警（rate-of-change alarm）：导数经内部 PT1（3*dt_s）去噪，|roc| 超限延时报警+锁存，roc 输出可看趋势 |
| `pous/fb_debounce.st` | `FB_DEBOUNCE` | DI 去抖：u 连续保持 t_on_s 置位 / t_off_s 复位（双向独立确认） |
| `pous/fb_motor.st` | `FB_MOTOR` | 电机启停封装：远程闸启动、停车优先、反馈超时不一致故障锁存、状态字 0/1/2/3 |
| `pous/fb_valve.st` | `FB_VALVE` | 开关阀封装：ZSO/ZSC 回讯、行程超时与双限位故障、故障安全关阀、状态字 0/1/2/3 |
| `pous/fb_runtime.st` | `FB_RUNTIME` | 设备运行小时 + 启动次数统计（上升沿计数），reset 清零，实例可放 VAR RETAIN，喂 FB_DUTY2 做小时均衡 |
| `pous/fb_duty2.st` | `FB_DUTY2` | 双泵一用一备：每启轮换或小时均衡选值班，已在转的泵无扰接管，值班故障备用立即顶上，双故障全停；纯沿记忆无计时 |
| `pous/fb_interlock8.st` | `FB_INTERLOCK8` | 8 路联锁汇总 + 首出记录（DCS first-out）：条件全清且 reset 才复位，enable1..8 可旁路 |
| `pous/fb_totalizer.st` | `FB_TOTALIZER` | 流量累积 m3/h → m3（`total := total + flow*dt_s/3600`），reset 清零，实例可放 VAR RETAIN |
| `pous/fb_hyst.st` | `FB_HYST` | 双向迟滞开关：按 on_sp/off_sp 相对位置自动判高位接通或低位接通（如分级机电流控给料） |
| `pous/fb_hilo_fill.st` | `FB_HILO_FILL` | 高位关低位开补给控制（水箱补水/料仓补料），区间内保持 |
| `pous/fb_pulser.st` | `FB_PULSER` | 周期脉冲发生器（空气炮/气锤吹扫）：每 period_s 出 pulse_len_s 脉冲，停用清相位 |
| `pous/fb_alt2.st` | `FB_ALT2` | 双阀定时轮换 + 双外部联锁（除铁器+包装秤）：任一联锁失去两阀全关 |
| `pous/demo_main.st` | `demo_main`（PROGRAM） | 迷你碳化塔回路演示：量程→PID→报警→空气炮→补水 + 比值→PID(前馈/track)→分程、双泵 DUTY2+RUNTIME、INTERLOCK8 首出；**自包含**（内联 10 个 FB 拷贝，原因见下） |

## 使用说明

- **消费方式**：把需要的 `fb_*.st` 拷进项目的 `pous/`，在你的 PROGRAM 里
  声明实例并按命名参数调用，调度由项目 `tasks.toml` 决定（POU 文件里不要写
  CONFIGURATION）。
- **`dt_s` 约定**：所有含时间行为的 FB 都吃一个 REAL 采样周期（秒）。
  传 `tasks.toml` 的 `interval_ms / 1000.0`（默认 100 ms → `0.1`）。改了任务
  周期记得同步改这个常量，否则所有延时/脉冲/累积都按错的时基跑。
- **断电保持**：`FB_TOTALIZER` / `FB_RUNTIME` 实例放在 `VAR RETAIN ... END_VAR`
  下即可随 IA2 的 retain.json 快照保持。注意 IA2 恢复按 i32 存取，REAL 累积值
  的保持精度受此限制（计数/设定值类没问题）。
- **报警确认**：`FB_ALARM_HL` / `FB_ALARM_DEV` / `FB_ALARM_ROC` 的 `ack` 是
  电平有效；输出语义为 `报警 = 触发条件 OR (已锁存 AND NOT ack)` —— 条件在则
  常报，条件消失后保持到确认。`FB_INTERLOCK8` 同理但用 `reset`：条件全清且
  reset 才复位，首出号随复位清零。
- **FB_PID 升级（前馈/跟踪/SP 斜坡）向后兼容**：新增输入 `ff`、`track` +
  `track_value`、`sp_rate` 均带默认值，取默认值时与旧版逐拍一致，旧调用
  无需改动。模式优先级 track > 手动 > 自动；`ff` 在钳位前叠加，靠每拍回算
  `acc := out - ff` 保证钳位/前馈增减/模式切换全程无扰；`dev` 输出是经
  SP 斜坡后的工作偏差（pv - sp_int）。
- **首扫描对中**：`FB_LAG` / `FB_LEADLAG` / `FB_RATE_LIMIT` 首个扫描周期自动
  `out := u`（避免从 0 爬升的投运冲击）；`FB_RAMP` / `FB_PID` 的 sp 斜坡同理
  （RAMP 输出本身用 track 对中）。
- **demo_main.st 是自包含的**：它内联了所用 10 个 FB 的逐字拷贝（由单 FB
  文件 `cat` 拼装生成，保证逐字一致）。要单独跑 demo，把 `demo_main.st`
  一个文件放进项目即可。与单 FB 文件混放同一项目时，当前 vendored ironplc
  实测**容忍**逐字相同的 FUNCTION_BLOCK 重复声明（项目编译通过），但哪份
  生效未定义——不要让拷贝漂移；正式项目建议二选一。

### 验证命令（本库交付时的实测结果）

```text
$ target/release/cs check library/process-control/pous/*.st
✓ 27 files clean        # 退出码 0

# 跨文件 + codegen 全量验证（临时项目：26 个单 FB 文件 + demo_main.st +
# 实例化全部 26 个 FB 的测试 PROGRAM；fb_test 与 demo_main 两个 PROGRAM
# 同绑一个 task，重复 FB 声明被容忍——一次性验证整库 + demo）
$ target/release/cs project check /tmp/fbcheck
✓ project pc_lib_test compiles cleanly
```

## 踩到的方言坑（vendored ironplc / cs）

1. **`cs check` 是逐文件独立检查，不做跨文件类型解析。** PROGRAM 实例化
   另一个文件里声明的 FB 时，单文件检查必报
   `P2008 Cannot determine kind of type identifier` + `P4012 invocation is not
   a variable in scope`——即使把多个文件一起传给 `cs check` 也一样（按文档
   "each is checked independently"）。跨文件解析只发生在**项目级编译**
   （`cs project check <dir>`，离线、不需要 server，且包含 codegen）。
   这就是 `demo_main.st` 必须内联 FB 拷贝的原因。
2. **`dt` / `DT` 是保留字**（DATE_AND_TIME 类型），用作变量名直接
   P0002 语法错误。本库统一用 `dt_s`、`td_s`、`ti_s`。
3. **重复 FUNCTION_BLOCK 声明在项目编译下被容忍**（实测：同名同体 FB 出现
   在两个文件，`cs project check` 通过，不报重复声明）。方便，但也意味着
   拷贝漂移不会被编译器抓住——靠纪律保持同步。
4. **`MAX()` / `ABS()` / `SQRT()` 实测可用**（静态检查与项目编译 codegen 均
   通过；SQRT 由 `FB_SQRT_FLOW` 实际使用）。MAX/ABS 本库仍按可移植性偏好用
   内联 `IF` 实现（如 `FB_SCALE` 的防除零下限），不依赖内建函数表。
5. **以下写法均验证可用**（写库前逐一探针确认）：中文 `(* ... *)` 注释及
   `→`/`≥` 等字符、科学计数法字面量（`1.0E-6`）、负数字面量初值、FB 体内
   `RETURN` 提前返回、`VAR_INPUT` 默认值（`kp : REAL := 1.0;`，含
   `enable1 : BOOL := TRUE` 这类默认 TRUE）、多行命名参数调用、表达式直接
   作实参（`c4 := fault_a AND fault_b`、`ff := gas_flow * 0.01`）、把一个
   实例的输出直接作另一个调用的实参（`permissive1 := mot.out_run`）、
   `CASE ... OF 1, 2: ... ELSE ... END_CASE`、
   `enable/reset/auto/direct/total/state/out/q` 等标识符不与保留字冲突、
   一个项目多个 PROGRAM 且 tasks.toml 多条 `[[programs]]` 同绑一个 task。
6. **通用方言规则**（沿自 IA2 技能文档，本库遵守）：布尔用 `AND/OR/NOT`
   不用 `&&/||/!`；每条语句以 `;` 结尾（含 `END_IF` 前最后一条）；POU 文件里
   不写 CONFIGURATION/TASK（由 tasks.toml 合成）；优先手工 `dt_s` 累加计时
   而非 TON/TP 实例（可测、可移植——本库零标准 FB 依赖即源于此）。
7. **FUNCTION_BLOCK 作用域的 ARRAY 不能 codegen（且 `cs check` 抓不到）。**
   `ARRAY[1..8] OF REAL` 在 FB 的 VAR / VAR_INPUT 里**静态检查通过**
   （含变量下标读写），但项目编译在 codegen 阶段报
   `P9999 Capability is not implemented`（`compile_array.rs#L156`：
   `array_vars` 只登记 PROGRAM 作用域变量）。PROGRAM 作用域数组完全可用
   （元素读写、变量下标、`:= [..]` 字面量初始化均过 codegen）；整数组作
   实参传给 FB 自然也不可用。这就是 `FB_CHAR` 用标量点对 x1..x8/y1..y8
   而非数组的原因。教训：**`cs check` 过 ≠ codegen 过**，新语法务必用
   `cs project check` 探针到底。
