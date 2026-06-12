(*
  FB_LEADLAG — 超前-滞后补偿器（前馈通道动态整形）。

  ≈ 标准超前-滞后补偿（lead-lag），前馈通路动态整形的惯用块。

  用途：前馈补偿的动态匹配：让前馈量经 (1 + s*lead_s)/(1 + s*lag_s) 传递，
  补偿扰动通道与调节通道的时间差。lead_s > lag_s 超前（加速响应），
  lead_s < lag_s 滞后（缓冲）。输出常接 FB_PID 的 ff 输入。

  离散近似（标准分解：内部 PT1 + 直通混合）：
    filt := filt + dt_s / (lag_s + dt_s) * (u - filt)   —— 内部一阶滞后
    out  := u * (lead_s / lag_s) + filt * (1 - lead_s / lag_s)
  与连续传递函数对照：稳态增益 1，高频增益 lead_s/lag_s（一阶近似，
  dt_s << lag_s 时逼近连续解）。
  说明：
    lag_s <= 0 → 直通（out := u，filt 跟随）；
    lead_s = 0 → 退化为纯 PT1；lead_s = lag_s → 直通；
    lead_s/lag_s 远大于 1 时高频增益大，注意输入噪声放大；
    reset 或首扫描 → filt := u、out := u（无扰投运）；dt_s <= 0 → 保持。

  输入：
    u      REAL  输入
    lead_s REAL  超前时间常数 s
    lag_s  REAL  滞后时间常数 s（0 = 直通）
    dt_s   REAL  采样周期 s
    reset  BOOL  复位/对中（电平有效）
  输出：
    out    REAL  补偿输出
*)
FUNCTION_BLOCK FB_LEADLAG
  VAR_INPUT
    u      : REAL;
    lead_s : REAL := 0.0;
    lag_s  : REAL := 1.0;
    dt_s   : REAL := 0.1;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    filt      : REAL; (* 内部 PT1 状态 *)
    gain_hf   : REAL; (* lead_s / lag_s *)
    init_done : BOOL;
  END_VAR

  IF reset OR NOT init_done THEN
    filt := u;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF lag_s <= 0.0 THEN
    filt := u;
    out := u; (* 直通 *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 保持 *)
  END_IF;

  filt := filt + dt_s / (lag_s + dt_s) * (u - filt);
  gain_hf := lead_s / lag_s;
  out := u * gain_hf + filt * (1.0 - gain_hf);
END_FUNCTION_BLOCK
