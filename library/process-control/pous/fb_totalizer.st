(*
  FB_TOTALIZER — 流量累积器：m3/h 瞬时流量 → m3 累积量。

  用途：进水/进料流量累计、班产/批量计量。
  实例可由调用方放在 VAR RETAIN 块下，使 total 随实例一起断电保持
  （IA2 将 RETAIN 变量快照到 state/retain.json）。

  公式：total := total + flow * dt_s / 3600.0（reset 优先，复位周期不累加）。
  说明：flow 为负时照常累加（允许双向计量）；dt_s <= 0 时不累加。

  输入：
    flow  REAL  瞬时流量 m3/h
    dt_s  REAL  采样周期 s
    reset BOOL  累积清零（电平有效，优先于累加）
  输出：
    total REAL  累积量 m3
*)
FUNCTION_BLOCK FB_TOTALIZER
  VAR_INPUT
    flow  : REAL;
    dt_s  : REAL := 0.1;
    reset : BOOL;
  END_VAR
  VAR_OUTPUT
    total : REAL;
  END_VAR

  IF reset THEN
    total := 0.0;
  ELSIF dt_s > 0.0 THEN
    total := total + flow * dt_s / 3600.0;
  END_IF;
END_FUNCTION_BLOCK
