(*
  FB_DERIV — filtered derivative (du/dt, filter first then difference).

  ~ Standard filtered-derivative block, a staple of every major platform's standard library.

  Purpose: quantifying trend (degC/s, %/s for display/feedforward/custom logic),
  derivative shaping in a feedforward path. Whenever you want "alarm on limit", use
  FB_ALARM_ROC directly (the same filter-difference core + delay/latch).
  Behavior:
    u first passes through an internal PT1 filter (time constant t_filt_s; <= 0 takes
    3*dt_s automatically, settling in about 4 scans), then a backward difference of
    the filtered value:
      u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f)
      out := (u_f - u_f_prev) / dt_s
    A single-scan difference is extremely sensitive to quantization/noise and must be
    filtered, or the output is all spikes -- for stronger noise rejection, increase
    t_filt_s or put FB_LAG ahead of u.
    reset / first scan: the filter state aligns to u, out := 0 (no spurious startup derivative).
    dt_s <= 0 -> hold (no action).

  Inputs:
    u        REAL  input
    t_filt_s REAL  derivative pre-filter time constant s (<= 0 = auto 3*dt_s)
    dt_s     REAL  sample period s
    reset    BOOL  reset/align (level-active: filter state follows u, out cleared)
  Outputs:
    out REAL  filtered rate of change units/s (signed)
*)
FUNCTION_BLOCK FB_DERIV
  VAR_INPUT
    u        : REAL;
    t_filt_s : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    u_f       : REAL; (* PT1-filtered input *)
    u_f_prev  : REAL; (* previous-scan filtered value, for the backward difference *)
    t_f       : REAL; (* filter time constant actually used *)
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  IF reset OR NOT init_done THEN
    u_f := u; (* align the filter state, no spurious startup derivative *)
    u_f_prev := u;
    out := 0.0;
    init_done := TRUE;
    RETURN;
  END_IF;

  t_f := t_filt_s;
  IF t_f <= 0.0 THEN
    t_f := 3.0 * dt_s; (* auto: the minimum noise rejection that settles in about 4 scans *)
  END_IF;

  u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f);
  out := (u_f - u_f_prev) / dt_s;
  u_f_prev := u_f;
END_FUNCTION_BLOCK
