(*
  FB_SCALE — 线性量程变换：原始计数值 → 工程单位，含 NE43 风格断线提示。

  用途：模拟量输入标定。例如 4-20 mA 采样为原始计数（S7 风格 5530..27648），
  变换为 0..100 % 或 0..5 m 等工程值。

  公式：out := out_min + (raw - raw_min) * (out_max - out_min) / span
        其中 span = MAX(raw_max - raw_min, 1e-6)（内联 IF 实现，防除零）。
  假定 raw_max > raw_min（正向量程）。

  输入：
    raw      REAL  原始计数值（如 AI 通道原值）
    raw_min  REAL  量程下限对应的原始值（4 mA 点）
    raw_max  REAL  量程上限对应的原始值（20 mA 点）
    out_min  REAL  工程量程下限
    out_max  REAL  工程量程上限
    clamp    BOOL  TRUE 时输出钳位在 [out_min, out_max]（支持反向工程量程）
  输出：
    out      REAL  工程值
    fault    BOOL  原始值越界 >5% 量程（NE43 风格断线/超限提示，不锁存）
*)
FUNCTION_BLOCK FB_SCALE
  VAR_INPUT
    raw     : REAL;
    raw_min : REAL := 0.0;
    raw_max : REAL := 27648.0;
    out_min : REAL := 0.0;
    out_max : REAL := 100.0;
    clamp   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out   : REAL;
    fault : BOOL;
  END_VAR
  VAR
    span   : REAL;
    margin : REAL;
    lo     : REAL;
    hi     : REAL;
  END_VAR

  (* span = MAX(raw_max - raw_min, 1e-6) — 内联 IF 防除零 *)
  span := raw_max - raw_min;
  IF span < 0.000001 THEN
    span := 0.000001;
  END_IF;

  (* NE43 风格越界判定：超出原始量程 5% 以上视为断线/故障 *)
  margin := 0.05 * span;
  fault := (raw < raw_min - margin) OR (raw > raw_max + margin);

  out := out_min + (raw - raw_min) * (out_max - out_min) / span;

  IF clamp THEN
    IF out_min <= out_max THEN
      lo := out_min;
      hi := out_max;
    ELSE
      lo := out_max;
      hi := out_min;
    END_IF;
    IF out < lo THEN
      out := lo;
    ELSIF out > hi THEN
      out := hi;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
