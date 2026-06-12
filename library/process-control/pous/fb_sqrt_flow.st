(*
  FB_SQRT_FLOW — 差压流量开方提取。

  ≈ 差压流量开方提取（square-root extraction）+ 小流量切除，DCS 模拟量处理惯用块。

  用途：孔板/文丘里/喷嘴等差压式流量测量：流量与差压的平方根成正比。
  公式：flow := flow_max * SQRT(dp_pct / 100.0)；dp_pct <= cutoff_pct → flow := 0。
  说明：
    cutoff_pct 为小流量切除：低差压区开方增益趋于无穷，噪声放大严重，
    惯例直接切零。默认 6.25%（差压）↔ 25%（流量）切除点。
    dp_pct <= 0 按 0 处理；dp_pct > 100 不截断（允许量程外开方延伸，
    需要硬限时调用方自行钳位）。
    SQRT() 为本方言内建函数（已经 cs project check 含 codegen 验证可用）。

  输入：
    dp_pct     REAL  差压 %（0..100，常接 FB_SCALE 输出）
    flow_max   REAL  100% 差压对应的流量工程值
    cutoff_pct REAL  小流量切除阈值 %（差压百分数）
  输出：
    flow       REAL  流量工程值
*)
FUNCTION_BLOCK FB_SQRT_FLOW
  VAR_INPUT
    dp_pct     : REAL;
    flow_max   : REAL := 100.0;
    cutoff_pct : REAL := 6.25;
  END_VAR
  VAR_OUTPUT
    flow : REAL;
  END_VAR

  IF dp_pct <= cutoff_pct OR dp_pct <= 0.0 THEN
    flow := 0.0; (* 小流量切除 *)
    RETURN;
  END_IF;

  flow := flow_max * SQRT(dp_pct / 100.0);
END_FUNCTION_BLOCK
