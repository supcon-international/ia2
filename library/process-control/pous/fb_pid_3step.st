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
