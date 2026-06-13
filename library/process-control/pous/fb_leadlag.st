(*
  FB_LEADLAG — lead-lag compensator (feedforward-path dynamic shaping).

  ~ Standard lead-lag compensation, the customary block for dynamic shaping of a feedforward path.

  Purpose: dynamic matching for feedforward compensation: pass the feedforward
  signal through (1 + s*lead_s)/(1 + s*lag_s) to compensate the time difference
  between the disturbance path and the control path. lead_s > lag_s leads (speeds
  up the response), lead_s < lag_s lags (buffers). The output typically feeds the
  ff input of FB_PID.

  Discrete approximation (standard decomposition: internal PT1 + pass-through mix):
    filt := filt + dt_s / (lag_s + dt_s) * (u - filt)   -- internal first-order lag
    out  := u * (lead_s / lag_s) + filt * (1 - lead_s / lag_s)
  Compared with the continuous transfer function: steady-state gain 1, high-frequency
  gain lead_s/lag_s (first-order approximation; approaches the continuous solution
  when dt_s << lag_s).
  Notes:
    lag_s <= 0 -> pass-through (out := u, filt follows);
    lead_s = 0 -> degenerates to a pure PT1; lead_s = lag_s -> pass-through;
    when lead_s/lag_s is much greater than 1 the high-frequency gain is large, so
    watch for input-noise amplification;
    reset or first scan -> filt := u, out := u (bumpless startup); dt_s <= 0 -> hold.

  Inputs:
    u      REAL  input
    lead_s REAL  lead time constant s
    lag_s  REAL  lag time constant s (0 = pass-through)
    dt_s   REAL  sample period s
    reset  BOOL  reset/align (level-active)
  Outputs:
    out    REAL  compensated output
*)
FUNCTION_BLOCK FB_LEADLAG
  VAR_INPUT
    u      : REAL;
    lead_s : REAL := 0.0;
    lag_s  : REAL := 1.0;
    dt_s   : REAL := 0.1;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    filt      : REAL; (* internal PT1 state *)
    gain_hf   : REAL; (* lead_s / lag_s *)
    init_done : BOOL;
  END_VAR

  IF reset OR NOT init_done THEN
    filt := u;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF lag_s <= 0.0 THEN
    filt := u;
    out := u; (* pass-through *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* hold *)
  END_IF;

  filt := filt + dt_s / (lag_s + dt_s) * (u - filt);
  gain_hf := lead_s / lag_s;
  out := u * gain_hf + filt * (1.0 - gain_hf);
END_FUNCTION_BLOCK
