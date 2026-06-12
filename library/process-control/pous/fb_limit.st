(*
  FB_LIMIT — 值限幅器（高低钳位 + 越限指示）。

  ≈ 标准限幅器（limiter / high-low clamp），各主流平台标准库必备。

  用途：设定值/输出值安全钳位、串级外环输出限制、操作员输入合法化、
  下游裸执行器的最后一道保护。
  行为（纯组合逻辑，无内部状态）：
    out := u 钳位到 [lo, hi]；q_hi / q_lo 指示正在钳位（u 越上/下限）。
    组态错误 lo > hi → out := (lo + hi) / 2，且 q_hi 与 q_lo 同时为 TRUE
    ——双越限位同亮在物理上不可能出现，专作组态错误信号，便于上位
    报警捕捉。

  输入：
    u  REAL  输入
    lo REAL  下限
    hi REAL  上限（假定 lo <= hi；设反触发组态错误信号）
  输出：
    out  REAL  钳位输出
    q_hi BOOL  越上限（正在按 hi 钳位）
    q_lo BOOL  越下限（正在按 lo 钳位；与 q_hi 同时 TRUE = lo > hi 组态错误）
*)
FUNCTION_BLOCK FB_LIMIT
  VAR_INPUT
    u  : REAL;
    lo : REAL := 0.0;
    hi : REAL := 100.0;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF lo > hi THEN
    (* 组态错误：取中值输出，双越限位同亮作错误信号 *)
    out := (lo + hi) / 2.0;
    q_hi := TRUE;
    q_lo := TRUE;
    RETURN;
  END_IF;

  IF u > hi THEN
    out := hi;
    q_hi := TRUE;
    q_lo := FALSE;
  ELSIF u < lo THEN
    out := lo;
    q_hi := FALSE;
    q_lo := TRUE;
  ELSE
    out := u;
    q_hi := FALSE;
    q_lo := FALSE;
  END_IF;
END_FUNCTION_BLOCK
