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
