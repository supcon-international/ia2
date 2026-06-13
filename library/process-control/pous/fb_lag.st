(*
  FB_LAG — first-order lag filter (PT1 / first-order lag).

  ~ Standard first-order lag filter (PT1 / LAG), a staple of every major platform's standard library.

  Purpose: analog noise rejection, measurement smoothing, pre-filtering ahead of
  derivative / rate-of-change computation.
  Algorithm (backward-Euler discretization, unconditionally stable):
    out := out + dt_s / (t_s + dt_s) * (u - out)
  Notes:
    t_s <= 0 -> pass-through (out := u); dt_s <= 0 -> hold (no action);
    reset level-active -> out := u (filter alignment, bumpless startup);
    the first scan auto-aligns (out := u) to avoid a startup transient
    crawling up from 0.

  Inputs:
    u     REAL  input
    t_s   REAL  filter time constant s (0 = pass-through)
    dt_s  REAL  sample period s
    reset BOOL  reset/align (level-active: out follows u)
  Outputs:
    out   REAL  filtered output
*)
FUNCTION_BLOCK FB_LAG
  VAR_INPUT
    u     : REAL;
    t_s   : REAL := 1.0;
    dt_s  : REAL := 0.1;
    reset : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF reset OR NOT init_done THEN
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_s <= 0.0 THEN
    out := u; (* pass-through *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  out := out + dt_s / (t_s + dt_s) * (u - out);
END_FUNCTION_BLOCK
