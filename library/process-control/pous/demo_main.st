(* ============================================================================
  demo_main — 迷你碳化塔回路演示 PROGRAM（自包含文件）。

  注意：本文件内联了它实例化的 28 个 FB 声明的逐字拷贝（FB_SCALE / FB_PID /
  FB_ALARM_HL / FB_PULSER / FB_HILO_FILL / FB_RATIO / FB_SPLIT_RANGE /
  FB_RUNTIME / FB_DUTY2 / FB_INTERLOCK8 / FB_LIMIT / FB_STATS / FB_AVG_TIME /
  FB_INTEG / FB_DERIV / FB_DEADTIME8 / FB_MID3 / FB_CASCADE / FB_PV_HOLD /
  FB_RAMPSOAK8 / FB_GAIN_SCHED3 / FB_PID_POS / FB_MANSTATION / FB_PID_3STEP /
  FB_MOTOR_REV / FB_MOV / FB_DOSE / FB_FLOW_COMP）。原因：`cs check` 对每个文件独立
  做静态检查，不做跨文件类型解析；只有项目级编译（cs project check /
  tasks.toml 调度编译）才会把 pous/ 下所有文件合并解析。自包含使
  `cs check pous/*.st` 整组通过。
  因此：把本文件导入真实项目时，不要与同目录的 fb_scale.st 等单 FB 文件
  共存（虽然实测项目编译容忍逐字相同的重复声明，但拷贝漂移无人兜底）
  ——二选一。

  回路接线（演示，变量可经 iomap.toml 绑定到实际通道）：
    lt0101_raw（AI 原始计数）→ FB_SCALE → level_pct（塔液位 %）
    level_pct / level_sp     → FB_PID（反作用，旧签名调用，验证向后兼容）
                               → valve_pct（进料调节阀开度）
    level_pct                → FB_ALARM_HL → 四级液位报警 + any_alm
    hammer_enable            → FB_PULSER → hammer_q（空气炮周期吹扫）
    level_pct                → FB_HILO_FILL → fill_open（高位关低位开补水）
    —— 以下为扩展回路 ——
    液位HH/断线/塔温HH/双泵故障 → FB_INTERLOCK8 → ilk_trip + 首出号
    pump_run_fb_a/b          → FB_RUNTIME ×2 → 双泵小时/启动次数
    fill_open AND NOT ilk    → FB_DUTY2（小时均衡）→ 双补水泵一用一备
    gas_flow（主动量）       → FB_RATIO → milk_sp（石灰乳配比设定）
    milk_flow / milk_sp      → FB_PID（前馈 ff + SP 斜坡 + 联锁 track 强制）
                               → FB_SPLIT_RANGE（顺序分程）→ 小阀/大阀
    —— 以下为数学/动态块演示（蒸汽加热级联 + FOPDT 对象仿真） ——
    steam_valve → FB_DEADTIME8(20 s) → 手工 PT1(30 s) → 塔温 temp_a
                               （纯滞后+一阶惯性 = FOPDT 对象仿真）
    temp_a / temp_b / 坏点    → FB_MID3（两好一坏取均值）→ FB_LIMIT → 表决塔温
    表决塔温                  → FB_DERIV → 变化率 ℃/s；
                               → FB_STATS（全程统计）+ FB_AVG_TIME（60 s 翻转窗）
    塔温主 FB_PID → FB_CASCADE → 蒸汽流量副 FB_PID → steam_valve
                               （非级联时主调经 track 跟踪，CAS 投切无扰）
    主 PID 偏差 dev           → FB_INTEG（联锁 hold，钳 ±1000）→ 偏差积分 ℃·s
    —— 以下为 0.3.0 回路/设备/信号扩展块演示（第 17~26 节） ——
    表决塔温 + q_fail        → FB_PV_HOLD（坏值先保持 5 s 再保持/替代）→ temp_held
    升温-保温 4 段曲线        → FB_RAMPSOAK8 → rs_sp（程序设定值）
    temp_held                → FB_GAIN_SCHED3（三区参数 + 区界过渡）
                               → FB_PID_POS（程序 SP + 调度参数，p/i/d 可观测）
                               → FB_MANSTATION（手操站，balance 无扰）→ heat_out
    temp_held / rs_sp        → FB_PID_3STEP（电动执行机构脉冲调节，观察用）
    搅拌正/反按钮             → FB_MOTOR_REV（换向闭锁 2 s，反馈回送仿真）
    fill_open                → FB_MOV（电动补水阀，30 s 仿真行程双限位）
    下料启动                  → FB_DOSE（粗 8 / 精 1.5 单位/s 仿真流量）
    gas_flow + 工况 p/t       → FB_FLOW_COMP（DP 开方式温压补偿）→ gas_flow_std
  scan_s 必须与 tasks.toml 的 interval_ms 一致（默认 100 ms = 0.1 s）。
============================================================================ *)

(*
  FB_SCALE — 线性量程变换：原始计数值 → 工程单位，含 NE43 风格断线提示。

  用途：模拟量输入标定。例如 4-20 mA 采样为原始计数（S7 风格 5530..27648），
  变换为 0..100 % 或 0..5 m 等工程值。

  公式：out := out_min + (raw - raw_min) * (out_max - out_min) / span
        其中 span = MAX(raw_max - raw_min, 1e-6)（内联 IF 实现，防除零）。
  假定 raw_max > raw_min（正向量程）。

  输入：
    raw      REAL  原始计数值（如 AI 通道原值）
    raw_min  REAL  量程下限对应的原始值（4 mA 点）
    raw_max  REAL  量程上限对应的原始值（20 mA 点）
    out_min  REAL  工程量程下限
    out_max  REAL  工程量程上限
    clamp    BOOL  TRUE 时输出钳位在 [out_min, out_max]（支持反向工程量程）
  输出：
    out      REAL  工程值
    fault    BOOL  原始值越界 >5% 量程（NE43 风格断线/超限提示，不锁存）
*)
FUNCTION_BLOCK FB_SCALE
  VAR_INPUT
    raw     : REAL;
    raw_min : REAL := 0.0;
    raw_max : REAL := 27648.0;
    out_min : REAL := 0.0;
    out_max : REAL := 100.0;
    clamp   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out   : REAL;
    fault : BOOL;
  END_VAR
  VAR
    span   : REAL;
    margin : REAL;
    lo     : REAL;
    hi     : REAL;
  END_VAR

  (* span = MAX(raw_max - raw_min, 1e-6) — 内联 IF 防除零 *)
  span := raw_max - raw_min;
  IF span < 0.000001 THEN
    span := 0.000001;
  END_IF;

  (* NE43 风格越界判定：超出原始量程 5% 以上视为断线/故障 *)
  margin := 0.05 * span;
  fault := (raw < raw_min - margin) OR (raw > raw_max + margin);

  out := out_min + (raw - raw_min) * (out_max - out_min) / span;

  IF clamp THEN
    IF out_min <= out_max THEN
      lo := out_min;
      hi := out_max;
    ELSE
      lo := out_max;
      hi := out_min;
    END_IF;
    IF out < lo THEN
      out := lo;
    ELSIF out > hi THEN
      out := hi;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_PID — 增量式（速度型）PID 调节器，带抗积分饱和、无扰切换、前馈、
  输出跟踪与设定值斜坡。

  ≈ 通用 PID 必备形态：SP 斜坡 / 输出跟踪 / 前馈 / 手自动无扰（各主流 DCS/PLC 标准库均有同类块）。

  典型用法：流量 PID 驱动变频泵，正作用，输出限幅即变频上下限（如 10~45Hz
  ——流量作 pv，变频器频率给定作 out：out_min:=10.0, out_max:=45.0,
    direct := TRUE 正作用——pv 升高输出升高）。

  算法（增量式 + 前馈）：
    sp_int := sp 经 sp_rate 限速斜坡（sp_rate <= 0 直接取 sp；首扫描对中）
    e      := sp_int - pv（反作用，默认）；direct=TRUE 时 e := pv - sp_int
    Δpid   := kp*(e - e_prev)                          —— 比例增量
            + kp*dt_s/ti_s * e（ti_s>0 时）            —— 积分增量
            + kp*td_s/dt_s * (e - 2*e_prev + e_prev2)（td_s>0 时）—— 微分增量
    acc    := acc + Δpid；out := acc + ff，钳位到 [out_min, out_max]
    收尾    acc := out - ff（回算）
  抗积分饱和：输出钳位 + 回算 + 条件积分（输出已贴限且积分增量继续推向
  限幅时丢弃积分项）。
  模式优先级：track（输出跟踪，联锁强制/外部接管用）> NOT auto（手动）>
  自动。track/手动时 out 跟踪给定值（钳位），误差历史照常更新，回到自动
  无跳变。ff 直接叠加在钳位前输出上（前馈即时生效；回算保证 ff 增减、
  钳位、模式切换全程无扰）。
  向后兼容：新增输入（ff/track/track_value/sp_rate）均带默认值，取默认值
  时行为与旧版逐拍一致，既有调用无需改动；新增输出 dev 不影响旧接线。

  输入：
    pv          REAL  过程值
    sp          REAL  设定值
    kp          REAL  比例增益
    ti_s        REAL  积分时间 s（0 = 积分关）
    td_s        REAL  微分时间 s（0 = 微分关，微分项不滤波）
    dt_s        REAL  采样周期 s（<=0 时本周期保持不动作）
    direct      BOOL  TRUE = 正作用（pv↑→out↑），FALSE = 反作用
    auto        BOOL  TRUE 自动 / FALSE 手动
    manual_out  REAL  手动给定值
    out_min     REAL  输出下限（假定 out_min < out_max）
    out_max     REAL  输出上限
    ff          REAL  前馈量（钳位前叠加到输出；接 FB_LEADLAG 可加动态补偿）
    track       BOOL  输出跟踪使能（优先级最高，盖过 auto/手动）
    track_value REAL  跟踪值
    sp_rate     REAL  设定值斜坡限速 单位/s（0 = 不限速）
  输出：
    out         REAL  调节输出（各模式均钳位）
    dev         REAL  偏差 pv - sp_int（经 SP 斜坡后的实际工作偏差）
*)
FUNCTION_BLOCK FB_PID
  VAR_INPUT
    pv          : REAL;
    sp          : REAL;
    kp          : REAL := 1.0;
    ti_s        : REAL := 0.0;
    td_s        : REAL := 0.0;
    dt_s        : REAL := 0.1;
    direct      : BOOL := FALSE;
    auto        : BOOL := TRUE;
    manual_out  : REAL := 0.0;
    out_min     : REAL := 0.0;
    out_max     : REAL := 100.0;
    ff          : REAL := 0.0;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
    sp_rate     : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out : REAL;
    dev : REAL;
  END_VAR
  VAR
    sp_int    : REAL; (* 斜坡后的内部工作设定值 *)
    acc       : REAL; (* PID 累加器（不含 ff） *)
    e         : REAL;
    e_prev    : REAL;
    e_prev2   : REAL;
    p_term    : REAL;
    i_term    : REAL;
    d_term    : REAL;
    sp_step   : REAL;
    sp_delta  : REAL;
    init_done : BOOL; (* sp_int 首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持输出与历史不变 *)
  END_IF;

  (* 设定值斜坡：sp_rate > 0 时 sp_int 限速逼近 sp；首扫描直接对中 *)
  IF NOT init_done THEN
    sp_int := sp;
    init_done := TRUE;
  ELSIF sp_rate > 0.0 THEN
    sp_step := sp_rate * dt_s;
    sp_delta := sp - sp_int;
    IF sp_delta > sp_step THEN
      sp_int := sp_int + sp_step;
    ELSIF sp_delta < 0.0 - sp_step THEN
      sp_int := sp_int - sp_step;
    ELSE
      sp_int := sp;
    END_IF;
  ELSE
    sp_int := sp;
  END_IF;

  dev := pv - sp_int;

  (* 控制偏差；正作用取反号 *)
  IF direct THEN
    e := pv - sp_int;
  ELSE
    e := sp_int - pv;
  END_IF;

  IF track THEN
    (* 输出跟踪：无条件跟随 track_value（联锁强制/超驰接管），优先级最高 *)
    out := track_value;
  ELSIF NOT auto THEN
    (* 手动：跟踪 manual_out，切回自动时增量式自然无扰 *)
    out := manual_out;
  ELSE
    p_term := kp * (e - e_prev);

    i_term := 0.0;
    IF ti_s > 0.0 THEN
      i_term := kp * dt_s / ti_s * e;
      (* 条件积分：输出已饱和且积分仍推向限幅方向时，本周期丢弃积分项 *)
      IF (out >= out_max AND i_term > 0.0) OR (out <= out_min AND i_term < 0.0) THEN
        i_term := 0.0;
      END_IF;
    END_IF;

    d_term := 0.0;
    IF td_s > 0.0 THEN
      d_term := kp * td_s / dt_s * (e - 2.0 * e_prev + e_prev2);
    END_IF;

    acc := acc + p_term + i_term + d_term;
    out := acc + ff; (* 前馈钳位前叠加 *)
  END_IF;

  (* 输出钳位（增量式下同时起抗积分饱和作用） *)
  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;

  (* 回算：acc 恒 = 钳位后输出 - 前馈 → 钳位/ff 变化/手动/跟踪全程无扰 *)
  acc := out - ff;

  e_prev2 := e_prev;
  e_prev  := e;
END_FUNCTION_BLOCK
(*
  FB_ALARM_HL — 四级高低报警（H/L/HH/LL），带死区、延时与简化 ISA-18.2 确认锁存。

  用途：液位/温度/压力等模拟量的 高/低/高高/低低 报警。
  例：碳化塔液位 LT 报警，HH 联锁停进料。

  行为（每级相同，独立计时）：
    1) 原始越限判定（带死区回差）：
       高类：pv > sp 置位；pv < sp - deadband 复位。
       低类：pv < sp 置位；pv > sp + deadband 复位。
    2) 延时：越限条件连续保持 delay_s 后才触发（内部用 dt_s 手工累加计时，
       不使用 TON 实例，便于离线测试与移植）。
    3) 锁存（简化 ISA-18.2）：报警输出 = 触发条件 OR (已锁存 AND NOT ack)。
       即条件存在时报警常在；条件消失后仍保持，直到 ack 确认才清除。

  输入：
    pv        REAL  过程值
    h_sp      REAL  高报设定
    l_sp      REAL  低报设定
    hh_sp     REAL  高高报设定
    ll_sp     REAL  低低报设定
    deadband  REAL  死区（回差，工程单位）
    delay_s   REAL  报警延时 s（0 = 立即）
    dt_s      REAL  采样周期 s
    ack       BOOL  确认（电平有效）
  输出：
    h_alm / l_alm / hh_alm / ll_alm  BOOL  锁存后的报警输出
    any_alm                          BOOL  任一报警
    h_cond / l_cond / hh_cond / ll_cond BOOL 原始越限位（含死区，未延时未锁存）
*)
FUNCTION_BLOCK FB_ALARM_HL
  VAR_INPUT
    pv       : REAL;
    h_sp     : REAL := 80.0;
    l_sp     : REAL := 20.0;
    hh_sp    : REAL := 90.0;
    ll_sp    : REAL := 10.0;
    deadband : REAL := 0.0;
    delay_s  : REAL := 0.0;
    dt_s     : REAL := 0.1;
    ack      : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    h_alm   : BOOL;
    l_alm   : BOOL;
    hh_alm  : BOOL;
    ll_alm  : BOOL;
    any_alm : BOOL;
    h_cond  : BOOL;
    l_cond  : BOOL;
    hh_cond : BOOL;
    ll_cond : BOOL;
  END_VAR
  VAR
    h_t     : REAL; (* 各级延时累加器，手工计时替代 TON *)
    l_t     : REAL;
    hh_t    : REAL;
    ll_t    : REAL;
    h_trip  : BOOL;
    l_trip  : BOOL;
    hh_trip : BOOL;
    ll_trip : BOOL;
  END_VAR

  (* ---- 高报 H ---- *)
  IF pv > h_sp THEN
    h_cond := TRUE;
  ELSIF pv < h_sp - deadband THEN
    h_cond := FALSE;
  END_IF;
  IF h_cond THEN
    h_t := h_t + dt_s;
  ELSE
    h_t := 0.0;
  END_IF;
  h_trip := h_cond AND (h_t >= delay_s);
  h_alm  := h_trip OR (h_alm AND NOT ack);

  (* ---- 低报 L ---- *)
  IF pv < l_sp THEN
    l_cond := TRUE;
  ELSIF pv > l_sp + deadband THEN
    l_cond := FALSE;
  END_IF;
  IF l_cond THEN
    l_t := l_t + dt_s;
  ELSE
    l_t := 0.0;
  END_IF;
  l_trip := l_cond AND (l_t >= delay_s);
  l_alm  := l_trip OR (l_alm AND NOT ack);

  (* ---- 高高报 HH ---- *)
  IF pv > hh_sp THEN
    hh_cond := TRUE;
  ELSIF pv < hh_sp - deadband THEN
    hh_cond := FALSE;
  END_IF;
  IF hh_cond THEN
    hh_t := hh_t + dt_s;
  ELSE
    hh_t := 0.0;
  END_IF;
  hh_trip := hh_cond AND (hh_t >= delay_s);
  hh_alm  := hh_trip OR (hh_alm AND NOT ack);

  (* ---- 低低报 LL ---- *)
  IF pv < ll_sp THEN
    ll_cond := TRUE;
  ELSIF pv > ll_sp + deadband THEN
    ll_cond := FALSE;
  END_IF;
  IF ll_cond THEN
    ll_t := ll_t + dt_s;
  ELSE
    ll_t := 0.0;
  END_IF;
  ll_trip := ll_cond AND (ll_t >= delay_s);
  ll_alm  := ll_trip OR (ll_alm AND NOT ack);

  any_alm := h_alm OR l_alm OR hh_alm OR ll_alm;
