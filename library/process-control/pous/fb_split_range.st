(*
  FB_SPLIT_RANGE — 分程输出：一个调节器输出按 split 点分配到两个阀。

  ≈ 分程输出（split range）：一个调节输出按区段驱动两个阀（加热/冷却、大小阀）。

  用途：一个 PID 驱动两个执行机构：大小阀扩量程（顺序分程，
  reverse_b := FALSE），或加热/冷却、进料/回流互补型分程
  （reverse_b := TRUE，按执行机构气开气关与 PID 作用方向整定）。

  行为（u 先钳位 0..100，两输出亦钳位 0..100）：
    out_a：u 在 [0, split] 内 0→100 线性，u >= split 后保持 100。
    out_b：u 在 [split, 100] 内 0→100 线性，u <= split 时为 0；
           reverse_b = TRUE 时 B 段反转为 100→0（含段外端值保持，
           即 u <= split 时 out_b = 100，输出连续无跳变）。
    曲线示意（split = 50）：
      reverse_b = FALSE（大小阀顺序分程）：
        u:     0 …… 50 …… 100
        out_a: 0 → 100 ┤ 保持 100
        out_b: 0   0   ├ 0 → 100
      reverse_b = TRUE（互补型，B 先全开后渐关）：
        u:     0 …… 50 …… 100
        out_a: 0 → 100 ┤ 保持 100
        out_b: 100 100 ├ 100 → 0
    退化：split <= 0 → A 段无量程（u > 0 即 100）；split >= 100 → B 段
    无量程（u >= 100 才动作）。split 应整定在 (0, 100) 内。

  输入：
    u         REAL  调节器输出 0..100
    split     REAL  分程点 %（B 阀起点）
    reverse_b BOOL  TRUE = B 阀在其分程段内反作用（100→0）
  输出：
    out_a     REAL  A 阀开度 0..100
    out_b     REAL  B 阀开度 0..100
*)
FUNCTION_BLOCK FB_SPLIT_RANGE
  VAR_INPUT
    u         : REAL;
    split     : REAL := 50.0;
    reverse_b : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out_a : REAL;
    out_b : REAL;
  END_VAR
  VAR
    uc     : REAL; (* 钳位后的输入 *)
    span_b : REAL;
  END_VAR

  (* 输入钳位 0..100 *)
  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  (* A 阀：[0, split] 内 0→100 *)
  IF split > 0.0 THEN
    out_a := uc * 100.0 / split;
  ELSE
    (* 退化：A 无分程段 *)
    IF uc > 0.0 THEN
      out_a := 100.0;
    ELSE
      out_a := 0.0;
    END_IF;
  END_IF;
  IF out_a < 0.0 THEN
    out_a := 0.0;
  ELSIF out_a > 100.0 THEN
    out_a := 100.0;
  END_IF;

  (* B 阀：[split, 100] 内 0→100 *)
  span_b := 100.0 - split;
  IF span_b > 0.0 THEN
    out_b := (uc - split) * 100.0 / span_b;
  ELSE
    (* 退化：B 无分程段 *)
    IF uc >= split THEN
      out_b := 100.0;
    ELSE
      out_b := 0.0;
    END_IF;
  END_IF;
  IF out_b < 0.0 THEN
    out_b := 0.0;
  ELSIF out_b > 100.0 THEN
    out_b := 100.0;
  END_IF;

  (* B 段反作用（钳位后再反转，段外保持端值，输出连续） *)
  IF reverse_b THEN
    out_b := 100.0 - out_b;
  END_IF;
END_FUNCTION_BLOCK
