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