END_FUNCTION_BLOCK
(*
  FB_PULSER — 周期脉冲发生器。

  用途：料仓空气炮/气锤周期吹扫、周期润滑、心跳输出。
  例：使能后每 period_s 秒输出一个 pulse_len_s 秒宽的脉冲（周期起点先出脉冲）。

  行为：
    enable 时相位 phase_s 以 dt_s 手工累加；phase_s < pulse_len_s 期间 q := TRUE，
    到 period_s 回卷（减法回卷，长期节拍不漂移）。
    NOT enable 或参数非法（period_s/dt_s <= 0）时 q := FALSE 且相位清零
    （重新使能从脉冲开始）。
    pulse_len_s >= period_s 时 q 恒为 TRUE（按定义连续输出）。

  输入：
    enable      BOOL  使能
    period_s    REAL  脉冲周期 s
    pulse_len_s REAL  脉冲宽度 s
    dt_s        REAL  采样周期 s
  输出：
    q BOOL  脉冲输出
*)
FUNCTION_BLOCK FB_PULSER
  VAR_INPUT
    enable      : BOOL;
    period_s    : REAL := 60.0;
    pulse_len_s : REAL := 1.0;
    dt_s        : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* 周期内相位 0..period_s，手工计时替代 TP/TON *)
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  (* 先按当前相位出脉冲（使能首周期立即为 TRUE），再推进相位 *)
  q := phase_s < pulse_len_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* 减法回卷，保持长期节拍 *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_HILO_FILL — 高位关、低位开的水箱/料仓补给控制。

  用途：液位低于低位开补给阀，高于高位关阀，之间保持当前状态
  （等效于低位接通方向的迟滞开关，专名封装便于组态阅读）。
  例：循环水箱补水：pv <= l_sp 开补水阀，pv >= h_sp 关阀。

  行为：
    pv <= l_sp → valve_open := TRUE（低位开）
    pv >= h_sp → valve_open := FALSE（高位关）
    l_sp < pv < h_sp → 保持。
  假定 l_sp < h_sp；若设反则高位判断优先（先开后即被关，输出为 FALSE）。

  输入：
    pv   REAL  液位/料位过程值
    h_sp REAL  高位设定（关阀）
    l_sp REAL  低位设定（开阀）
  输出：
    valve_open BOOL  补给阀输出（TRUE 开）
*)
FUNCTION_BLOCK FB_HILO_FILL
  VAR_INPUT
    pv   : REAL;
    h_sp : REAL := 85.0;
    l_sp : REAL := 15.0;
  END_VAR
  VAR_OUTPUT
    valve_open : BOOL;
  END_VAR

  IF pv >= h_sp THEN
    valve_open := FALSE; (* 高位关，优先 *)
  ELSIF pv <= l_sp THEN
    valve_open := TRUE;  (* 低位开 *)
  END_IF;
  (* 区间内保持 *)
END_FUNCTION_BLOCK
(*
  FB_RATIO — 比值站（比值给定器）。

  ≈ 比值站（ratio station）：配比/配料回路惯用块，输出接下游 PID 的 sp。

  用途：配比控制：从动流量设定值 = 主动（wild）流量 × 比值 + 偏置，
  sp 输出接下游 FB_PID 的 sp 输入构成比值回路。
  例：碳化塔石灰乳流量按进气量 0.85 配比跟随。

  公式：sp := clamp(ratio, ratio_min, ratio_max) * wild_flow + bias。
  说明：仅对 ratio 钳位（操作员可调比值的安全限幅），不对 sp 钳位——
  sp 量纲随 wild_flow，下游 PID 自有输出钳位。

  输入：
    wild_flow REAL  主动流量（不受本回路控制的"野"流量）
    ratio     REAL  比值
    ratio_min REAL  比值下限
    ratio_max REAL  比值上限
    bias      REAL  偏置（工程单位）
  输出：
    sp        REAL  从动回路设定值
*)
FUNCTION_BLOCK FB_RATIO
  VAR_INPUT
    wild_flow : REAL;
    ratio     : REAL := 1.0;
    ratio_min : REAL := 0.0;
    ratio_max : REAL := 10.0;
    bias      : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    sp : REAL;
  END_VAR
  VAR
    ratio_used : REAL;
  END_VAR

  ratio_used := ratio;
  IF ratio_used < ratio_min THEN
    ratio_used := ratio_min;
  ELSIF ratio_used > ratio_max THEN
    ratio_used := ratio_max;
  END_IF;

  sp := ratio_used * wild_flow + bias;
END_FUNCTION_BLOCK
(*
  FB_SPLIT_RANGE — 分程输出：一个调节器输出按 split 点分配到两个阀。

  ≈ 分程输出（split range）：一个调节输出按区段驱动两个阀（加热/冷却、大小阀）。

  用途：一个 PID 驱动两个执行机构：大小阀扩量程（顺序分程，
  reverse_b := FALSE），或加热/冷却、进料/回流互补型分程
  （reverse_b := TRUE，按执行机构气开气关与 PID 作用方向整定）。

  行为（u 先钳位 0..100，两输出亦钳位 0..100）：
    out_a：u 在 [0, split] 内 0→100 线性，u >= split 后保持 100。
    out_b：u 在 [split, 100] 内 0→100 线性，u <= split 时为 0；
           reverse_b = TRUE 时 B 段反转为 100→0（含段外端值保持，
           即 u <= split 时 out_b = 100，输出连续无跳变）。
    曲线示意（split = 50）：
      reverse_b = FALSE（大小阀顺序分程）：
        u:     0 …… 50 …… 100
        out_a: 0 → 100 ┤ 保持 100
        out_b: 0   0   ├ 0 → 100
      reverse_b = TRUE（互补型，B 先全开后渐关）：
        u:     0 …… 50 …… 100
        out_a: 0 → 100 ┤ 保持 100
        out_b: 100 100 ├ 100 → 0
    退化：split <= 0 → A 段无量程（u > 0 即 100）；split >= 100 → B 段
    无量程（u >= 100 才动作）。split 应整定在 (0, 100) 内。

  输入：
    u         REAL  调节器输出 0..100
    split     REAL  分程点 %（B 阀起点）
    reverse_b BOOL  TRUE = B 阀在其分程段内反作用（100→0）
  输出：
    out_a     REAL  A 阀开度 0..100
    out_b     REAL  B 阀开度 0..100
*)
FUNCTION_BLOCK FB_SPLIT_RANGE
  VAR_INPUT
    u         : REAL;
    split     : REAL := 50.0;
    reverse_b : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out_a : REAL;
    out_b : REAL;
  END_VAR
  VAR
    uc     : REAL; (* 钳位后的输入 *)
    span_b : REAL;
  END_VAR

  (* 输入钳位 0..100 *)
  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  (* A 阀：[0, split] 内 0→100 *)
  IF split > 0.0 THEN
    out_a := uc * 100.0 / split;
  ELSE
    (* 退化：A 无分程段 *)
    IF uc > 0.0 THEN
      out_a := 100.0;
    ELSE
      out_a := 0.0;
    END_IF;
  END_IF;
  IF out_a < 0.0 THEN
    out_a := 0.0;
  ELSIF out_a > 100.0 THEN
    out_a := 100.0;
  END_IF;

  (* B 阀：[split, 100] 内 0→100 *)
  span_b := 100.0 - split;
  IF span_b > 0.0 THEN
    out_b := (uc - split) * 100.0 / span_b;
  ELSE
    (* 退化：B 无分程段 *)
    IF uc >= split THEN
      out_b := 100.0;
    ELSE
      out_b := 0.0;
    END_IF;
  END_IF;
  IF out_b < 0.0 THEN
    out_b := 0.0;
  ELSIF out_b > 100.0 THEN
    out_b := 100.0;
  END_IF;

  (* B 段反作用（钳位后再反转，段外保持端值，输出连续） *)
  IF reverse_b THEN
    out_b := 100.0 - out_b;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_RUNTIME — 设备运行小时 + 启动次数统计。

  ≈ 设备运行统计（累计运行小时 + 启动次数），维护保养数据标配。

  用途：泵/风机磨损均衡（hours 接 FB_DUTY2 的 hours_a/b，配 alt_mode=1）、
  维保到期提醒、启停频度统计。
  行为：
    running 为 TRUE 期间 hours := hours + dt_s/3600；
    running 上升沿 starts := starts + 1；
    reset 电平有效，两者清零（优先，复位周期不累计）。
    实例可由调用方放 VAR RETAIN 块下断电保持（同 FB_TOTALIZER 注意事项：
    IA2 retain 按 i32 存取，REAL 小时数的保持精度受此限制；starts 为
    整数计数不受影响）。

  输入：
    running BOOL  设备运行中（接运行反馈，必要时前置 FB_DEBOUNCE）
    reset   BOOL  清零（电平有效，优先；检修换泵后用）
    dt_s    REAL  采样周期 s
  输出：
    hours  REAL  累计运行小时
    starts DINT  累计启动次数
*)
FUNCTION_BLOCK FB_RUNTIME
  VAR_INPUT
    running : BOOL;
    reset   : BOOL := FALSE;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    hours  : REAL;
    starts : DINT;
  END_VAR
  VAR
    prev_run : BOOL; (* 上升沿记忆 *)
  END_VAR

  IF reset THEN
    hours := 0.0;
    starts := 0;
  ELSE
    IF running AND NOT prev_run THEN
      starts := starts + 1;
    END_IF;
    IF running AND dt_s > 0.0 THEN
      hours := hours + dt_s / 3600.0;
    END_IF;
  END_IF;
  prev_run := running;
END_FUNCTION_BLOCK
(*
  FB_DUTY2 — 双泵值班/备用（duty/standby）轮换控制。

  ≈ 双泵一用一备（duty/standby）轮换：按启动次序或运行小时均衡选值班泵，故障即时顶替。

  用途：两台泵一用一备：每次启动轮换或按累计运行小时均衡选值班泵；
  值班泵故障备用泵立即顶上；双故障全停。
  行为（扫描确定性：无内部计时，仅沿记忆）：
    demand 上升沿选定本次值班泵，优先级：
      1) 恰有一台已在运转（run_fb 单边 TRUE）→ 直接采纳其为值班
         （无扰接管，避免停转再起备用泵）；
      2) alt_mode = 1 → 累计小时小者值班（hours_a/b 接两个 FB_RUNTIME）；
      3) alt_mode = 0（默认）→ 在当前值班位上翻转（含故障顶替后的状态，
         天然磨损均衡）。
    demand 保持期间：值班泵故障且另一台健康 → 值班立即切换（顶替）；
    被命令泵故障即不出指令；双故障 → cmd 双 FALSE（值班选择保持）。
    demand 撤销 → cmd 双 FALSE。
    接线建议：fault_x 接 FB_MOTOR 的 fault 或电气综保；run_fb_x 接运行
    反馈（抖动触点前置 FB_DEBOUNCE）；反馈超时判故障交给 FB_MOTOR，
    本块不重复计时。

  输入：
    demand   BOOL  系统需泵（电平）
    alt_mode INT   0 = 每次启动轮换 / 1 = 运行小时均衡
    run_fb_a BOOL  A 泵运行反馈
    run_fb_b BOOL  B 泵运行反馈
    fault_a  BOOL  A 泵故障
    fault_b  BOOL  B 泵故障
    hours_a  REAL  A 泵累计小时（alt_mode=1 用，接 FB_RUNTIME.hours）
    hours_b  REAL  B 泵累计小时（alt_mode=1 用）
    dt_s     REAL  采样周期 s（本块当前不计时，保留以统一库接口）
  输出：
    cmd_a     BOOL  A 泵启动指令
    cmd_b     BOOL  B 泵启动指令
    duty_is_a BOOL  当前值班泵为 A
*)
FUNCTION_BLOCK FB_DUTY2
  VAR_INPUT
    demand   : BOOL;
    alt_mode : INT := 0;
    run_fb_a : BOOL := FALSE;
    run_fb_b : BOOL := FALSE;
    fault_a  : BOOL := FALSE;
    fault_b  : BOOL := FALSE;
    hours_a  : REAL := 0.0;
    hours_b  : REAL := 0.0;
    dt_s     : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    cmd_a     : BOOL;
    cmd_b     : BOOL;
    duty_is_a : BOOL;
  END_VAR
  VAR
    prev_demand : BOOL; (* 上升沿记忆 *)
    rising      : BOOL;
  END_VAR

  rising := demand AND NOT prev_demand;
  prev_demand := demand;

  IF rising THEN
    IF run_fb_a AND NOT run_fb_b THEN
      duty_is_a := TRUE;  (* 已在转的泵直接作值班，无扰接管 *)
    ELSIF run_fb_b AND NOT run_fb_a THEN
      duty_is_a := FALSE;
    ELSIF alt_mode = 1 THEN
      duty_is_a := hours_a <= hours_b; (* 小时均衡：少者值班 *)
    ELSE
      duty_is_a := NOT duty_is_a; (* 每次启动轮换（初值 FALSE → 首启选 A） *)
    END_IF;
  END_IF;

  IF demand THEN
    (* 故障顶替：值班故障且另一台健康 → 值班立即切换 *)
    IF duty_is_a AND fault_a AND NOT fault_b THEN
      duty_is_a := FALSE;
    ELSIF NOT duty_is_a AND fault_b AND NOT fault_a THEN
      duty_is_a := TRUE;
    END_IF;
    cmd_a := duty_is_a AND NOT fault_a;
    cmd_b := NOT duty_is_a AND NOT fault_b;
  ELSE
    cmd_a := FALSE;
    cmd_b := FALSE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_INTERLOCK8 — 8 路联锁汇总 + 首出（first-out）记录。

  ≈ 联锁汇总 + 首出捕捉（first-out annunciation，参考 ISA-18.1 首出窗概念）。

  用途：机组联锁跳车原因追溯：多条件汇总成一个跳闸位，并锁存最先
  动作的那一路，事后定位跳车根因（首出之后的连锁性动作不覆盖记录）。
  行为：
    有效条件 ax := cx AND enablex（enablex 默认 TRUE；置 FALSE = 旁路该路）。
    tripped := 任一有效条件 OR (已锁存 AND NOT reset)
    ——条件在则常报；条件全清后保持，直到 reset（等效"reset 且全清"
    才能复位；条件未清时按 reset 无效）。
    首出捕获：tripped 由 FALSE→TRUE 的那一拍记录当时动作的最小路号
    （同拍多路同动取小号）。联锁保持期间后续动作不改写首出；
    复位成功（tripped 回 FALSE）时 first_out 清 0。
    cx 语义：TRUE = 该条件已动作（跳闸方向）。常闭型信号请在外部取反。

  输入：
    c1..c8           BOOL  联锁条件（TRUE = 动作/越限）
    enable1..enable8 BOOL  各路投用（默认 TRUE；FALSE = 旁路）
    reset            BOOL  复位（电平有效；仅条件全清后生效）
  输出：
    tripped   BOOL  联锁动作（锁存）
    first_out INT   首出路号 1..8，0 = 无
*)
FUNCTION_BLOCK FB_INTERLOCK8
  VAR_INPUT
    c1      : BOOL := FALSE;
    c2      : BOOL := FALSE;
    c3      : BOOL := FALSE;
    c4      : BOOL := FALSE;
    c5      : BOOL := FALSE;
    c6      : BOOL := FALSE;
    c7      : BOOL := FALSE;
    c8      : BOOL := FALSE;
    enable1 : BOOL := TRUE;
    enable2 : BOOL := TRUE;
    enable3 : BOOL := TRUE;
    enable4 : BOOL := TRUE;
    enable5 : BOOL := TRUE;
    enable6 : BOOL := TRUE;
    enable7 : BOOL := TRUE;
    enable8 : BOOL := TRUE;
    reset   : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    tripped   : BOOL;
    first_out : INT;
  END_VAR
  VAR
    a1    : BOOL; (* 投用后的有效条件 *)
    a2    : BOOL;
    a3    : BOOL;
    a4    : BOOL;
    a5    : BOOL;
    a6    : BOOL;
    a7    : BOOL;
    a8    : BOOL;
    any_c : BOOL;
    was   : BOOL; (* 上一拍 tripped，捕获首出沿 *)
  END_VAR

  a1 := c1 AND enable1;
  a2 := c2 AND enable2;
  a3 := c3 AND enable3;
  a4 := c4 AND enable4;
  a5 := c5 AND enable5;
  a6 := c6 AND enable6;
  a7 := c7 AND enable7;
  a8 := c8 AND enable8;
  any_c := a1 OR a2 OR a3 OR a4 OR a5 OR a6 OR a7 OR a8;

  was := tripped;
  tripped := any_c OR (tripped AND NOT reset);

  IF tripped AND NOT was THEN
    (* 新跳闸事件：记录首出（同拍多路取最小号） *)
    IF a1 THEN
      first_out := 1;
    ELSIF a2 THEN
      first_out := 2;
    ELSIF a3 THEN
      first_out := 3;
    ELSIF a4 THEN
      first_out := 4;
    ELSIF a5 THEN
      first_out := 5;
    ELSIF a6 THEN
      first_out := 6;
    ELSIF a7 THEN
      first_out := 7;
    ELSIF a8 THEN
      first_out := 8;
    END_IF;
  ELSIF NOT tripped THEN
    first_out := 0; (* 复位成功清首出 *)
  END_IF;
