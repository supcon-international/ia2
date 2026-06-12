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
