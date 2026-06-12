(* ============================================================================
  demo_main — 迷你碳化塔回路演示 PROGRAM（自包含文件）。

  注意：本文件内联了它实例化的 10 个 FB 声明的逐字拷贝（FB_SCALE / FB_PID /
  FB_ALARM_HL / FB_PULSER / FB_HILO_FILL / FB_RATIO / FB_SPLIT_RANGE /
  FB_RUNTIME / FB_DUTY2 / FB_INTERLOCK8）。原因：`cs check` 对每个文件独立
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
END_PROGRAM