END_FUNCTION_BLOCK

(*
  FB_LIMIT — 值限幅器（高低钳位 + 越限指示）。

  ≈ 标准限幅器（limiter / high-low clamp），各主流平台标准库必备。

  用途：设定值/输出值安全钳位、串级外环输出限制、操作员输入合法化、
  下游裸执行器的最后一道保护。
  行为（纯组合逻辑，无内部状态）：
    out := u 钳位到 [lo, hi]；q_hi / q_lo 指示正在钳位（u 越上/下限）。
    组态错误 lo > hi → out := (lo + hi) / 2，且 q_hi 与 q_lo 同时为 TRUE
    ——双越限位同亮在物理上不可能出现，专作组态错误信号，便于上位
    报警捕捉。

  输入：
    u  REAL  输入
    lo REAL  下限
    hi REAL  上限（假定 lo <= hi；设反触发组态错误信号）
  输出：
    out  REAL  钳位输出
    q_hi BOOL  越上限（正在按 hi 钳位）
    q_lo BOOL  越下限（正在按 lo 钳位；与 q_hi 同时 TRUE = lo > hi 组态错误）
*)
FUNCTION_BLOCK FB_LIMIT
  VAR_INPUT
    u  : REAL;
    lo : REAL := 0.0;
    hi : REAL := 100.0;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF lo > hi THEN
    (* 组态错误：取中值输出，双越限位同亮作错误信号 *)
    out := (lo + hi) / 2.0;
    q_hi := TRUE;
    q_lo := TRUE;
    RETURN;
  END_IF;

  IF u > hi THEN
    out := hi;
    q_hi := TRUE;
    q_lo := FALSE;
  ELSIF u < lo THEN
    out := lo;
    q_hi := FALSE;
    q_lo := TRUE;
  ELSE
    out := u;
    q_hi := FALSE;
    q_lo := FALSE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_STATS — 累计统计：最小/最大/均值/标准差（Welford 单遍递推）。

  ≈ 标准统计块（min/max/average/standard deviation），各主流平台
  标准库均有同类块；本块用 Welford 单遍递推，无数组、数值稳定。

  用途：质量数据统计（批次温度均值/波动）、测量噪声水平评估、
  调试期信号摸底（接 FB_AVG_TIME 看时间平均，接本块看全程分布）。
  行为（按"样本"计，不吃 dt_s：每个 enable 为 TRUE 的扫描拍计 1 个样本）：
    Welford 递推（标量，单遍，避免大数平方相减的精度灾难）：
      n := n + 1；delta := u - avg；avg := avg + delta / n；
      m2 := m2 + delta * (u - avg)
    sd := SQRT(m2 / n) —— 总体标准差（除 n 而非 n-1）：过程监视看的是
    波动幅度，样本数大时两者无差，且单遍递推下除 n 形式最直接。
    reset 电平有效：清零并在下一个使能样本重新播种（vmin/vmax/avg 取
    该样本值）。
    注意：n 为 REAL 计数，单精度下计到约 1.6E7 个样本（100 ms 采样约
    19 天）后 +1 失效、统计冻结——长期运行请按班/按天 reset。

  输入：
    u      REAL  输入样本
    enable BOOL  采样使能（FALSE 拍不计样本，输出保持）
    reset  BOOL  清零（电平有效，优先；下一使能样本重新播种）
  输出：
    vmin REAL  最小值
    vmax REAL  最大值
    avg  REAL  算术均值
    sd   REAL  总体标准差
    n    REAL  已计样本数
*)
FUNCTION_BLOCK FB_STATS
  VAR_INPUT
    u      : REAL;
    enable : BOOL := TRUE;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    vmin : REAL;
    vmax : REAL;
    avg  : REAL;
    sd   : REAL;
    n    : REAL;
  END_VAR
  VAR
    m2     : REAL; (* Welford 二阶累计量 Σ(u-avg)·(u-avg') *)
    delta  : REAL;
    delta2 : REAL;
  END_VAR

  IF reset THEN
    n := 0.0;
    avg := 0.0;
    m2 := 0.0;
    sd := 0.0;
    vmin := 0.0;
    vmax := 0.0;
    RETURN;
  END_IF;

  IF NOT enable THEN
    RETURN; (* 不计样本：输出保持 *)
  END_IF;

  IF n <= 0.0 THEN
    (* 首样本播种 *)
    n := 1.0;
    avg := u;
    m2 := 0.0;
    sd := 0.0;
    vmin := u;
    vmax := u;
    RETURN;
  END_IF;

  n := n + 1.0;
  delta := u - avg;
  avg := avg + delta / n;
  delta2 := u - avg;
  m2 := m2 + delta * delta2;
  sd := SQRT(m2 / n);

  IF u < vmin THEN
    vmin := u;
  END_IF;
  IF u > vmax THEN
    vmax := u;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_AVG_TIME — 翻转窗时间平均（tumbling-window time average）。

  ≈ 标准定时平均块（time average），DCS 报表/平稳量监视的惯用形态。

  用途：报表用整窗平均（如每分钟平均流量）、慢变量的抗噪取值、
  与瞬时值比较做漂移监视。
  行为：
    窗内逐拍累加 sum / count；avg_run := sum / count 为当前窗的滚动均值；
    窗满（累计时间达 window_s）时把 avg_run 拷入 avg、置 ready，随后
    清空累加器开下一窗（相位减法回卷，长期节拍不漂移）。
    首窗未满期间 avg 暂跟 avg_run（避免投运期输出 0），ready = FALSE
    标示"尚非完整窗结果"；首窗完成后 ready 恒 TRUE，avg 仅在窗满拍刷新。
    reset 电平有效：清空累加器与 ready，重新开窗。
    window_s <= 0 → 直通（avg := avg_run := u，ready := TRUE）；
    dt_s <= 0 → 保持不动作。
    注意：本块是翻转窗（每窗出一个值），不是滑动窗——真滑动窗需要
    数组缓存（本方言 FB 作用域数组不能 codegen，见 README 坑 7）；
    要"每拍都平滑"的连续输出请用 FB_LAG 指数平滑近似。

  输入：
    u        REAL  输入
    window_s REAL  平均窗长 s（<= 0 直通）
    dt_s     REAL  采样周期 s
    reset    BOOL  复位（电平有效：清窗重开）
  输出：
    avg     REAL  上一完整窗的平均（首窗未满暂跟 avg_run）
    avg_run REAL  当前窗的滚动均值
    ready   BOOL  首个完整窗已完成（avg 可信）
*)
FUNCTION_BLOCK FB_AVG_TIME
  VAR_INPUT
    u        : REAL;
    window_s : REAL := 60.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    avg     : REAL;
    avg_run : REAL;
    ready   : BOOL;
  END_VAR
  VAR
    sum       : REAL; (* 当前窗累加和 *)
    count     : REAL; (* 当前窗样本数 *)
    elapsed_s : REAL; (* 当前窗累计时间，手工计时 *)
  END_VAR

  IF reset THEN
    sum := 0.0;
    count := 0.0;
    elapsed_s := 0.0;
    avg := 0.0;
    avg_run := 0.0;
    ready := FALSE;
    RETURN;
  END_IF;

  IF window_s <= 0.0 THEN
    avg := u; (* 窗长非法：直通 *)
    avg_run := u;
    ready := TRUE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  sum := sum + u;
  count := count + 1.0;
  elapsed_s := elapsed_s + dt_s;
  avg_run := sum / count;

  IF elapsed_s >= window_s THEN
    (* 窗满：拷贝结果，清空开下一窗（减法回卷保持长期节拍） *)
    avg := avg_run;
    ready := TRUE;
    sum := 0.0;
    count := 0.0;
    elapsed_s := elapsed_s - window_s;
  ELSIF NOT ready THEN
    avg := avg_run; (* 首窗未满：avg 暂跟滚动均值，避免投运期输出 0 *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_INTEG — 通用积分器（带保持/复位/置数与输出钳位）。

  ≈ 标准积分块（integrator），各主流平台标准库必备。

  用途：偏差积分（℃·s 之类的累计量监视）、简单物料/能量累计、
  对象仿真中的积分环节、自定义控制算法的积分通路。
  与 FB_TOTALIZER 的区别：TOTALIZER 是固定 /3600 的流量累积专用块；
  本块为通用积分 out := out + k * u * dt_s，带 hold/preset/钳位。
  行为（命令优先级 reset > preset > hold > 积分）：
    reset  电平有效：out := 0，越限位清零；
    preset 电平有效：out := preset_value（随后照常钳位）；
    hold   电平有效：暂停积分，输出保持（钳位与越限位照常刷新）；
    其余：out := out + k * u * dt_s，钳位到 [out_min, out_max]。
    q_hi / q_lo 指示输出贴上/下限（抗饱和：贴限后反向输入立即退出）。
    dt_s <= 0 → 本拍保持不动作（reset/preset 为电平命令，不受此限）。

  输入：
    u            REAL  被积输入
    k            REAL  积分增益（1/s 量纲由调用方定义）
    dt_s         REAL  采样周期 s
    hold         BOOL  暂停积分（电平有效）
    reset        BOOL  复位到 0（电平有效，最高优先）
    preset       BOOL  置数（电平有效，优先于 hold）
    preset_value REAL  置数值
    out_min      REAL  输出下限（默认 -1.0E38 ≈ 不限）
    out_max      REAL  输出上限（默认 +1.0E38 ≈ 不限）
  输出：
    out  REAL  积分输出（钳位）
    q_hi BOOL  贴上限
    q_lo BOOL  贴下限
*)
FUNCTION_BLOCK FB_INTEG
  VAR_INPUT
    u            : REAL;
    k            : REAL := 1.0;
    dt_s         : REAL := 0.1;
    hold         : BOOL := FALSE;
    reset        : BOOL := FALSE;
    preset       : BOOL := FALSE;
    preset_value : REAL := 0.0;
    out_min      : REAL := -1.0E38;
    out_max      : REAL := 1.0E38;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF reset THEN
    out := 0.0;
    q_hi := FALSE;
    q_lo := FALSE;
    RETURN;
  END_IF;

  IF preset THEN
    out := preset_value; (* 置数后照常走下方钳位 *)
  ELSIF NOT hold THEN
    IF dt_s <= 0.0 THEN
      RETURN; (* 采样周期非法：保持 *)
    END_IF;
    out := out + k * u * dt_s;
  END_IF;
  (* hold 电平期间：不积分，仅照常刷新钳位与越限位 *)

  q_hi := FALSE;
  q_lo := FALSE;
  IF out >= out_max THEN
    out := out_max;
    q_hi := TRUE;
  ELSIF out <= out_min THEN
    out := out_min;
    q_lo := TRUE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_DERIV — 滤波微分器（du/dt，先滤波后差分）。

  ≈ 标准微分块（filtered derivative），各主流平台标准库必备。

  用途：变化趋势量化（℃/s、%/s 供显示/前馈/自定义逻辑用）、
  前馈通路的微分整形。只要"超限即报警"请直接用 FB_ALARM_ROC
  （同一滤波-差分内核 + 延时/锁存）。
  行为：
    u 先经内部 PT1 滤波（时间常数 t_filt_s；<= 0 时自动取 3*dt_s，
    约 4 拍收敛），再对滤波值做后向差分：
      u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f)
      out := (u_f - u_f_prev) / dt_s
    单拍差分对量化/噪声极敏感，必须滤波否则输出全是毛刺——需要更强
    去噪时加大 t_filt_s 或给 u 前置 FB_LAG。
    reset / 首扫描：滤波状态对中到 u，out := 0（不出投运假导数）。
    dt_s <= 0 → 保持不动作。

  输入：
    u        REAL  输入
    t_filt_s REAL  微分前置滤波时间常数 s（<= 0 = 自动 3*dt_s）
    dt_s     REAL  采样周期 s
    reset    BOOL  复位/对中（电平有效：滤波态跟随 u，out 清零）
  输出：
    out REAL  滤波后的变化率 单位/s（带符号）
*)
FUNCTION_BLOCK FB_DERIV
  VAR_INPUT
    u        : REAL;
    t_filt_s : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    u_f       : REAL; (* PT1 滤波后的输入 *)
    u_f_prev  : REAL; (* 上一拍滤波值，后向差分用 *)
    t_f       : REAL; (* 实际使用的滤波时间常数 *)
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  IF reset OR NOT init_done THEN
    u_f := u; (* 滤波态对中，不出投运假导数 *)
    u_f_prev := u;
    out := 0.0;
    init_done := TRUE;
    RETURN;
  END_IF;

  t_f := t_filt_s;
  IF t_f <= 0.0 THEN
    t_f := 3.0 * dt_s; (* 自动：约 4 拍收敛的最低限度去噪 *)
  END_IF;

  u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f);
  out := (u_f - u_f_prev) / dt_s;
  u_f_prev := u_f;
END_FUNCTION_BLOCK
(*
  FB_DEADTIME8 — 纯滞后（dead time / transport delay），8 标量抽头 + 线性插值。

  ≈ 标准纯滞后块（dead time / PTt / transport delay），对象仿真与
  前馈动态补偿（Smith 预估类结构）的惯用块。

  用途：对象仿真（与 FB_LAG 串联即组成一阶惯性+纯滞后 FOPDT 对象）、
  前馈通路的传输延迟匹配、皮带/管道输送类测量的时间对齐。
  行为：
    抽头链 z1..z8（z1 最新、z8 最老），抽样周期 = t_delay_s / 8：
    相位 phase_s 以 dt_s 手工累加，每经过一个抽样周期整链移位一次
    （z8←z7←…←z1←u）；一拍最多移 8 次（8 次后整链已被 u 充满，
    WHILE 有界、扫描确定性）。
    输出在最老两个抽头间按相位分数线性插值：
      out := z8 + (z7 - z8) * phase_s / period_s
    ——阶跃输入以一个抽样周期的斜坡渡过，输出连续无台阶。
    分辨率 = t_delay_s / 8：8 抽头是无数组方言下的折衷，延迟时间的
    阶梯/插值误差约为延迟的 1/8——回路整定够用；要更细分辨率请串联
    两级本块（各承担一半延迟）。
    t_delay_s <= 0 → 直通（out := u），抽头随 u 充满——延迟在线投用
    的那一刻从当前值平滑起步（无扰）。
    reset / 首扫描：全部抽头充 u，out := u，相位清零。
    dt_s <= 0 → 保持不动作。

  输入：
    u         REAL  输入
    t_delay_s REAL  纯滞后时间 s（<= 0 直通；在线可改，按新周期推进）
    dt_s      REAL  采样周期 s
    reset     BOOL  复位（电平有效：抽头充 u，输出跟随 u）
  输出：
    out REAL  延迟后的输出（线性插值，连续）
*)
FUNCTION_BLOCK FB_DEADTIME8
  VAR_INPUT
    u         : REAL;
    t_delay_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
    reset     : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    z1        : REAL; (* 抽头链：z1 最新 … z8 最老（无数组方言的标量形式） *)
    z2        : REAL;
    z3        : REAL;
    z4        : REAL;
    z5        : REAL;
    z6        : REAL;
    z7        : REAL;
    z8        : REAL;
    phase_s   : REAL; (* 当前抽样周期内相位，手工计时 *)
    period_s  : REAL; (* 抽样周期 = t_delay_s / 8 *)
    shifts    : REAL; (* 本拍移位计数（封顶 8：整链已饱和） *)
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF reset OR NOT init_done THEN
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_delay_s <= 0.0 THEN
    (* 直通；抽头随 u 充满，延迟在线投用时从当前值无扰起步 *)
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  period_s := t_delay_s / 8.0;
  phase_s := phase_s + dt_s;

  (* 每经过一个抽样周期整链移位一次；封顶 8 次/拍（dt_s 跨多个周期时
     补移，8 次后整链已被 u 充满，再移无意义）——WHILE 有界 *)
  shifts := 0.0;
  WHILE (phase_s >= period_s) AND (shifts < 8.0) DO
    z8 := z7;
    z7 := z6;
    z6 := z5;
    z5 := z4;
    z4 := z3;
    z3 := z2;
    z2 := z1;
    z1 := u;
    phase_s := phase_s - period_s;
    shifts := shifts + 1.0;
  END_WHILE;
  IF phase_s >= period_s THEN
    phase_s := 0.0; (* 退化情形 dt_s >= t_delay_s：整链已是 u，相位清零防积累 *)
  END_IF;

  (* 最老两抽头间按相位分数插值：移位瞬间 frac=0 → out=z8(旧 z7)，连续 *)
  out := z8 + (z7 - z8) * phase_s / period_s;
