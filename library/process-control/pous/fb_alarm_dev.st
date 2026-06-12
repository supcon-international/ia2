(*
  FB_ALARM_DEV — 偏差报警（pv 对 sp 的上/下偏差），死区 + 延时 + 确认锁存。

  ≈ 偏差报警（deviation alarm，上/下偏差），回路监视标配。

  用途：回路性能监视：控制偏差持续超限报警（阀卡死、负荷骤变、
  整定失效、执行机构失灵）。
  行为（沿用 FB_ALARM_HL 的死区+延时+锁存范式）：
    dev := pv - sp。
    上偏差：dev > dev_h 置位；dev < dev_h - deadband 复位。
    下偏差：dev < -dev_l 置位；dev > -dev_l + deadband 复位（dev_l 传正数）。
    各自连续保持 delay_s 后触发；输出 = 触发 OR (已锁存 AND NOT ack)
    ——条件在则常报，条件消失后保持到确认。

  输入：
    pv       REAL  过程值
    sp       REAL  设定值（接 FB_PID 同源 sp；若 PID 用了 sp_rate，
                   建议接经斜坡后的实际工作 sp）
    dev_h    REAL  上偏差限（pv 高出 sp 的允许量）
    dev_l    REAL  下偏差限（pv 低于 sp 的允许量，传正数）
    deadband REAL  死区（回差，工程单位）
    delay_s  REAL  报警延时 s（0 = 立即）
    dt_s     REAL  采样周期 s
    ack      BOOL  确认（电平有效）
  输出：
    h_alm   BOOL  上偏差报警（锁存）
    l_alm   BOOL  下偏差报警（锁存）
    any_alm BOOL  任一偏差报警
*)
FUNCTION_BLOCK FB_ALARM_DEV
  VAR_INPUT
    pv       : REAL;
    sp       : REAL;
    dev_h    : REAL := 10.0;
    dev_l    : REAL := 10.0;
    deadband : REAL := 0.0;
    delay_s  : REAL := 0.0;
    dt_s     : REAL := 0.1;
    ack      : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    h_alm   : BOOL;
    l_alm   : BOOL;
    any_alm : BOOL;
  END_VAR
  VAR
    dv     : REAL; (* pv - sp *)
    h_cond : BOOL;
    l_cond : BOOL;
    h_t    : REAL; (* 延时累加器，手工计时替代 TON *)
    l_t    : REAL;
    h_trip : BOOL;
    l_trip : BOOL;
  END_VAR

  dv := pv - sp;

  (* ---- 上偏差 ---- *)
  IF dv > dev_h THEN
    h_cond := TRUE;
  ELSIF dv < dev_h - deadband THEN
    h_cond := FALSE;
  END_IF;
  IF h_cond THEN
    h_t := h_t + dt_s;
  ELSE
    h_t := 0.0;
  END_IF;
  h_trip := h_cond AND (h_t >= delay_s);
  h_alm  := h_trip OR (h_alm AND NOT ack);

  (* ---- 下偏差 ---- *)
  IF dv < 0.0 - dev_l THEN
    l_cond := TRUE;
  ELSIF dv > 0.0 - dev_l + deadband THEN
    l_cond := FALSE;
  END_IF;
  IF l_cond THEN
    l_t := l_t + dt_s;
  ELSE
    l_t := 0.0;
  END_IF;
  l_trip := l_cond AND (l_t >= delay_s);
  l_alm  := l_trip OR (l_alm AND NOT ack);

  any_alm := h_alm OR l_alm;
END_FUNCTION_BLOCK
