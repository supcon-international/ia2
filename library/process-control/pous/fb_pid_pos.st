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