END_FUNCTION_BLOCK
(*
  FB_MID3 — 三取中表决（median select / 2oo3 模拟量表决）。

  ≈ 标准三取中/冗余测量表决块（median selector，2oo3 voting），
  冗余变送器信号选择的惯用块。

  用途：三冗余变送器取中（剔除单点漂移/跳变）、双冗余取均、
  关键测量的坏值降级处理（配 FB_SCALE 的 fault 作 good_x）。
  行为（按好通道数 n_good 降级）：
    3 好 → 取中值（天然剔除单通道漂移）；
    2 好 → 取两好通道均值；
    1 好 → 直通该通道；
    0 好 → out 保持上一拍值，q_fail = TRUE（电平，非锁存）。
    偏差监视：dev_max > 0 时，任意两个好通道之差 > dev_max → q_dev
    （提示冗余通道间漂移拉大，该校验变送器了；dev_max <= 0 关闭监视）。
    good_x 接坏值判定（变送器断线/超量程，如 FB_SCALE.fault 取反），
    默认 TRUE = 全好。
    注意：首拍即 0 好时 out 为初值 0（无历史可保持）；q_fail 期间的
    保持值是"最后一次有好通道时的表决值"。

  输入：
    a, b, c                REAL  三路冗余测量
    good_a, good_b, good_c BOOL  各路有效（默认 TRUE；FALSE = 剔除）
    dev_max                REAL  好通道间偏差监视限值（<= 0 = 关闭）
  输出：
    out    REAL  表决输出（0 好时保持）
    n_good REAL  当前好通道数 0..3
    q_dev  BOOL  好通道间偏差超限
    q_fail BOOL  全部通道失效（输出处于保持）
*)
FUNCTION_BLOCK FB_MID3
  VAR_INPUT
    a       : REAL;
    b       : REAL;
    c       : REAL;
    good_a  : BOOL := TRUE;
    good_b  : BOOL := TRUE;
    good_c  : BOOL := TRUE;
    dev_max : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    n_good : REAL;
    q_dev  : BOOL;
    q_fail : BOOL;
  END_VAR

  n_good := 0.0;
  IF good_a THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_b THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_c THEN
    n_good := n_good + 1.0;
  END_IF;

  q_fail := n_good <= 0.0;

  (* 偏差监视：任意两个好通道差超 dev_max（内联 IF 替代 ABS，库惯例） *)
  q_dev := FALSE;
  IF dev_max > 0.0 THEN
    IF good_a AND good_b THEN
      IF (a - b > dev_max) OR (b - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_a AND good_c THEN
      IF (a - c > dev_max) OR (c - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_b AND good_c THEN
      IF (b - c > dev_max) OR (c - b > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
  END_IF;

  IF n_good >= 3.0 THEN
    (* 三好：取中值 *)
    IF (a >= b AND a <= c) OR (a <= b AND a >= c) THEN
      out := a;
    ELSIF (b >= a AND b <= c) OR (b <= a AND b >= c) THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  ELSIF n_good >= 2.0 THEN
    (* 两好：取均值 *)
    IF NOT good_a THEN
      out := (b + c) / 2.0;
    ELSIF NOT good_b THEN
      out := (a + c) / 2.0;
    ELSE
      out := (a + b) / 2.0;
    END_IF;
  ELSIF n_good >= 1.0 THEN
    (* 一好：直通 *)
    IF good_a THEN
      out := a;
    ELSIF good_b THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  END_IF;
  (* 0 好：out 保持上一拍值，q_fail 提示 *)
END_FUNCTION_BLOCK
(*
  FB_CASCADE — 级联耦合胶水（主回路输出 → 副回路 SP 映射 + 非级联跟踪回算）。

  ≈ 标准级联耦合块（cascade SP coupling / 主回路跟踪回算），
  串级回路组态的惯用胶水：两台 PID 之间只差这一块。

  用途：串级回路组态（温度主回路 → 流量副回路之类），把主调输出
  0..100% 映射到副回路 SP 工程量程，并在非级联时给主调供跟踪值，
  保证 CAS 投/切无扰。
  行为：
    CAS（cas_on = TRUE）：
      slave_sp := sp_min + master_out / 100 * (sp_max - sp_min)
      master_track := FALSE（主调正常调节）。
    非 CAS：
      slave_sp := slave_sp_local（操作员直接给副回路的本地 SP）；
      master_track := TRUE，master_track_value :=
        (slave_sp_now - sp_min) / (sp_max - sp_min) * 100，钳位 0..100
      ——主调输出被强制为"当前副 SP 的等效百分比"，切回 CAS 的那一拍
      slave_sp 不跳变（无扰切换）。
      量程未组态（|sp_max - sp_min| < 1E-6）→ 跟踪值取 0（防除零）。
    跟踪值每拍都计算（与模式无关），CAS 切除瞬间即有效。
    纯组合逻辑，无内部状态。

  接线（与两台 FB_PID 组级联；slave_sp_now 喂上一拍工作 SP）：
    casc(master_out := pid_master.out, cas_on := cas_enable,
         sp_min := 0.0, sp_max := 2000.0,
         slave_sp_local := op_sp, slave_sp_now := slave_sp_used);
    slave_sp_used := casc.slave_sp;
    pid_master(pv := temp_pv, sp := temp_sp, ..., dt_s := scan_s,
               track := casc.master_track,
               track_value := casc.master_track_value);
    pid_slave(pv := flow_pv, sp := slave_sp_used, ..., dt_s := scan_s);

  输入：
    master_out     REAL  主调输出 0..100 %
    cas_on         BOOL  级联投入（FALSE = 副回路用本地 SP，主调跟踪）
    sp_min         REAL  副回路 SP 量程下限（工程单位）
    sp_max         REAL  副回路 SP 量程上限
    slave_sp_local REAL  非级联时的副回路本地 SP
    slave_sp_now   REAL  副回路当前工作 SP（喂上一拍 slave_sp，跟踪回算用）
  输出：
    slave_sp           REAL  副回路设定值（接 pid_slave.sp）
    master_track       BOOL  主调跟踪命令（接 pid_master.track）
    master_track_value REAL  主调跟踪值 0..100 %（接 pid_master.track_value）
*)
FUNCTION_BLOCK FB_CASCADE
  VAR_INPUT
    master_out     : REAL;
    cas_on         : BOOL := FALSE;
    sp_min         : REAL := 0.0;
    sp_max         : REAL := 100.0;
    slave_sp_local : REAL := 0.0;
    slave_sp_now   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    slave_sp           : REAL;
    master_track       : BOOL;
    master_track_value : REAL;
  END_VAR
  VAR
    span : REAL; (* 副回路 SP 量程跨度 *)
  END_VAR

  span := sp_max - sp_min;

  (* 跟踪值每拍计算：当前副 SP 的等效百分比（量程未组态取 0，防除零） *)
  IF (span < 0.000001) AND (span > -0.000001) THEN
    master_track_value := 0.0;
  ELSE
    master_track_value := (slave_sp_now - sp_min) / span * 100.0;
    IF master_track_value < 0.0 THEN
      master_track_value := 0.0;
    ELSIF master_track_value > 100.0 THEN
      master_track_value := 100.0;
    END_IF;
  END_IF;

  IF cas_on THEN
    slave_sp := sp_min + master_out / 100.0 * span;
    master_track := FALSE;
  ELSE
    slave_sp := slave_sp_local;
    master_track := TRUE;
  END_IF;
END_FUNCTION_BLOCK

(*
  FB_PV_HOLD — 坏值保持/替代（bad-PV hold / substitute）。

  ≈ 标准坏值处理块（last-good hold / substitute value，含坏值
  超时降级），测量信号进控制回路前的标准防线，各主流 DCS/PLC
  的输入处理均有同类机制。

  用途：变送器断线/超量程（如 FB_SCALE.fault、FB_MID3.q_fail 取反
  作 good）时给下游回路一个可控的 PV：先保持最后好值渡过抖动，
  超时后按组态保持/替代/跟随，并出 q_bad/q_timeout 供报警与
  回路切手动逻辑使用。

  行为：
    good = TRUE：out := u，记忆最后好值，q_bad/q_timeout 清零，
    坏值计时清零。
    good = FALSE：q_bad := TRUE，坏值计时累加（手工 dt_s）：
      未到 timeout_s：out := 最后好值（短时抖动按保持渡过）；
      到 timeout_s（timeout_s = 0 立即）：q_timeout := TRUE，按 mode：
        mode = 0：继续保持最后好值；
        mode = 1：out := sub_value（替代值，如安全侧设定）；
        mode = 2：out := u（跟随原值——只挂报警牌不干预，
                  下游自行处置）。
    首扫描即坏值时最后好值取当前 u（无更好历史）。
    dt_s <= 0 → 坏值计时不推进（timeout_s > 0 时维持保持段）。

  输入：
    u         REAL  输入（原始 PV）
    good      BOOL  输入有效（FALSE = 坏值；接质量位/断线判定取反）
    mode      REAL  0 = 保持最后好值 / 1 = 替代值 / 2 = 跟随原值
    sub_value REAL  替代值（mode = 1 用）
    timeout_s REAL  坏值持续超时 s（0 = 立即按 mode 动作；> 0 先保持）
    dt_s      REAL  采样周期 s
  输出：
    out       REAL  处理后的 PV
    q_bad     BOOL  坏值中（电平）
    q_timeout BOOL  坏值已超时（mode 已生效）
*)
FUNCTION_BLOCK FB_PV_HOLD
  VAR_INPUT
    u         : REAL;
    good      : BOOL := TRUE;
    mode      : REAL := 0.0;
    sub_value : REAL := 0.0;
    timeout_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out       : REAL;
    q_bad     : BOOL;
    q_timeout : BOOL;
  END_VAR
  VAR
    last_good : REAL; (* 最后好值 *)
    bad_t     : REAL; (* 坏值持续累计时间 s *)
    init_done : BOOL; (* 首扫描播种标志 *)
  END_VAR

  IF NOT init_done THEN
    last_good := u; (* 首扫描即坏值时也以当前 u 作保持值 *)
    init_done := TRUE;
  END_IF;

  IF good THEN
    last_good := u;
    out := u;
    q_bad := FALSE;
    q_timeout := FALSE;
    bad_t := 0.0;
    RETURN;
  END_IF;

  q_bad := TRUE;

  (* 超时前：保持最后好值（短时抖动渡过段） *)
  IF timeout_s > 0.0 THEN
    IF dt_s > 0.0 THEN
      bad_t := bad_t + dt_s;
    END_IF;
    IF bad_t < timeout_s THEN
      out := last_good;
      q_timeout := FALSE;
      RETURN;
    END_IF;
  END_IF;

  (* 超时（或 timeout_s = 0 立即）：按 mode 动作 *)
  q_timeout := TRUE;
  IF mode >= 1.5 THEN
    out := u;         (* 2 = 跟随原值 *)
  ELSIF mode >= 0.5 THEN
    out := sub_value; (* 1 = 替代值 *)
  ELSE
    out := last_good; (* 0 = 保持最后好值 *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_RAMPSOAK8 — 8 段程序给定发生器（ramp-soak setpoint programmer）。

  ≈ 标准程序给定/曲线给定块（ramp-soak / setpoint programmer，
  升温曲线、热处理曲线、批次温度程序），各主流 DCS/PLC 与程序
  调节仪均有同类块。

  用途：按"斜坡到目标 + 保温"的分段曲线生成设定值，接 FB_PID /
  FB_PID_POS 的 sp 输入：热处理/养护/反应釜升温程序、开停车
  升降负荷曲线。段参数为标量展开 sp1..sp8 / ramp1_s..ramp8_s /
  soak1_s..soak8_s（本方言 FB 作用域数组不能 codegen，见 README
  坑 7；FB_CHAR 同例）。

  行为：
    每段 N：sp 从上一段电平（段 1 从 sp_init）经 rampN_s 线性斜坡到
    spN（rampN_s <= 0 跳变），再保温 soakN_s（soaking = TRUE；
    soakN_s <= 0 即过）。段参数在进段拍锁存——运行中改参数下一段
    生效。
    run 电平有效：TRUE 启动并推进程序；FALSE → 回到初始（seg := 0，
    sp := sp_init，done 清零），再置 TRUE 从段 1 重新开始。
    hold 电平有效：冻结 sp 与段内计时（hold 期间 advance 无效）。
    advance 上升沿：立即跳下一段（从当前 sp 续斜坡，曲线连续）；
    在末段 advance → 完成（或 cyclic 回绕）。
    末段（n_seg 钳 2..8）保温结束：
      cyclic = FALSE → done := TRUE，sp 钉在末段目标，seg := 0，
      保持到 reset 或 run 撤销；
      cyclic = TRUE  → 回绕段 1（从当前 sp 续斜坡，长期循环）。
    reset 电平有效（优先）：回初始同 run 撤销。
    dt_s <= 0 → 段内不推进（启动/advance 沿照常处理）。

  输入：
    sp1..sp8       REAL  各段目标值（标量展开）
    ramp1_s..ramp8_s REAL  各段斜坡时间 s（<= 0 = 跳变）
    soak1_s..soak8_s REAL  各段保温时间 s（<= 0 = 不保温）
    n_seg          REAL  使用段数 2..8（自动钳位）
    run            BOOL  运行（电平；撤销回初始）
    hold           BOOL  暂停（电平；冻结 sp 与计时）
    advance        BOOL  跳段（上升沿）
    reset          BOOL  复位（电平，优先）
    cyclic         BOOL  TRUE = 末段完成后回绕段 1 循环
    sp_init        REAL  起始值（段 1 斜坡的出发电平）
    dt_s           REAL  采样周期 s
  输出：
    sp      REAL  程序设定值输出
    seg     REAL  当前段 1..8（0 = 未运行/已完成）
    soaking BOOL  保温段进行中
    done    BOOL  程序完成（非循环；保持到 reset / run 撤销）
*)
FUNCTION_BLOCK FB_RAMPSOAK8
  VAR_INPUT
    sp1     : REAL := 0.0;
    sp2     : REAL := 0.0;
    sp3     : REAL := 0.0;
    sp4     : REAL := 0.0;
    sp5     : REAL := 0.0;
    sp6     : REAL := 0.0;
    sp7     : REAL := 0.0;
    sp8     : REAL := 0.0;
    ramp1_s : REAL := 0.0;
    ramp2_s : REAL := 0.0;
    ramp3_s : REAL := 0.0;
    ramp4_s : REAL := 0.0;
    ramp5_s : REAL := 0.0;
    ramp6_s : REAL := 0.0;
    ramp7_s : REAL := 0.0;
    ramp8_s : REAL := 0.0;
    soak1_s : REAL := 0.0;
    soak2_s : REAL := 0.0;
    soak3_s : REAL := 0.0;
    soak4_s : REAL := 0.0;
    soak5_s : REAL := 0.0;
    soak6_s : REAL := 0.0;
    soak7_s : REAL := 0.0;
    soak8_s : REAL := 0.0;
    n_seg   : REAL := 2.0;
    run     : BOOL := FALSE;
    hold    : BOOL := FALSE;
    advance : BOOL := FALSE;
    reset   : BOOL := FALSE;
    cyclic  : BOOL := FALSE;
    sp_init : REAL := 0.0;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    sp      : REAL;
    seg     : REAL;
    soaking : BOOL;
    done    : BOOL;
  END_VAR
  VAR
    nn         : REAL; (* 钳位后的段数 2..8 *)
    elapsed_t  : REAL; (* 段内相位计时 s（斜坡/保温各自从 0 起） *)
    level_from : REAL; (* 本段斜坡出发电平 *)
    seg_sp     : REAL; (* 进段锁存的段参数 *)
    seg_ramp_s : REAL;
    seg_soak_s : REAL;
    enter_seg  : REAL; (* 本拍要进入的段号（0 = 不进段） *)
    go_next    : BOOL; (* 本拍段完成/跳段 *)
    adv_edge   : BOOL;
    prev_adv   : BOOL; (* advance 上升沿记忆 *)
  END_VAR

  IF reset THEN
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    elapsed_t := 0.0;
    prev_adv := advance;
    RETURN;
  END_IF;

  adv_edge := advance AND NOT prev_adv;
  prev_adv := advance;

  IF NOT run THEN
    (* 停止：回初始，重新 run 从段 1 开始 *)
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    RETURN;
  END_IF;

  IF done THEN
    RETURN; (* 完成保持：sp 钉在末段目标，等 reset / run 撤销 *)
  END_IF;

  nn := n_seg;
  IF nn < 2.0 THEN
    nn := 2.0;
  ELSIF nn > 8.0 THEN
    nn := 8.0;
  END_IF;

  enter_seg := 0.0;
  go_next := FALSE;

  IF seg < 0.5 THEN
    (* 启动：从 sp_init 进段 1 *)
    sp := sp_init;
    enter_seg := 1.0;
  ELSIF hold THEN
    RETURN; (* 冻结：sp 与计时保持（advance 在 hold 期间无效） *)
  ELSIF adv_edge THEN
    go_next := TRUE; (* 跳段：从当前 sp 续走 *)
  ELSE
    IF dt_s <= 0.0 THEN
      RETURN; (* 采样周期非法：段内不推进 *)
    END_IF;
    elapsed_t := elapsed_t + dt_s;
    IF NOT soaking THEN
      (* 斜坡相位 *)
      IF seg_ramp_s <= 0.0 THEN
        sp := seg_sp; (* 跳变段 *)
        soaking := TRUE;
        elapsed_t := 0.0;
      ELSIF elapsed_t >= seg_ramp_s THEN
        sp := seg_sp;
        soaking := TRUE;
        elapsed_t := elapsed_t - seg_ramp_s; (* 跨相位余量结转 *)
      ELSE
        sp := level_from + (seg_sp - level_from) * elapsed_t / seg_ramp_s;
      END_IF;
    END_IF;
    IF soaking AND elapsed_t >= seg_soak_s THEN
      go_next := TRUE; (* 保温结束 *)
    END_IF;
  END_IF;

  (* 段推进决策：末段 → 完成/回绕；否则进下一段 *)
  IF go_next THEN
    IF seg >= nn - 0.5 THEN
      IF cyclic THEN
        enter_seg := 1.0; (* 回绕：段 1 从当前 sp 续斜坡 *)
      ELSE
        done := TRUE;
        sp := seg_sp; (* 完成：钉在末段目标（提前跳段也直接到位） *)
        seg := 0.0;
        soaking := FALSE;
      END_IF;
    ELSE
      enter_seg := seg + 1.0;
    END_IF;
  END_IF;

  (* 进段：锁存段参数，斜坡从当前 sp 出发 *)
  IF enter_seg > 0.5 THEN
    seg := enter_seg;
    level_from := sp;
    elapsed_t := 0.0;
    soaking := FALSE;
    IF seg <= 1.5 THEN
      seg_sp := sp1;
      seg_ramp_s := ramp1_s;
      seg_soak_s := soak1_s;
    ELSIF seg <= 2.5 THEN
      seg_sp := sp2;
      seg_ramp_s := ramp2_s;
      seg_soak_s := soak2_s;
    ELSIF seg <= 3.5 THEN
      seg_sp := sp3;
      seg_ramp_s := ramp3_s;
      seg_soak_s := soak3_s;
    ELSIF seg <= 4.5 THEN
      seg_sp := sp4;
      seg_ramp_s := ramp4_s;
      seg_soak_s := soak4_s;
    ELSIF seg <= 5.5 THEN
      seg_sp := sp5;
      seg_ramp_s := ramp5_s;
      seg_soak_s := soak5_s;
    ELSIF seg <= 6.5 THEN
      seg_sp := sp6;
      seg_ramp_s := ramp6_s;
      seg_soak_s := soak6_s;
    ELSIF seg <= 7.5 THEN
      seg_sp := sp7;
      seg_ramp_s := ramp7_s;
      seg_soak_s := soak7_s;
    ELSE
      seg_sp := sp8;
      seg_ramp_s := ramp8_s;
      seg_soak_s := soak8_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_GAIN_SCHED3 — 三区增益调度（gain scheduling，迟滞分区 +
  可选区界线性过渡）。

  ≈ 标准增益调度器（gain scheduler / parameter scheduler，按工况
  变量分区切换 PID 参数组），非线性对象分段整定的惯用块。

  用途：对象增益随工况大幅变化（负荷段、阀特性段、pH 强非线性段）
  时按调度变量（负荷/开度/PV 本身）分三区出 kp/ti/td 参数组，
  输出直接接 FB_PID / FB_PID_POS 的参数输入——参数在线改变即时
  生效（两块均按当前参数算增量/分项）。

  行为：
    分区（u_sched 对 x12 / x23 两个区界，假定 x12 < x23）：
      区 1：u <= x12；区 2：x12 < u <= x23；区 3：u > x23。
    迟滞：越过区界 ± hyst 才换区（防区界附近抖动来回切参数）；
    首扫描按纯比较播种。大阶跃一拍内可连跳两区（逐级迁移收敛）。
    参数输出：
      interp = FALSE：取当前区参数组（迟滞切换，参数有台阶——
      接 FB_PID 增量式天然无扰，接 FB_PID_POS 时 P 项会按新 kp
      立即重算）；
      interp = TRUE 且 hyst > 0：u 落在区界 ±hyst 内时两区参数
      线性过渡（参数输出连续、仅由 u_sched 决定；zone 仍按迟滞出
      离散指示）。
    注意：interp 混用 ti = 0（积分关）与 ti > 0 的区时，过渡段会
    产生极小的 ti（极强积分）——各区 ti/td 应同为 0 或同为非 0。

  输入：
    u_sched         REAL  调度变量（负荷/阀位/PV 等）
    x12             REAL  区 1/2 边界
    x23             REAL  区 2/3 边界（> x12，且两过渡带不重叠）
    hyst            REAL  区界迟滞半宽（0 = 无迟滞）
    interp          BOOL  TRUE = 区界 ±hyst 内参数线性过渡
    kp1, ti1_s, td1_s REAL  区 1 参数组
    kp2, ti2_s, td2_s REAL  区 2 参数组
    kp3, ti3_s, td3_s REAL  区 3 参数组
  输出：
    kp_out   REAL  调度后比例增益（接 PID 的 kp）
    ti_out_s REAL  调度后积分时间 s
    td_out_s REAL  调度后微分时间 s
    zone     REAL  当前区 1 / 2 / 3（迟滞判定）
*)
FUNCTION_BLOCK FB_GAIN_SCHED3
  VAR_INPUT
    u_sched : REAL;
    x12     : REAL := 33.3;
    x23     : REAL := 66.6;
    hyst    : REAL := 0.0;
    interp  : BOOL := FALSE;
    kp1     : REAL := 1.0;
    ti1_s   : REAL := 0.0;
    td1_s   : REAL := 0.0;
    kp2     : REAL := 1.0;
    ti2_s   : REAL := 0.0;
    td2_s   : REAL := 0.0;
    kp3     : REAL := 1.0;
    ti3_s   : REAL := 0.0;
    td3_s   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    kp_out   : REAL;
    ti_out_s : REAL;
    td_out_s : REAL;
    zone     : REAL;
  END_VAR
  VAR
    f : REAL; (* 过渡带内插值分数 0..1 *)
  END_VAR

  (* 1) 区选择（迟滞）：首扫描 zone = 0 → 按纯比较播种 *)
  IF zone < 0.5 THEN
    IF u_sched <= x12 THEN
      zone := 1.0;
    ELSIF u_sched <= x23 THEN
      zone := 2.0;
    ELSE
      zone := 3.0;
    END_IF;
  END_IF;

  (* 逐级迁移（1→2、3→2 先判，2→1/2→3 后判：大阶跃一拍收敛） *)
  IF zone <= 1.5 THEN
    IF u_sched > x12 + hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 2.5 THEN
    IF u_sched < x23 - hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 1.5 AND zone <= 2.5 THEN
    IF u_sched > x23 + hyst THEN
      zone := 3.0;
    ELSIF u_sched < x12 - hyst THEN
      zone := 1.0;
    END_IF;
  END_IF;

  (* 2) 区参数（离散取值，迟滞切换） *)
  IF zone <= 1.5 THEN
    kp_out   := kp1;
    ti_out_s := ti1_s;
    td_out_s := td1_s;
  ELSIF zone <= 2.5 THEN
    kp_out   := kp2;
    ti_out_s := ti2_s;
    td_out_s := td2_s;
  ELSE
    kp_out   := kp3;
    ti_out_s := ti3_s;
    td_out_s := td3_s;
  END_IF;

  (* 3) interp：区界 ±hyst 内线性过渡（参数连续、仅由 u_sched 决定，
        覆盖上面的离散取值；zone 输出不受影响） *)
  IF interp AND hyst > 0.0 THEN
    IF (u_sched > x12 - hyst) AND (u_sched < x12 + hyst) THEN
      f := (u_sched - (x12 - hyst)) / (2.0 * hyst);
      kp_out   := kp1 + f * (kp2 - kp1);
      ti_out_s := ti1_s + f * (ti2_s - ti1_s);
      td_out_s := td1_s + f * (td2_s - td1_s);
    ELSIF (u_sched > x23 - hyst) AND (u_sched < x23 + hyst) THEN
      f := (u_sched - (x23 - hyst)) / (2.0 * hyst);
      kp_out   := kp2 + f * (kp3 - kp2);
      ti_out_s := ti2_s + f * (ti3_s - ti2_s);
      td_out_s := td2_s + f * (td3_s - td2_s);
    ELSIF u_sched <= x12 - hyst THEN
      kp_out   := kp1;
      ti_out_s := ti1_s;
      td_out_s := td1_s;
    ELSIF u_sched >= x23 + hyst THEN
      kp_out   := kp3;
      ti_out_s := ti3_s;
      td_out_s := td3_s;
    ELSE
      kp_out   := kp2; (* 两过渡带之间：纯区 2 *)
      ti_out_s := ti2_s;
      td_out_s := td2_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_PID_POS — 位置式（绝对输出）PID 调节器，p/i/d 分项可观测，
  带 SP 加权、微分先行（D 于 PV + PT1 滤波）、回算抗饱和与无扰切换。

  ≈ 标准位置式 PID（绝对输出形态，P/I/D 分项可观测、SP 加权 b、
  微分作用于 PV），各主流 DCS/PLC 标准库均有同类块。

  用途：与速度型 FB_PID 互补——需要观测/调试积分项、要 SP 加权
  （b < 1 抑制设定值阶跃的比例踢）、或下游需要绝对量输出的回路。
  增量输出/级联接力等场景仍推荐 FB_PID。

  算法（位置式）：
    e   := sp - pv（反作用，默认）；direct = TRUE 时 e := pv - sp
    e_p := b*sp - pv（P 项专用的 SP 加权偏差；direct 时取反向）
    p_part := kp * e_p
    i_part := i_part + kp*dt_s/ti_s * e（ti_s > 0 时累加）
    d_part := -kp*td_s * d(pv_f)/dt（微分作用于 PV：SP 阶跃不踢；
              pv 先经 PT1 滤波，t_dfilt_s <= 0 自动取 3*dt_s；
              direct 时符号取正）
    out := p_part + i_part + d_part + ff，钳位到 [out_min, out_max]
  抗积分饱和与无扰（回算）：每拍收尾把 i_part 回算为
    i_part := out - p_part - d_part - ff
  ——钳位时积分被拉回（不饱和）；手动/跟踪时 out 跟随给定，i_part
  同步回算，切回自动无跳变。ti_s = 0（无积分）时 i_part 仅在手动/
  跟踪期间回算（此时它是固定偏置/手动复位项，自动期间不被钳位事件
  改写，P 型调节的静差特性保持稳定）。
  模式优先级：track > NOT auto（手动）> 自动（与 FB_PID 一致）。
  恒等式：任意模式下 out = p_part + i_part + d_part + ff（钳位后，
  ti_s = 0 的自动钳位拍除外）——分项之和即输出，调试直观。

  输入：
    pv          REAL  过程值
    sp          REAL  设定值
    kp          REAL  比例增益
    ti_s        REAL  积分时间 s（0 = 无积分）
    td_s        REAL  微分时间 s（0 = 微分关）
    t_dfilt_s   REAL  微分前置 PT1 滤波时间常数 s（<= 0 = 自动 3*dt_s）
    sp_weight_b REAL  SP 加权系数 b（仅作用于 P 项；1 = 不加权，
                      0 = P 项只看 pv——SP 阶跃完全不踢比例）
    dt_s        REAL  采样周期 s（<= 0 时本周期保持不动作）
    direct      BOOL  TRUE = 正作用（pv↑→out↑），FALSE = 反作用
    auto        BOOL  TRUE 自动 / FALSE 手动
    manual_out  REAL  手动给定值
    out_min     REAL  输出下限（假定 out_min < out_max）
    out_max     REAL  输出上限
    ff          REAL  前馈量（钳位前叠加到输出）
    track       BOOL  输出跟踪使能（优先级最高）
    track_value REAL  跟踪值
  输出：
    out    REAL  调节输出（各模式均钳位）
    dev    REAL  偏差（反作用 sp-pv / 正作用 pv-sp，不带 SP 加权）
    p_part REAL  比例分项（含 SP 加权）
    i_part REAL  积分分项（含回算修正）
    d_part REAL  微分分项
