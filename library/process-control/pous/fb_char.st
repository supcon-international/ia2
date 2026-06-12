(*
  FB_CHAR — 折线信号特性化（分段线性插值，最多 8 点）。

  ≈ 折线信号特性化（piecewise-linear characterizer），阀特性线性化/非线性传感器惯用块。

  用途：阀特性线性化（等百分比阀开度补偿）、非线性传感器标定、
  料位-体积换算等任意单调折线变换。

  行为：
    取前 n 个点 (x1,y1)..(xn,yn)（n 钳位到 2..8），u 落在段 (xk, xk+1] 内
    线性插值；u <= x1 → out := y1；u >= xn → out := yn（两端钳位）。
    约定：x 必须升序（本块不做硬校验；遇到非升序退化段取右端点 y）。
    本方言 ARRAY 在 FUNCTION_BLOCK 作用域不能 codegen（见 README 方言坑），
    故采用标量点对 x1..x8 / y1..y8 + n 的形式。

  输入：
    u          REAL  输入
    n          INT   使用点数 2..8（自动钳位）
    x1..x8     REAL  折线 x 坐标（升序）
    y1..y8     REAL  折线 y 坐标
  输出：
    out        REAL  特性化输出（两端钳位）
*)
FUNCTION_BLOCK FB_CHAR
  VAR_INPUT
    u  : REAL;
    n  : INT := 2;
    x1 : REAL := 0.0;
    y1 : REAL := 0.0;
    x2 : REAL := 100.0;
    y2 : REAL := 100.0;
    x3 : REAL := 0.0;
    y3 : REAL := 0.0;
    x4 : REAL := 0.0;
    y4 : REAL := 0.0;
    x5 : REAL := 0.0;
    y5 : REAL := 0.0;
    x6 : REAL := 0.0;
    y6 : REAL := 0.0;
    x7 : REAL := 0.0;
    y7 : REAL := 0.0;
    x8 : REAL := 0.0;
    y8 : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    nn    : INT;  (* 钳位后的点数 *)
    xa    : REAL; (* 选中段左端 *)
    ya    : REAL;
    xb    : REAL; (* 选中段右端 *)
    yb    : REAL;
    span  : REAL;
    found : BOOL;
  END_VAR

  nn := n;
  IF nn < 2 THEN
    nn := 2;
  ELSIF nn > 8 THEN
    nn := 8;
  END_IF;

  (* 低端钳位 *)
  IF u <= x1 THEN
    out := y1;
    RETURN;
  END_IF;

  (* 段查找：u 落入 (xk, xk+1] 取该段；只看前 nn 个点 *)
  found := FALSE;
  IF u <= x2 THEN
    xa := x1; ya := y1; xb := x2; yb := y2; found := TRUE;
  ELSIF nn >= 3 AND u <= x3 THEN
    xa := x2; ya := y2; xb := x3; yb := y3; found := TRUE;
  ELSIF nn >= 4 AND u <= x4 THEN
    xa := x3; ya := y3; xb := x4; yb := y4; found := TRUE;
  ELSIF nn >= 5 AND u <= x5 THEN
    xa := x4; ya := y4; xb := x5; yb := y5; found := TRUE;
  ELSIF nn >= 6 AND u <= x6 THEN
    xa := x5; ya := y5; xb := x6; yb := y6; found := TRUE;
  ELSIF nn >= 7 AND u <= x7 THEN
    xa := x6; ya := y6; xb := x7; yb := y7; found := TRUE;
  ELSIF nn >= 8 AND u <= x8 THEN
    xa := x7; ya := y7; xb := x8; yb := y8; found := TRUE;
  END_IF;

  IF found THEN
    span := xb - xa;
    IF span > 0.0 THEN
      out := ya + (u - xa) * (yb - ya) / span;
    ELSE
      out := yb; (* 退化段（x 未升序）：取右端点 *)
    END_IF;
    RETURN;
  END_IF;

  (* 高端钳位：u 大于第 nn 点 *)
  CASE nn OF
    3:
      out := y3;
    4:
      out := y4;
    5:
      out := y5;
    6:
      out := y6;
    7:
      out := y7;
    8:
      out := y8;
  ELSE
    out := y2;
  END_CASE;
END_FUNCTION_BLOCK
