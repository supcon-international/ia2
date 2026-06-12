(*
  FB_RATE_LIMIT — 变化率限幅器（跟随输入的斜率钳位）。

  ≈ 标准变化率限幅器（velocity/slew limiter）。

  用途：限制连续变化信号的每秒变化量：阀位指令防抖、变频给定缓变、
  前馈量平滑。与 FB_RAMP 的区别：本块跟随持续变化的输入 u（无目标概念），
  只钳 |Δ|/s；RAMP 朝固定 target 走。

  行为：
    delta := u - out 钳位到 ±rate_s*dt_s 后累加：out := out + delta_clamped。
    rate_s <= 0 → 直通；dt_s <= 0 → 保持；
    reset 或首扫描 → out := u（对中，避免投运从 0 全速爬升）。

  输入：
    u      REAL  输入
    rate_s REAL  最大变化率 |单位|/s（0 = 直通）
    dt_s   REAL  采样周期 s
    reset  BOOL  复位/对中（电平有效：out 跟随 u）
  输出：
    out    REAL  限速输出
*)
FUNCTION_BLOCK FB_RATE_LIMIT
  VAR_INPUT
    u      : REAL;
    rate_s : REAL := 0.0;
    dt_s   : REAL := 0.1;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    delta     : REAL;
    max_step  : REAL;
    init_done : BOOL;
  END_VAR

  IF reset OR NOT init_done THEN
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF rate_s <= 0.0 THEN
    out := u; (* 直通 *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 保持 *)
  END_IF;

  delta := u - out;
  max_step := rate_s * dt_s;
  IF delta > max_step THEN
    delta := max_step;
  ELSIF delta < 0.0 - max_step THEN
    delta := 0.0 - max_step;
  END_IF;
  out := out + delta;
END_FUNCTION_BLOCK