*)
FUNCTION_BLOCK FB_PID_POS
  VAR_INPUT
    pv          : REAL;
    sp          : REAL;
    kp          : REAL := 1.0;
    ti_s        : REAL := 0.0;
    td_s        : REAL := 0.0;
    t_dfilt_s   : REAL := 0.0;
    sp_weight_b : REAL := 1.0;
    dt_s        : REAL := 0.1;
    direct      : BOOL := FALSE;
    auto        : BOOL := TRUE;
    manual_out  : REAL := 0.0;
    out_min     : REAL := 0.0;
    out_max     : REAL := 100.0;
    ff          : REAL := 0.0;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    dev    : REAL;
    p_part : REAL;
    i_part : REAL;
    d_part : REAL;
  END_VAR
  VAR
    pv_f      : REAL; (* D 项 PT1 滤波后的 PV *)
    pv_f_prev : REAL; (* 上一拍滤波值，后向差分用 *)
    t_f       : REAL; (* 实际使用的滤波时间常数 *)
    d_pv      : REAL; (* 滤波 PV 的变化率 单位/s *)
    e         : REAL; (* 控制偏差（含作用方向） *)
    e_p       : REAL; (* P 项偏差（SP 加权 b） *)
    init_done : BOOL; (* 滤波态首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持输出与历史不变 *)
  END_IF;

  IF NOT init_done THEN
    pv_f := pv; (* 滤波态对中：不出投运假微分 *)
    pv_f_prev := pv;
    init_done := TRUE;
  END_IF;

  (* D 项前置 PT1 滤波 + 后向差分（与 FB_DERIV 同核） *)
  t_f := t_dfilt_s;
  IF t_f <= 0.0 THEN
    t_f := 3.0 * dt_s;
  END_IF;
  pv_f := pv_f + dt_s / (t_f + dt_s) * (pv - pv_f);
  d_pv := (pv_f - pv_f_prev) / dt_s;
  pv_f_prev := pv_f;

  (* 偏差（按作用方向）与 SP 加权 P 偏差 *)
  IF direct THEN
    e := pv - sp;
    e_p := pv - sp_weight_b * sp;
  ELSE
    e := sp - pv;
    e_p := sp_weight_b * sp - pv;
  END_IF;
  dev := e;

  p_part := kp * e_p;

  d_part := 0.0;
  IF td_s > 0.0 THEN
    (* 微分作用于 PV（SP 阶跃不踢微分）：反作用 pv↑ 应压输出 → 取负号 *)
    IF direct THEN
      d_part := kp * td_s * d_pv;
    ELSE
      d_part := 0.0 - kp * td_s * d_pv;
    END_IF;
  END_IF;

  IF track THEN
    (* 输出跟踪：无条件跟随 track_value，优先级最高 *)
    out := track_value;
  ELSIF NOT auto THEN
    (* 手动：跟踪 manual_out，i_part 回算保证返回自动无扰 *)
    out := manual_out;
  ELSE
    IF ti_s > 0.0 THEN
      i_part := i_part + kp * dt_s / ti_s * e;
    END_IF;
    out := p_part + i_part + d_part + ff;
  END_IF;

  (* 输出钳位 *)
  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;

  (* 回算 i_part：使 P+I+D+ff 恒等于钳位后输出 → 钳位即抗积分饱和、
     手动/跟踪即无扰对中。ti_s = 0 时仅手动/跟踪期间回算（见头注） *)
  IF ti_s > 0.0 OR track OR NOT auto THEN
    i_part := out - p_part - d_part - ff;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_MANSTATION — 手操/偏置站（manual loader / bias station）。

  ≈ 标准手操站 + 偏置站（manual station / auto-manual station with
  bias），调节输出与执行机构之间的标准操作层，各主流 DCS/PLC
  标准库均有同类块。

  用途：放在调节器输出与执行机构之间：自动时透传（可加偏置、可限
  速），手动时操作员直接给定输出；balance 保证手动→自动切换无扰。
  也可单独作纯手操器（auto := FALSE 常手动）或纯偏置/限速站。

  行为：
    自动（auto = TRUE）：out 朝 (auto_in + bias + corr) 逼近——
      rate > 0 时按 rate 单位/秒限速，rate = 0 直接跳变；钳位到
      [out_min, out_max]。corr 是 balance 切换时回算的内部偏置修正。
    手动（auto = FALSE）：out := manual_out（钳位，不限速），
      tracking := TRUE。
    无扰切换（balance = TRUE）：手动→自动切换沿回算
      corr := out - auto_in - bias
    ——切换瞬间 out 连续；之后 auto_in/bias 的变化照常生效，corr
    保持到下一次切换再重算。balance = FALSE 时 corr 恒 0（切换差值
    靠 rate 限速渡过；rate = 0 则跳变）。
    首扫描：out 直接对中到当前模式的目标值（不从 0 爬升）。
    dt_s <= 0 → 本周期保持不动作。
    注意：bias 是操作员/上位写的名义偏置；corr 是块内自整定修正，
    两者叠加。要"清掉"历史修正：置 balance := FALSE 一拍即可。

  输入：
    auto_in    REAL  自动输入（上游调节器输出）
    auto       BOOL  TRUE 自动 / FALSE 手动
    manual_out REAL  手动给定值
    bias       REAL  偏置（自动时叠加）
    rate       REAL  自动输出限速 单位/s（0 = 不限速）
    out_min    REAL  输出下限
    out_max    REAL  输出上限
    balance    BOOL  TRUE = 手动→自动切换回算偏置修正（无扰）
    dt_s       REAL  采样周期 s
  输出：
    out      REAL  站输出（各模式均钳位）
    tracking BOOL  手动中（上游调节器可接此位做输出跟踪）
