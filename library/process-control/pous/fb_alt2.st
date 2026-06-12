(*
  FB_ALT2 — 双阀（双设备）定时轮换器，带双外部联锁。

  用途：两路分料阀/双泵按固定周期轮换运行，磨损均衡。
  例：除铁器 + 包装秤 双联锁分料：permissive1（除铁器运行）与
  permissive2（包装秤就绪）任一失去 → 两阀同时关；联锁恢复后按
  轮换计时继续（轮换选择保持，计时清零重计）。

  行为：
    ok := enable AND permissive1 AND permissive2。
    NOT ok：q1 := FALSE，q2 := FALSE，轮换计时清零（选择位保持，
    恢复后从上次选中的一路继续）。
    ok：计时 dt_s 手工累加，到 swap_period_s 切换选择并清零计时；
    q1/q2 互斥输出当前选中一路。swap_period_s <= 0 时不轮换（保持当前路）。

  输入：
    enable        BOOL  使能
    swap_period_s REAL  轮换周期 s
    dt_s          REAL  采样周期 s
    permissive1   BOOL  外部联锁 1（如除铁器运行）
    permissive2   BOOL  外部联锁 2（如包装秤就绪）
  输出：
    q1 BOOL  一号阀输出
    q2 BOOL  二号阀输出（与 q1 互斥；联锁失去时两者均 FALSE）
*)
FUNCTION_BLOCK FB_ALT2
  VAR_INPUT
    enable        : BOOL;
    swap_period_s : REAL := 300.0;
    dt_s          : REAL := 0.1;
    permissive1   : BOOL := TRUE;
    permissive2   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    q1 : BOOL;
    q2 : BOOL;
  END_VAR
  VAR
    ok    : BOOL;
    sel2  : BOOL; (* FALSE 选一号 / TRUE 选二号，停用期间保持 *)
    run_t : REAL; (* 本路已运行时间 s，手工计时 *)
  END_VAR

  ok := enable AND permissive1 AND permissive2;

  IF NOT ok THEN
    (* 任一联锁失去：两阀同关，计时清零，选择保持 *)
    q1    := FALSE;
    q2    := FALSE;
    run_t := 0.0;
    RETURN;
  END_IF;

  IF dt_s > 0.0 THEN
    run_t := run_t + dt_s;
  END_IF;
  IF swap_period_s > 0.0 AND run_t >= swap_period_s THEN
    run_t := 0.0;
    sel2  := NOT sel2;
  END_IF;

  q1 := NOT sel2;
  q2 := sel2;
END_FUNCTION_BLOCK
