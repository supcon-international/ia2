(*
  FB_RAMP — 设定值斜坡发生器（朝固定目标限速逼近）。

  ≈ 标准设定值斜坡发生器（ramp generator）。

  用途：设定值缓变（避免阶跃冲击执行机构/工艺）、升温升压曲线分段执行。
  与 FB_RATE_LIMIT 的区别：本块朝固定 target 走并报告 ramping；
  RATE_LIMIT 连续跟随变化的输入、只钳每秒变化量。

  行为：
    out 以 rate_up（上行）/ rate_dn（下行，传正数）单位/秒逼近 target，
    到达后 ramping := FALSE。速率 = 0 表示该方向直接跳变。
    track 电平有效：out 无条件跟随 track_value 且 ramping := FALSE；
    track 撤销后从当前值续 ramp 到 target（无扰投运惯用：先 track 对中
    当前工艺值，再放开开始斜坡）。
    dt_s <= 0 → 保持。注意 out 初值为 0：投运首拍建议用 track 对中。

  输入：
    target      REAL  目标值
    rate_up     REAL  上行速率 单位/s（0 = 跳变）
    rate_dn     REAL  下行速率 单位/s（传正数；0 = 跳变）
    dt_s        REAL  采样周期 s
    track       BOOL  跟踪使能（电平有效）
    track_value REAL  跟踪值
  输出：
    out     REAL  斜坡输出
    ramping BOOL  斜坡进行中（未到 target）
*)
FUNCTION_BLOCK FB_RAMP
  VAR_INPUT
    target      : REAL;
    rate_up     : REAL := 0.0;
    rate_dn     : REAL := 0.0;
    dt_s        : REAL := 0.1;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out     : REAL;
    ramping : BOOL;
  END_VAR
  VAR
    delta : REAL;
    step  : REAL;
  END_VAR

  IF track THEN
    out := track_value; (* 无扰：撤销后从此处续 ramp *)
    ramping := FALSE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 保持 *)
  END_IF;

  delta := target - out;
  IF delta > 0.0 THEN
    step := rate_up * dt_s;
    IF rate_up <= 0.0 OR step >= delta THEN
      out := target; (* 速率 0 跳变 / 本拍可达 *)
      ramping := FALSE;
    ELSE
      out := out + step;
      ramping := TRUE;
    END_IF;
  ELSIF delta < 0.0 THEN
    step := rate_dn * dt_s;
    IF rate_dn <= 0.0 OR step >= 0.0 - delta THEN
      out := target;
      ramping := FALSE;
    ELSE
      out := out - step;
      ramping := TRUE;
    END_IF;
  ELSE
    ramping := FALSE;
  END_IF;
END_FUNCTION_BLOCK