*)
FUNCTION_BLOCK FB_MANSTATION
  VAR_INPUT
    auto_in    : REAL;
    auto       : BOOL := TRUE;
    manual_out : REAL := 0.0;
    bias       : REAL := 0.0;
    rate       : REAL := 0.0;
    out_min    : REAL := 0.0;
    out_max    : REAL := 100.0;
    balance    : BOOL := TRUE;
    dt_s       : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out      : REAL;
    tracking : BOOL;
  END_VAR
  VAR
    corr      : REAL; (* balance 回算的内部偏置修正 *)
    target    : REAL; (* 自动目标 = auto_in + bias + corr *)
    delta     : REAL;
    step      : REAL;
    prev_auto : BOOL; (* 切换沿记忆 *)
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  IF NOT balance THEN
    corr := 0.0;
  END_IF;

  IF NOT init_done THEN
    (* 首扫描：直接对中当前模式目标，不从 0 爬升 *)
    IF auto THEN
      out := auto_in + bias;
    ELSE
      out := manual_out;
    END_IF;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := NOT auto;
    prev_auto := auto;
    init_done := TRUE;
    RETURN;
  END_IF;

  (* 手动→自动切换沿：balance 回算修正，使 out 连续 *)
  IF auto AND NOT prev_auto AND balance THEN
    corr := out - auto_in - bias;
  END_IF;
  prev_auto := auto;

  IF NOT auto THEN
    (* 手动：直接给定（钳位，不限速） *)
    out := manual_out;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := TRUE;
    RETURN;
  END_IF;
  tracking := FALSE;

  (* 自动：朝 auto_in + bias + corr 限速逼近（rate = 0 跳变） *)
  target := auto_in + bias + corr;
  IF rate > 0.0 THEN
    delta := target - out;
    step := rate * dt_s;
    IF delta > step THEN
      out := out + step;
    ELSIF delta < 0.0 - step THEN
      out := out - step;
    ELSE
      out := target;
    END_IF;
  ELSE
    out := target;
  END_IF;

  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_PID_3STEP — 步进（三步）PID：电动执行机构的开/关脉冲调节，
  支持无阀位反馈（行程时间积分估计）与有反馈两种接法。

  ≈ 标准步进 PID / 三步输出调节器（step controller / 3-step PID，
  驱动电动调节阀、电动风门等积分型执行机构），各主流 DCS/PLC
  标准库均有同类块。

  用途：电动执行机构（电动阀/电动风门，机构本身是积分环节：通电
  开/关方向运行，断电停在原位）的闭环调节。无阀位变送器时用行程
  时间积分估计阀位（pos_fb < 0），有 4-20 mA 阀位反馈时直接用。

  行为：
    1) 阀位估计 pos_est：pos_fb >= 0 直接取反馈（钳 0..100）；否则按
       上一拍输出与行程时间积分：开脉冲 +100*dt_s/motor_time_s，
       关脉冲同理递减（钳 0..100）。
    2) 内部位置式 PID（反作用 e = sp - pv，D 作用于 PV、PT1 自动
       3*dt_s 滤波）算出目标阀位 target 0..100；积分回算抗饱和；
       手动时 target 跟踪 pos_est，返回自动无扰。正作用场景请在
       外部对 pv/sp 做取反处理。
    3) 三步输出：阀位偏差 = target - pos_est；
       > deadband_pct 出开脉冲，< -deadband_pct 出关脉冲，死区内停。
       自动方式下脉冲一旦发出至少保持 min_pulse_s（防短脉冲打接触器）；
       手动点动 man_open/man_close 直通（无最小脉宽，同发互锁），
       仅受行程限位约束。换向必经一拍全停（先停后反向）。
       贴行程边界（pos_est 到 0 或 100）立即停该方向脉冲。
    整定提示：deadband_pct 应大于最小脉宽对应的行程
    （min_pulse_s / motor_time_s * 100%），否则死区内仍会过冲振荡。

  输入：
    pv           REAL  过程值
    sp           REAL  设定值
    kp           REAL  比例增益
    ti_s         REAL  积分时间 s（0 = 无积分）
    td_s         REAL  微分时间 s（0 = 微分关）
    dt_s         REAL  采样周期 s（<= 0 时本周期保持不动作）
    motor_time_s REAL  执行机构全行程时间 s（0→100%；估计与限位用）
    min_pulse_s  REAL  自动方式最小脉冲宽度 s
    deadband_pct REAL  阀位死区 %（偏差死区内不动作）
    auto         BOOL  TRUE 自动 / FALSE 手动点动
    man_open     BOOL  手动开点动（与 man_close 同发互锁不动作）
    man_close    BOOL  手动关点动
    pos_fb       REAL  阀位反馈 0..100 %（< 0 = 无反馈，用行程估计）
  输出：
    q_open  BOOL  开方向脉冲输出
    q_close BOOL  关方向脉冲输出
    pos_est REAL  阀位估计/反馈 0..100 %
*)
FUNCTION_BLOCK FB_PID_3STEP
  VAR_INPUT
    pv           : REAL;
    sp           : REAL;
    kp           : REAL := 1.0;
    ti_s         : REAL := 0.0;
    td_s         : REAL := 0.0;
    dt_s         : REAL := 0.1;
    motor_time_s : REAL := 60.0;
    min_pulse_s  : REAL := 0.5;
    deadband_pct : REAL := 1.0;
    auto         : BOOL := TRUE;
    man_open     : BOOL := FALSE;
    man_close    : BOOL := FALSE;
    pos_fb       : REAL := -1.0;
  END_VAR
  VAR_OUTPUT
    q_open  : BOOL;
    q_close : BOOL;
    pos_est : REAL;
  END_VAR
  VAR
    i_acc     : REAL; (* 位置式 PID 积分项（回算无扰） *)
    pv_f      : REAL; (* D 项 PT1 滤波态 *)
    pv_f_prev : REAL;
    t_f       : REAL;
    e         : REAL;
    p_term    : REAL;
    d_term    : REAL;
    target    : REAL; (* PID 目标阀位 0..100 *)
    err_pos   : REAL; (* 阀位偏差 target - pos_est *)
    pulse_t   : REAL; (* 当前脉冲已持续时间 s *)
    open_req  : BOOL; (* 本拍开方向需求 *)
    close_req : BOOL;
    init_done : BOOL; (* 滤波态首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  IF NOT init_done THEN
    pv_f := pv;
    pv_f_prev := pv;
    init_done := TRUE;
  END_IF;

  (* 1) 阀位估计：有反馈直接用；无反馈按上一拍输出行程时间积分 *)
  IF pos_fb >= 0.0 THEN
    pos_est := pos_fb;
  ELSE
    IF motor_time_s > 0.0 THEN
      IF q_open THEN
        pos_est := pos_est + 100.0 * dt_s / motor_time_s;
      END_IF;
      IF q_close THEN
        pos_est := pos_est - 100.0 * dt_s / motor_time_s;
      END_IF;
    END_IF;
  END_IF;
  IF pos_est > 100.0 THEN
    pos_est := 100.0;
  ELSIF pos_est < 0.0 THEN
    pos_est := 0.0;
  END_IF;

  (* 2) 位置式 PID → 目标阀位（反作用；D 于 PV，自动 3*dt_s 滤波） *)
  t_f := 3.0 * dt_s;
  pv_f := pv_f + dt_s / (t_f + dt_s) * (pv - pv_f);
  e := sp - pv;
  p_term := kp * e;
  d_term := 0.0;
  IF td_s > 0.0 THEN
    d_term := 0.0 - kp * td_s * (pv_f - pv_f_prev) / dt_s;
  END_IF;
  pv_f_prev := pv_f;

  IF auto THEN
    IF ti_s > 0.0 THEN
      i_acc := i_acc + kp * dt_s / ti_s * e;
    END_IF;
    target := p_term + i_acc + d_term;
  ELSE
    target := pos_est; (* 手动：目标跟踪当前阀位，返回自动无扰 *)
  END_IF;
  IF target > 100.0 THEN
    target := 100.0;
  ELSIF target < 0.0 THEN
    target := 0.0;
  END_IF;
  IF ti_s > 0.0 OR NOT auto THEN
    i_acc := target - p_term - d_term; (* 回算：抗饱和 + 无扰 *)
  END_IF;

  (* 3) 三步输出需求 *)
  IF auto THEN
    err_pos := target - pos_est;
    open_req := err_pos > deadband_pct;
    close_req := err_pos < 0.0 - deadband_pct;
  ELSE
    open_req := man_open AND NOT man_close; (* 手动点动直通，同发互锁 *)
    close_req := man_close AND NOT man_open;
  END_IF;

  (* 行程限位：贴边不再出该方向脉冲（手/自动一致） *)
  IF pos_est >= 100.0 THEN
    open_req := FALSE;
  END_IF;
  IF pos_est <= 0.0 THEN
    close_req := FALSE;
  END_IF;

  (* 脉冲计时与输出：自动撤需求须满足最小脉宽；手动撤即停；
     换向必经一拍全停（当前方向先落，下一拍才允许反向） *)
  IF q_open OR q_close THEN
    pulse_t := pulse_t + dt_s;
  END_IF;

  IF q_open THEN
    IF pos_est >= 100.0 THEN
      q_open := FALSE; (* 限位即停 *)
    ELSIF NOT open_req AND (NOT auto OR pulse_t >= min_pulse_s) THEN
      q_open := FALSE;
    END_IF;
  ELSIF q_close THEN
    IF pos_est <= 0.0 THEN
      q_close := FALSE;
    ELSIF NOT close_req AND (NOT auto OR pulse_t >= min_pulse_s) THEN
      q_close := FALSE;
    END_IF;
  ELSE
    pulse_t := 0.0;
    IF open_req THEN
      q_open := TRUE;
    ELSIF close_req THEN
      q_close := TRUE;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_MOTOR_REV — 双向（正/反转）电机控制，带换向闭锁、运行反馈
  监视与故障锁存。

  ≈ 标准双向电机块（reversing motor / forward-reverse starter，
  含换向互锁延时），各主流 DCS/PLC 标准库均有同类块。

  用途：搅拌器、卷扬、双向皮带、闸门开度电机等可逆运转设备的
  标准封装（范式沿自 FB_MOTOR：命令锁存 + 反馈超时故障锁存）。

  行为：
    1) 故障监视：fb_fwd 与 fb_rev 同时为 TRUE → 反馈/接触器故障，
       立即 fault；输出与反馈不一致（q_fwd<>fb_fwd 或 q_rev<>fb_rev）
       持续 fb_timeout_s（手工 dt_s 累加计时）→ fault。均锁存，
       reset 且双反馈条件已消除时清除。
    2) 方向命令锁存（停车/故障优先）：cmd_fwd / cmd_rev 置位对应
       方向请求（同拍双向命令视为冲突，不动作）；stop 或 fault 清除
       全部请求。
    3) 换向闭锁：换向（反向请求仍在、或反向反馈仍在转）时强制双输出
       全停 reversal_delay_s 后才允许新方向输出——接触器灭弧/机械
       泄劲时间。reversal_delay_s 应小于 fb_timeout_s（闭锁等待期的
       反馈余转靠超时容忍）。
    4) 状态字 state：0 停止 / 1 正转 / 2 反转 / 3 故障 / 4 换向等待。

  输入：
    cmd_fwd          BOOL  正转命令
    cmd_rev          BOOL  反转命令
    stop             BOOL  停止命令（优先）
    fb_fwd           BOOL  正转运行反馈
    fb_rev           BOOL  反转运行反馈
    fb_timeout_s     REAL  反馈不一致容忍时间 s
    reversal_delay_s REAL  换向全停闭锁时间 s
    reset            BOOL  故障复位
    dt_s             REAL  采样周期 s
  输出：
    q_fwd BOOL  正转输出
    q_rev BOOL  反转输出
    fault BOOL  故障（锁存）
    state REAL  0 停止 / 1 正转 / 2 反转 / 3 故障 / 4 换向等待
*)
FUNCTION_BLOCK FB_MOTOR_REV
  VAR_INPUT
    cmd_fwd          : BOOL;
    cmd_rev          : BOOL;
    stop             : BOOL;
    fb_fwd           : BOOL;
    fb_rev           : BOOL;
    fb_timeout_s     : REAL := 5.0;
    reversal_delay_s : REAL := 2.0;
    reset            : BOOL;
    dt_s             : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_fwd : BOOL;
    q_rev : BOOL;
    fault : BOOL;
    state : REAL;
  END_VAR
  VAR
    want_fwd : BOOL; (* 锁存的方向请求 *)
    want_rev : BOOL;
    wait_t   : REAL; (* 换向闭锁剩余时间 s *)
    mism_t   : REAL; (* 反馈不一致累计时间 s *)
    conflict : BOOL; (* 同拍双向命令 *)
  END_VAR

  (* 1) 故障监视 *)
  IF fb_fwd AND fb_rev THEN
    fault := TRUE; (* 双向反馈同时有效：接触器粘连/反馈回路故障 *)
  END_IF;
  IF (q_fwd <> fb_fwd) OR (q_rev <> fb_rev) THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT (fb_fwd AND fb_rev) THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) 方向命令锁存（停车/故障优先；同拍双向命令冲突不动作） *)
  conflict := cmd_fwd AND cmd_rev;
  IF cmd_fwd AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_rev OR fb_rev THEN
      wait_t := reversal_delay_s; (* 反向仍在令/仍在转：先全停闭锁 *)
    END_IF;
    want_fwd := TRUE;
    want_rev := FALSE;
  ELSIF cmd_rev AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_fwd OR fb_fwd THEN
      wait_t := reversal_delay_s;
    END_IF;
    want_rev := TRUE;
    want_fwd := FALSE;
  END_IF;
  IF stop OR fault THEN
    want_fwd := FALSE;
    want_rev := FALSE;
  END_IF;

  (* 3) 换向闭锁计时（手工 dt_s 递减） *)
  IF wait_t > 0.0 THEN
    wait_t := wait_t - dt_s;
    IF wait_t < 0.0 THEN
      wait_t := 0.0;
    END_IF;
  END_IF;

  (* 4) 输出：闭锁期间双输出强制全停 *)
  q_fwd := want_fwd AND wait_t <= 0.0;
  q_rev := want_rev AND wait_t <= 0.0;

  (* 5) 状态字 *)
  IF fault THEN
    state := 3.0;
  ELSIF wait_t > 0.0 AND (want_fwd OR want_rev) THEN
    state := 4.0;
  ELSIF q_fwd THEN
    state := 1.0;
  ELSIF q_rev THEN
    state := 2.0;
  ELSE
    state := 0.0;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_MOV — 电动阀（开/停/关三命令 + ZSO/ZSC 双限位 + 行程超时）。

  ≈ 标准电动阀块（motor-operated valve，开/停/关型，带行程监视），
  各主流 DCS/PLC 标准库均有同类块。

  用途：电动闸阀/蝶阀等"开-停-关"型电动阀的标准封装。与 FB_VALVE
  （两位阀，单输出保持）的区别：本块是双方向脉冲驱动 + 可中停，
  电机到位由限位自动停；与 FB_PID_3STEP（调节型步进）的区别：
  本块只做全开/全关/中停的顺控操作，不做闭环调节。

  行为：
    1) 故障监视：zso 与 zsc 同时 TRUE → 限位开关故障，立即 fault；
       行程（任一方向输出有效且未到目标限位）持续超过 travel_s →
       行程超时 fault（阀卡/限位丢失），同时停输出。均锁存，
       reset 且双限位条件已消除时清除（行程计时同步清零）。
    2) 命令：cmd_stop 或故障 → 双输出停（优先；cmd_open 与
       cmd_close 同拍同发视为冲突，按停处理）；cmd_open 锁存开输出、
       cmd_close 锁存关输出（互斥，新命令即新行程，行程计时清零）。
    3) 到位自停：q_open 期间 zso 到 → 停开输出；q_close 期间 zsc
       到 → 停关输出。
    4) 状态字 pos_state：0 全关 / 1 开行程中 / 2 全开 / 3 关行程中 /
       4 故障；中停（无输出且不在限位）保持上一状态值。

  输入：
    cmd_open  BOOL  开阀命令
    cmd_close BOOL  关阀命令
    cmd_stop  BOOL  停止命令（优先，中停）
    zso       BOOL  开到位限位
    zsc       BOOL  关到位限位
    travel_s  REAL  行程超时 s（<= 0 = 不监视）
    reset     BOOL  故障复位
    dt_s      REAL  采样周期 s
  输出：
    q_open    BOOL  开方向输出
    q_close   BOOL  关方向输出
    pos_state REAL  0 全关 / 1 开行程中 / 2 全开 / 3 关行程中 / 4 故障
    fault     BOOL  故障（锁存）
