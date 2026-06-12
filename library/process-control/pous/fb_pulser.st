(*
  FB_PULSER — 周期脉冲发生器。

  用途：料仓空气炮/气锤周期吹扫、周期润滑、心跳输出。
  例：使能后每 period_s 秒输出一个 pulse_len_s 秒宽的脉冲（周期起点先出脉冲）。

  行为：
    enable 时相位 phase_s 以 dt_s 手工累加；phase_s < pulse_len_s 期间 q := TRUE，
    到 period_s 回卷（减法回卷，长期节拍不漂移）。
    NOT enable 或参数非法（period_s/dt_s <= 0）时 q := FALSE 且相位清零
    （重新使能从脉冲开始）。
    pulse_len_s >= period_s 时 q 恒为 TRUE（按定义连续输出）。

  输入：
    enable      BOOL  使能
    period_s    REAL  脉冲周期 s
    pulse_len_s REAL  脉冲宽度 s
    dt_s        REAL  采样周期 s
  输出：
    q BOOL  脉冲输出
*)
FUNCTION_BLOCK FB_PULSER
  VAR_INPUT
    enable      : BOOL;
    period_s    : REAL := 60.0;
    pulse_len_s : REAL := 1.0;
    dt_s        : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* 周期内相位 0..period_s，手工计时替代 TP/TON *)
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  (* 先按当前相位出脉冲（使能首周期立即为 TRUE），再推进相位 *)
  q := phase_s < pulse_len_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* 减法回卷，保持长期节拍 *)
  END_IF;
END_FUNCTION_BLOCK