*)
FUNCTION_BLOCK FB_MOV
  VAR_INPUT
    cmd_open  : BOOL;
    cmd_close : BOOL;
    cmd_stop  : BOOL;
    zso       : BOOL;
    zsc       : BOOL;
    travel_s  : REAL := 60.0;
    reset     : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_open    : BOOL;
    q_close   : BOOL;
    pos_state : REAL;
    fault     : BOOL;
  END_VAR
  VAR
    travel_t : REAL; (* 当前行程累计时间 s *)
  END_VAR

  (* 1) 双限位故障（立即） *)
  IF zso AND zsc THEN
    fault := TRUE; (* 双限位同时有效：限位开关故障 *)
  END_IF;

  (* 复位（双限位条件仍在时无效） *)
  IF reset AND NOT (zso AND zsc) THEN
    fault    := FALSE;
    travel_t := 0.0;
  END_IF;

  (* 2) 命令锁存：停/故障/同拍双命令 → 全停；开关互斥，新命令即新行程 *)
  IF cmd_stop OR fault OR (cmd_open AND cmd_close) THEN
    q_open  := FALSE;
    q_close := FALSE;
  ELSIF cmd_open THEN
    IF NOT q_open THEN
      travel_t := 0.0; (* 新行程：计时清零 *)
    END_IF;
    q_open  := TRUE;
    q_close := FALSE;
  ELSIF cmd_close THEN
    IF NOT q_close THEN
      travel_t := 0.0;
    END_IF;
    q_close := TRUE;
    q_open  := FALSE;
  END_IF;

  (* 3) 到位自停 *)
  IF q_open AND zso THEN
    q_open := FALSE;
  END_IF;
  IF q_close AND zsc THEN
    q_close := FALSE;
  END_IF;

  (* 4) 行程超时（手工 dt_s 累加计时） *)
  IF q_open OR q_close THEN
    travel_t := travel_t + dt_s;
    IF travel_s > 0.0 AND travel_t >= travel_s THEN
      fault   := TRUE; (* 行程超时：阀卡/限位丢失 *)
      q_open  := FALSE;
      q_close := FALSE;
    END_IF;
  ELSE
    travel_t := 0.0;
  END_IF;

  (* 5) 状态字（中停保持上一状态值） *)
  IF fault THEN
    pos_state := 4.0;
  ELSIF zso THEN
    pos_state := 2.0;
  ELSIF zsc THEN
    pos_state := 0.0;
  ELSIF q_open THEN
    pos_state := 1.0;
  ELSIF q_close THEN
    pos_state := 3.0;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_DOSE — 定量下料/定量给料（粗/精两段 + 落差补偿）。

  ≈ 标准定量加料块（dosing / batch weighing，coarse-fine 两速加料
  + in-flight 落差提前关），配料秤/定量给料控制的惯用块。

  用途：料仓向秤斗/反应釜定量加料：粗加料快速接近目标，精加料
  收口，按落差（关阀后仍在途的物料）提前关精阀，沉降后给出
  完成与过冲量。流量输入可来自称重微分或流量计。

  行为：
    start 上升沿（空闲且未完成、abort 未压下时）开新批：qty 清零，
    进入加料段。加料/沉降期间累积 qty := qty + flow * dt_s
    （flow <= 0 不计，落料完成的判据之一）。
      粗段：qty < target - fine_qty → q_coarse = TRUE（粗精两阀同开）；
      精段：qty < target - inflight → q_fine = TRUE；到点关精阀
      （落差 inflight 即在途量预留），进入沉降段。
      沉降段：等 flow <= 0 或固定 2 s（先到为准）→ done := TRUE，
      overshoot := qty - target（在途料落完后的实际过冲）。
    abort 电平有效：立即停双阀、退出加料/沉降，qty 保留（盘点用），
    done 不置位。
    done 后 start 无效，须 reset（电平有效）清 qty/done/overshoot
    开下一批。
    假定 fine_qty >= inflight >= 0（粗阀先关、精阀后关）；
    done 之后的滴漏不再计入 qty。
    dt_s <= 0 → 本周期保持不动作（命令沿照常记忆）。

  输入：
    start    BOOL  启动（上升沿开新批）
    abort    BOOL  中止（电平有效：停阀保量）
    flow     REAL  瞬时流量 单位/s（称重微分或流量计）
    target   REAL  目标量（与 flow 积分同单位）
    fine_qty REAL  精加料段量（剩这么多时关粗阀）
    inflight REAL  落差/在途量（剩这么多时关精阀）
    dt_s     REAL  采样周期 s
    reset    BOOL  批复位（电平有效：清 qty/done/overshoot）
  输出：
    q_coarse  BOOL  粗加料阀
    q_fine    BOOL  精加料阀
    qty       REAL  本批累计量
    done      BOOL  本批完成（沉降后置位，保持到 reset）
    overshoot REAL  完成后实际 - 目标（落差整定依据）
*)
FUNCTION_BLOCK FB_DOSE
  VAR_INPUT
    start    : BOOL;
    abort    : BOOL;
    flow     : REAL;
    target   : REAL := 100.0;
    fine_qty : REAL := 10.0;
    inflight : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL;
  END_VAR
  VAR_OUTPUT
    q_coarse  : BOOL;
    q_fine    : BOOL;
    qty       : REAL;
    done      : BOOL;
    overshoot : REAL;
  END_VAR
  VAR
    dosing     : BOOL; (* 加料段（粗/精） *)
    settling   : BOOL; (* 沉降等待段 *)
    settle_t   : REAL; (* 沉降计时 s *)
    prev_start : BOOL; (* 启动上升沿记忆 *)
  END_VAR

  IF reset THEN
    dosing     := FALSE;
    settling   := FALSE;
    settle_t   := 0.0;
    qty        := 0.0;
    done       := FALSE;
    overshoot  := 0.0;
    q_coarse   := FALSE;
    q_fine     := FALSE;
    prev_start := start;
    RETURN;
  END_IF;

  IF abort THEN
    (* 中止：停阀退段，qty 保留，done 不置位 *)
    dosing   := FALSE;
    settling := FALSE;
    q_coarse := FALSE;
    q_fine   := FALSE;
  END_IF;

  (* start 上升沿且空闲且未完成 → 开新批 *)
  IF start AND NOT prev_start AND NOT abort
     AND NOT dosing AND NOT settling AND NOT done THEN
    dosing    := TRUE;
    qty       := 0.0;
    overshoot := 0.0;
    settle_t  := 0.0;
  END_IF;
  prev_start := start;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  (* 加料/沉降期间累积流量（负流量不计） *)
  IF (dosing OR settling) AND flow > 0.0 THEN
    qty := qty + flow * dt_s;
  END_IF;

  IF dosing THEN
    q_coarse := qty < target - fine_qty;
    q_fine   := qty < target - inflight;
    IF NOT q_fine THEN
      (* 到落差预留点：关精阀进沉降段 *)
      q_coarse := FALSE;
      dosing   := FALSE;
      settling := TRUE;
      settle_t := 0.0;
    END_IF;
  END_IF;

  IF settling THEN
    settle_t := settle_t + dt_s;
    IF flow <= 0.0 OR settle_t >= 2.0 THEN
      settling  := FALSE;
      done      := TRUE;
      overshoot := qty - target;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_FLOW_COMP — 气体流量温压补偿（理想气体近似）。

  ≈ 标准温压补偿块（pressure/temperature compensation for gas
  flow），气体流量折标况/折设计条件的惯用块。

  用途：按工况压力/温度把气体流量修正到参比（设计）条件：
  设计条件标定的 DP 流量计在工况偏离设计点时的读数修正、
  体积式流量折标况量。补偿比：
    ratio := (p_abs / p_ref) * (t_ref_k / t_k)   —— 密度比（理想气体）
    p_abs := p_gauge_kpa + atm_kpa（表压 → 绝压）
    t_k   := t_c + 273.15
  行为（按 mode 选流量计类型）：
    mode = 0（开方式 DP 流量计，差压开方后的流量信号）：
      q_comp := q_raw * SQRT(ratio)
      ——DP 流量 ∝ SQRT(密度)，密度偏离设计值按开方修正。
    mode = 1（线性体积式，工况体积流量折参比条件）：
      q_comp := q_raw * ratio
    防护：t_k <= 0、t_ref_k <= 0、p_ref_kpa_abs <= 0 或 p_abs <= 0
    （测量/组态非法）→ q_comp := q_raw 直通。
    纯组合逻辑，无内部状态。
    范围说明：理想气体近似（不含压缩因子/湿度修正）；贸易交接级
    的全规格能量计量标准算法不在本块范围。
    参比条件默认 p_ref = 201.325 kPa(a)（即 100 kPa 表压 + 标准
    大气压）、t_ref = 20 ℃——按流量计设计数据单整定。

  输入：
    q_raw         REAL  原始流量（工况读数）
    p_gauge_kpa   REAL  工况表压 kPa(g)
    t_c           REAL  工况温度 ℃
    p_ref_kpa_abs REAL  参比绝压 kPa(a)（设计条件）
    t_ref_c       REAL  参比温度 ℃（设计条件）
    atm_kpa       REAL  当地大气压 kPa（表压→绝压换算）
    mode          REAL  0 = 开方式 DP（乘 SQRT(ratio)）/
                        1 = 线性体积式（乘 ratio）
  输出：
    q_comp REAL  补偿后的流量
*)
FUNCTION_BLOCK FB_FLOW_COMP
  VAR_INPUT
    q_raw         : REAL;
    p_gauge_kpa   : REAL;
    t_c           : REAL;
    p_ref_kpa_abs : REAL := 201.325;
    t_ref_c       : REAL := 20.0;
    atm_kpa       : REAL := 101.325;
    mode          : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    q_comp : REAL;
  END_VAR
  VAR
    p_abs_kpa : REAL; (* 工况绝压 *)
    t_k       : REAL; (* 工况热力学温度 K *)
    t_ref_k   : REAL; (* 参比热力学温度 K *)
    ratio     : REAL; (* 密度比（理想气体） *)
  END_VAR

  p_abs_kpa := p_gauge_kpa + atm_kpa;
  t_k := t_c + 273.15;
  t_ref_k := t_ref_c + 273.15;

  IF (t_k <= 0.0) OR (t_ref_k <= 0.0)
     OR (p_ref_kpa_abs <= 0.0) OR (p_abs_kpa <= 0.0) THEN
    q_comp := q_raw; (* 测量/组态非法：直通 *)
    RETURN;
  END_IF;

  ratio := (p_abs_kpa / p_ref_kpa_abs) * (t_ref_k / t_k);

  IF mode >= 0.5 THEN
    q_comp := q_raw * ratio;       (* 1 = 线性体积式 *)
  ELSE
    q_comp := q_raw * SQRT(ratio); (* 0 = 开方式 DP *)
  END_IF;
END_FUNCTION_BLOCK
(* ---------------------------------------------------------------------------
   PROGRAM demo_main — 接线层
--------------------------------------------------------------------------- *)
PROGRAM demo_main
  VAR
    (* —— 现场信号（经 iomap 绑定；演示给初值） —— *)
    lt0101_raw    : REAL := 13824.0; (* 液位变送器原始计数（4-20mA → 5530..27648） *)
    level_sp      : REAL := 60.0;    (* 液位设定 % *)
    alm_ack       : BOOL := FALSE;   (* 报警确认按钮 *)
    hammer_enable : BOOL := TRUE;    (* 空气炮投用 *)
    gas_flow      : REAL := 1200.0;  (* 进气流量 m3/h（主动量 FT0201） *)
    milk_flow     : REAL := 950.0;   (* 石灰乳流量 m3/h（从动 pv，FT0202） *)
    milk_ratio    : REAL := 0.85;    (* 石灰乳/进气 配比 *)
    pump_run_fb_a : BOOL := FALSE;   (* 1# 石灰乳泵运行反馈 *)
    pump_run_fb_b : BOOL := FALSE;   (* 2# 石灰乳泵运行反馈 *)
    pump_fault_a  : BOOL := FALSE;   (* 1# 泵故障（综保/热继） *)
    pump_fault_b  : BOOL := FALSE;   (* 2# 泵故障 *)
    temp_hh       : BOOL := FALSE;   (* 塔温高高（外部报警位） *)
    ilk_reset     : BOOL := FALSE;   (* 联锁复位按钮 *)
    rt_reset      : BOOL := FALSE;   (* 运行小时清零（检修后） *)

    scan_s : REAL := 0.1; (* 采样周期 s，须与 tasks.toml interval_ms 一致 *)

    (* —— FB 实例 —— *)
    scale_lt      : FB_SCALE;
    pid_level     : FB_PID;
    alm_level     : FB_ALARM_HL;
    pulser_hammer : FB_PULSER;
    fill_ctl      : FB_HILO_FILL;
    ilk_tower     : FB_INTERLOCK8;
    rt_pump_a     : FB_RUNTIME;
    rt_pump_b     : FB_RUNTIME;
    duty_pumps    : FB_DUTY2;
    ratio_milk    : FB_RATIO;
    pid_milk      : FB_PID;
    split_milk    : FB_SPLIT_RANGE;

    (* —— 输出/中间变量（经 iomap 绑定到 AO/DO） —— *)
    level_pct   : REAL; (* 塔液位 % *)
    lt_fault    : BOOL; (* 变送器断线 *)
    valve_pct   : REAL; (* 进料调节阀开度 % *)
    lv_any_alm  : BOOL; (* 任一液位报警 *)
    hammer_q    : BOOL; (* 空气炮电磁阀 *)
    fill_open   : BOOL; (* 补水需求（高位关低位开） *)
    ilk_trip    : BOOL; (* 塔联锁动作 *)
    ilk_first   : INT;  (* 联锁首出路号 *)
    pump_cmd_a  : BOOL; (* 1# 泵启动指令 *)
    pump_cmd_b  : BOOL; (* 2# 泵启动指令 *)
    duty_a      : BOOL; (* 当前值班泵为 1# *)
    milk_sp     : REAL; (* 配比后的石灰乳流量设定 m3/h *)
    milk_dev    : REAL; (* 石灰乳回路工作偏差（经 SP 斜坡后） *)
    valve_small : REAL; (* 石灰乳小阀开度 %（分程 A 段，细调） *)
    valve_big   : REAL; (* 石灰乳大阀开度 %（分程 B 段，扩量程） *)

    (* —— 数学/动态块演示：蒸汽加热级联 + FOPDT 对象仿真（第 12~16 节） —— *)
    cas_enable     : BOOL := TRUE;  (* 级联投入开关 *)
    temp_sp        : REAL := 75.0;  (* 塔温设定 ℃（级联主回路 SP） *)
    steam_local_sp : REAL := 800.0; (* 非级联时操作员给的蒸汽流量本地 SP kg/h *)
    stats_reset    : BOOL := FALSE; (* 统计/偏差积分清零按钮 *)

    dly_obj      : FB_DEADTIME8; (* 对象纯滞后 20 s *)
    mid_temp     : FB_MID3;      (* 塔温三取中（两好一坏） *)
    lim_temp     : FB_LIMIT;     (* 表决塔温限幅 0..150 ℃ *)
    deriv_temp   : FB_DERIV;     (* 塔温变化率 *)
    casc_temp    : FB_CASCADE;   (* 级联耦合：主输出 → 蒸汽 SP *)
    pid_tc       : FB_PID;       (* 级联主回路：塔温 *)
    pid_fc_steam : FB_PID;       (* 级联副回路：蒸汽流量 *)
    integ_dev    : FB_INTEG;     (* 主回路偏差积分 *)
    stats_temp   : FB_STATS;     (* 塔温全程统计 *)
    avgt_temp    : FB_AVG_TIME;  (* 塔温 60 s 翻转窗平均 *)

    sim_state     : REAL := 40.0; (* FOPDT 对象一阶状态（PT1 手工离散，T = 30 s） *)
    temp_a        : REAL;         (* 1# 塔温变送器 ℃（对象仿真输出） *)
    temp_b        : REAL;         (* 2# 冗余测点 ℃（仿真 +0.5 ℃ 偏置） *)
    temp_voted    : REAL;         (* 表决 + 限幅后的塔温 ℃ *)
    temp_roc      : REAL;         (* 塔温变化率 ℃/s *)
    steam_sp_used : REAL;         (* 蒸汽副回路工作 SP kg/h（喂回 slave_sp_now） *)
    steam_flow    : REAL;         (* 蒸汽流量 pv kg/h（仿真静态阀特性） *)
    steam_valve   : REAL;         (* 蒸汽阀开度 %（副回路输出） *)
    temp_dev_int  : REAL;         (* 塔温偏差积分 ℃·s *)

    (* —— 0.3.0 扩展块演示：程序给定 + 位置式 PID 链、设备层、补偿
          （第 17~26 节） —— *)
    rs_run       : BOOL := TRUE;  (* 程序给定运行 *)
    rs_hold      : BOOL := FALSE; (* 程序给定暂停 *)
    rs_advance   : BOOL := FALSE; (* 程序给定跳段按钮 *)
    batch_reset  : BOOL := FALSE; (* 程序/批次复位按钮 *)
    msta_auto    : BOOL := TRUE;  (* 手操站自动投入 *)
    msta_man_out : REAL := 0.0;   (* 手操站手动给定 % *)
    agit_fwd_btn : BOOL := FALSE; (* 搅拌正转按钮 *)
    agit_rev_btn : BOOL := FALSE; (* 搅拌反转按钮 *)
    dose_start   : BOOL := FALSE; (* 下料启动按钮 *)
    dose_abort   : BOOL := FALSE; (* 下料中止按钮 *)

    hold_temp  : FB_PV_HOLD;     (* 表决塔温坏值保持 *)
    rs_temp    : FB_RAMPSOAK8;   (* 升温-保温程序给定 *)
    gsch_temp  : FB_GAIN_SCHED3; (* 三区增益调度 *)
    pidp_temp  : FB_PID_POS;     (* 位置式塔温 PID（影子回路） *)
    msta_temp  : FB_MANSTATION;  (* 加热输出手操站 *)
    pid3s_temp : FB_PID_3STEP;   (* 步进 PID（电动执行机构，观察用） *)
    mrev_agit  : FB_MOTOR_REV;   (* 双向搅拌电机 *)
    mov_fill   : FB_MOV;         (* 电动补水阀 *)
    dose_lime  : FB_DOSE;        (* 石灰石定量下料 *)
    comp_gas   : FB_FLOW_COMP;   (* 进气温压补偿 *)

    temp_held    : REAL;          (* 坏值保持后的塔温 ℃ *)
    rs_sp        : REAL;          (* 程序给定 SP ℃ *)
    heat_out_pos : REAL;          (* 位置式 PID 输出 % *)
    heat_out     : REAL;          (* 手操站后的最终加热输出 % *)
    agit_fb_f    : BOOL;          (* 搅拌正转反馈（输出回送仿真） *)
    agit_fb_r    : BOOL;          (* 搅拌反转反馈 *)
    mov_sim_pos  : REAL := 0.0;   (* 电动阀仿真行程 0..100 % *)
    mov_zso      : BOOL := FALSE; (* 电动阀开到位（仿真） *)
    mov_zsc      : BOOL := TRUE;  (* 电动阀关到位（仿真，初始全关） *)
    mov_q_open   : BOOL;          (* 电动阀开输出（局部拷贝，坑 8） *)
    mov_q_close  : BOOL;          (* 电动阀关输出 *)
    dose_flow    : REAL := 0.0;   (* 下料仿真流量 kg/s *)
    dose_q_coar  : BOOL;          (* 粗加料阀（局部拷贝，坑 8） *)
    dose_q_fine  : BOOL;          (* 精加料阀 *)
    gas_flow_std : REAL;          (* 温压补偿后的标况进气量 m3/h *)
  END_VAR

  (* 1) 量程变换：原始计数 → 液位 % *)
  scale_lt(raw := lt0101_raw, raw_min := 5530.0, raw_max := 27648.0,
           out_min := 0.0, out_max := 100.0, clamp := TRUE);
  level_pct := scale_lt.out;
  lt_fault  := scale_lt.fault;

  (* 2) 四级液位报警，2% 死区、3 s 延时 *)
  alm_level(pv := level_pct, h_sp := 80.0, l_sp := 20.0,
            hh_sp := 90.0, ll_sp := 10.0,
            deadband := 2.0, delay_s := 3.0, dt_s := scan_s, ack := alm_ack);
  lv_any_alm := alm_level.any_alm;

  (* 3) 液位 PID（反作用：液位高 → 关小进料阀）；变送器断线时切手动落 0。
        注意：保持旧版签名调用——验证 FB_PID 升级后的向后兼容 *)
  pid_level(pv := level_pct, sp := level_sp,
            kp := 2.0, ti_s := 120.0, td_s := 0.0, dt_s := scan_s,
            direct := FALSE, auto := NOT lt_fault, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0);
  valve_pct := pid_level.out;

  (* 4) 空气炮：每 300 s 吹扫 2 s *)
  pulser_hammer(enable := hammer_enable, period_s := 300.0,
                pulse_len_s := 2.0, dt_s := scan_s);
  hammer_q := pulser_hammer.q;

  (* 5) 补水需求：低于 15% 要水，高于 85% 停 *)
  fill_ctl(pv := level_pct, h_sp := 85.0, l_sp := 15.0);
  fill_open := fill_ctl.valve_open;

  (* 6) 塔联锁汇总 + 首出：1=液位HH 2=变送器断线 3=塔温HH 4=双泵故障；
        5..8 备用旁路 *)
  ilk_tower(c1 := alm_level.hh_alm,
            c2 := lt_fault,
            c3 := temp_hh,
            c4 := pump_fault_a AND pump_fault_b,
            enable5 := FALSE, enable6 := FALSE,
            enable7 := FALSE, enable8 := FALSE,
            reset := ilk_reset);
  ilk_trip  := ilk_tower.tripped;
  ilk_first := ilk_tower.first_out;

  (* 7) 双泵运行统计（hours 喂 FB_DUTY2 小时均衡；实例可放 VAR RETAIN） *)
  rt_pump_a(running := pump_run_fb_a, reset := rt_reset, dt_s := scan_s);
  rt_pump_b(running := pump_run_fb_b, reset := rt_reset, dt_s := scan_s);

  (* 8) 双补水泵一用一备：小时均衡选值班；联锁动作时撤需求全停 *)
  duty_pumps(demand := fill_open AND NOT ilk_trip, alt_mode := 1,
             run_fb_a := pump_run_fb_a, run_fb_b := pump_run_fb_b,
             fault_a := pump_fault_a, fault_b := pump_fault_b,
             hours_a := rt_pump_a.hours, hours_b := rt_pump_b.hours,
             dt_s := scan_s);
  pump_cmd_a := duty_pumps.cmd_a;
  pump_cmd_b := duty_pumps.cmd_b;
  duty_a     := duty_pumps.duty_is_a;

  (* 9) 比值站：石灰乳流量设定 = 进气量 × 0.85（比值钳 0.5..1.2） *)
  ratio_milk(wild_flow := gas_flow, ratio := milk_ratio,
             ratio_min := 0.5, ratio_max := 1.2, bias := 0.0);
  milk_sp := ratio_milk.sp;

  (* 10) 石灰乳流量 PID：进气量静态前馈（增益 0.01 → %开度），
         SP 斜坡 50 (m3/h)/s 防配比阶跃，联锁动作 track 强制关阀 *)
  pid_milk(pv := milk_flow, sp := milk_sp,
           kp := 0.8, ti_s := 30.0, td_s := 0.0, dt_s := scan_s,
           direct := FALSE, auto := TRUE, manual_out := 0.0,
           out_min := 0.0, out_max := 100.0,
           ff := gas_flow * 0.01,
           track := ilk_trip, track_value := 0.0,
           sp_rate := 50.0);
  milk_dev := pid_milk.dev;

  (* 11) 分程：PID 输出 0..50% 走小阀细调，50..100% 大阀接力（顺序分程，
         不反转） *)
  split_milk(u := pid_milk.out, split := 50.0, reverse_b := FALSE);
  valve_small := split_milk.out_a;
  valve_big   := split_milk.out_b;

  (* 12) FOPDT 对象仿真：蒸汽阀开度（上一拍）→ FB_DEADTIME8 纯滞后 20 s →
         手工 PT1（T = 30 s，与 FB_LAG 同核）→ 塔温。纯滞后 + 一阶惯性
         两块即组成最常用的仿真/整定对象 *)
  dly_obj(u := steam_valve, t_delay_s := 20.0, dt_s := scan_s);
  sim_state := sim_state + scan_s / (30.0 + scan_s) * (dly_obj.out - sim_state);
  temp_a := 40.0 + 0.5 * sim_state; (* 0..100 % 阀开度 → 40..90 ℃ *)
  temp_b := temp_a + 0.5;           (* 2# 冗余测点：演示 +0.5 ℃ 偏置 *)

  (* 13) 三取中表决：3# 测点坏值被剔除（good_c := FALSE）→ 两好取均值；
         好通道差 > 5 ℃ 出 q_dev 提示 *)
  mid_temp(a := temp_a, b := temp_b, c := 0.0,
           good_a := TRUE, good_b := TRUE, good_c := FALSE,
           dev_max := 5.0);

  (* 14) 表决值限幅 0..150 ℃ + 滤波微分（变化率 ℃/s 供趋势/前馈用） *)
  lim_temp(u := mid_temp.out, lo := 0.0, hi := 150.0);
  temp_voted := lim_temp.out;
  deriv_temp(u := temp_voted, t_filt_s := 2.0, dt_s := scan_s);
  temp_roc := deriv_temp.out;

  (* 15) 级联：塔温主 PID（0..100 %）→ FB_CASCADE 映射为蒸汽流量 SP
         （0..2000 kg/h）→ 蒸汽流量副 PID → 蒸汽阀。非级联（cas_enable
         = FALSE）时副回路用本地 SP，主调经 track 跟踪其等效百分比，
         CAS 投切无扰。slave_sp_now 喂上一拍工作 SP（标准接法） *)
  casc_temp(master_out := pid_tc.out, cas_on := cas_enable,
            sp_min := 0.0, sp_max := 2000.0,
            slave_sp_local := steam_local_sp, slave_sp_now := steam_sp_used);
  steam_sp_used := casc_temp.slave_sp;
  pid_tc(pv := temp_voted, sp := temp_sp,
         kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
         direct := FALSE, auto := TRUE, manual_out := 0.0,
         out_min := 0.0, out_max := 100.0,
         track := casc_temp.master_track,
         track_value := casc_temp.master_track_value);
  pid_fc_steam(pv := steam_flow, sp := steam_sp_used,
               kp := 0.5, ti_s := 10.0, td_s := 0.0, dt_s := scan_s,
               direct := FALSE, auto := TRUE, manual_out := 0.0,
               out_min := 0.0, out_max := 100.0);
  steam_valve := pid_fc_steam.out;
  steam_flow := steam_valve * 20.0; (* 仿真静态阀特性：0..100 % → 0..2000 kg/h *)

  (* 16) 共用信号（表决塔温）统计三件套 + 偏差积分：全程 min/max/均值/
         标准差、60 s 翻转窗平均；主回路偏差积分 ℃·s（联锁动作时 hold，
         钳 ±1000） *)
  stats_temp(u := temp_voted, enable := TRUE, reset := stats_reset);
  avgt_temp(u := temp_voted, window_s := 60.0, dt_s := scan_s, reset := stats_reset);
  integ_dev(u := pid_tc.dev, k := 1.0, dt_s := scan_s,
            hold := ilk_trip, reset := stats_reset,
            preset := FALSE, preset_value := 0.0,
            out_min := -1000.0, out_max := 1000.0);
  temp_dev_int := integ_dev.out;

  (* 17) 坏值保持：表决全坏（q_fail）时先保持最后好值 5 s，超时后继续
         保持（mode 0），q_bad/q_timeout 供报警与回路切手动 *)
  hold_temp(u := mid_temp.out, good := NOT mid_temp.q_fail,
            mode := 0.0, sub_value := 60.0, timeout_s := 5.0,
            dt_s := scan_s);
  temp_held := hold_temp.out;

  (* 18) 8 段程序给定（用 4 段）：40 ℃ 起升温-保温-降温曲线；
         hold 暂停、advance 跳段、batch_reset 复位 *)
  rs_temp(sp1 := 55.0, ramp1_s := 120.0, soak1_s := 60.0,
          sp2 := 70.0, ramp2_s := 180.0, soak2_s := 120.0,
          sp3 := 85.0, ramp3_s := 120.0, soak3_s := 300.0,
          sp4 := 40.0, ramp4_s := 240.0, soak4_s := 0.0,
          n_seg := 4.0, run := rs_run, hold := rs_hold,
          advance := rs_advance, reset := batch_reset,
          cyclic := FALSE, sp_init := 40.0, dt_s := scan_s);
  rs_sp := rs_temp.sp;

  (* 19) 三区增益调度：按保持后塔温分区出 PID 参数（低温区强、
         高温区弱），区界 ±2 ℃ 线性过渡（interp） *)
  gsch_temp(u_sched := temp_held, x12 := 55.0, x23 := 75.0,
            hyst := 2.0, interp := TRUE,
            kp1 := 2.0, ti1_s := 90.0, td1_s := 0.0,
            kp2 := 1.5, ti2_s := 60.0, td2_s := 0.0,
            kp3 := 1.0, ti3_s := 45.0, td3_s := 0.0);

  (* 20) 位置式 PID（影子塔温回路）：程序给定 SP + 坏值保持 PV +
         调度参数，p/i/d 分项可观测；联锁 track 强制 0 *)
  pidp_temp(pv := temp_held, sp := rs_sp,
            kp := gsch_temp.kp_out, ti_s := gsch_temp.ti_out_s,
            td_s := gsch_temp.td_out_s, t_dfilt_s := 2.0,
            sp_weight_b := 1.0, dt_s := scan_s,
            direct := FALSE, auto := TRUE, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0,
            ff := 0.0, track := ilk_trip, track_value := 0.0);
  heat_out_pos := pidp_temp.out;

  (* 21) 手操站：自动透传位置式 PID 输出（限速 20 %/s），手动直给；
         balance 切回自动无扰 *)
  msta_temp(auto_in := heat_out_pos, auto := msta_auto,
            manual_out := msta_man_out, bias := 0.0, rate := 20.0,
            out_min := 0.0, out_max := 100.0, balance := TRUE,
            dt_s := scan_s);
  heat_out := msta_temp.out;

  (* 22) 步进 PID（观察用）：同回路驱动无阀位反馈的电动执行机构，
         全行程 60 s，最小脉宽 0.5 s、死区 1 %，行程估计 pos_est *)
  pid3s_temp(pv := temp_held, sp := rs_sp,
             kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
             motor_time_s := 60.0, min_pulse_s := 0.5,
             deadband_pct := 1.0, auto := TRUE,
             man_open := FALSE, man_close := FALSE, pos_fb := -1.0);

  (* 23) 双向搅拌电机：正/反按钮，联锁停车优先，换向闭锁 2 s；
         反馈用输出回送仿真（一拍滞后，永不超时） *)
  mrev_agit(cmd_fwd := agit_fwd_btn, cmd_rev := agit_rev_btn,
            stop := ilk_trip, fb_fwd := agit_fb_f, fb_rev := agit_fb_r,
            fb_timeout_s := 5.0, reversal_delay_s := 2.0,
            reset := ilk_reset, dt_s := scan_s);
  agit_fb_f := mrev_agit.q_fwd; (* 仿真反馈：输出回送（下一拍生效） *)
  agit_fb_r := mrev_agit.q_rev;

  (* 24) 电动补水阀：补水需求开/撤销关（联锁强制关），30 s 仿真行程
         驱动双限位；行程超时限 60 s *)
  mov_fill(cmd_open := fill_open AND NOT ilk_trip,
           cmd_close := NOT fill_open OR ilk_trip,
           cmd_stop := FALSE, zso := mov_zso, zsc := mov_zsc,
           travel_s := 60.0, reset := ilk_reset, dt_s := scan_s);
  mov_q_open  := mov_fill.q_open;  (* 先拷局部再作 IF 条件（坑 8） *)
  mov_q_close := mov_fill.q_close;
  IF mov_q_open THEN
    mov_sim_pos := mov_sim_pos + 100.0 * scan_s / 30.0; (* 30 s 全行程仿真 *)
  ELSIF mov_q_close THEN
    mov_sim_pos := mov_sim_pos - 100.0 * scan_s / 30.0;
  END_IF;
  IF mov_sim_pos > 100.0 THEN
    mov_sim_pos := 100.0;
  ELSIF mov_sim_pos < 0.0 THEN
    mov_sim_pos := 0.0;
  END_IF;
  mov_zso := mov_sim_pos >= 100.0;
  mov_zsc := mov_sim_pos <= 0.0;

  (* 25) 定量下料：目标 100 kg，精段 15 kg、落差 2 kg；仿真流量
         粗 8 / 精 1.5 kg/s（上一拍阀态回送，含一拍滞后） *)
  dose_lime(start := dose_start, abort := dose_abort,
            flow := dose_flow, target := 100.0,
            fine_qty := 15.0, inflight := 2.0,
            dt_s := scan_s, reset := batch_reset);
  dose_q_coar := dose_lime.q_coarse; (* 先拷局部再作 IF 条件（坑 8） *)
  dose_q_fine := dose_lime.q_fine;
  dose_flow := 0.0;
  IF dose_q_coar THEN
    dose_flow := 8.0;
  ELSIF dose_q_fine THEN
    dose_flow := 1.5;
  END_IF;

  (* 26) 气体温压补偿：进气量按工况 150 kPa(g)/45 ℃ 折设计条件
         （DP 开方式，mode 0）→ 标况进气量 *)
  comp_gas(q_raw := gas_flow, p_gauge_kpa := 150.0, t_c := 45.0,
           p_ref_kpa_abs := 201.325, t_ref_c := 20.0,
           atm_kpa := 101.325, mode := 0.0);
  gas_flow_std := comp_gas.q_comp;
END_PROGRAM
