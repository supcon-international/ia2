(*
  FB_DEADTIME8 — dead time / transport delay, 8 scalar taps + linear interpolation.

  ~ Standard dead-time block (dead time / PTt / transport delay), the customary block
  for plant simulation and feedforward dynamic compensation (Smith-predictor-type structures).

  Purpose: plant simulation (in series with FB_LAG it forms a first-order-lag-plus-dead-time
  FOPDT plant), transport-delay matching in a feedforward path, time alignment of
  belt/pipe-conveying measurements.
  Behavior:
    Tap chain z1..z8 (z1 newest, z8 oldest), sampling interval = t_delay_s / 8:
    the phase phase_s accumulates by dt_s manually, and each time a sampling interval
    elapses the whole chain shifts once (z8<-z7<-...<-z1<-u); at most 8 shifts per scan
    (after 8 the whole chain has been filled by u, so the WHILE is bounded and
    scan-deterministic).
    The output interpolates linearly between the two oldest taps by the phase fraction:
      out := z8 + (z7 - z8) * phase_s / period_s
    -- a step input is traversed as a ramp over one sampling interval, so the output is
       continuous with no steps.
    Resolution = t_delay_s / 8: 8 taps is the compromise in an array-free dialect; the
    staircase/interpolation error of the delay time is about 1/8 of the delay --
    adequate for loop tuning; for finer resolution, put two of these blocks in series
    (each carrying half the delay).
    t_delay_s <= 0 -> pass-through (out := u), and the taps fill with u -- when the
    delay is put into service online it starts smoothly from the current value (bumpless).
    reset / first scan: all taps filled with u, out := u, phase cleared.
    dt_s <= 0 -> hold (no action).

  Inputs:
    u         REAL  input
    t_delay_s REAL  dead time s (<= 0 pass-through; changeable online, advances at the new interval)
    dt_s      REAL  sample period s
    reset     BOOL  reset (level-active: taps filled with u, output follows u)
  Outputs:
    out REAL  delayed output (linearly interpolated, continuous)
*)
FUNCTION_BLOCK FB_DEADTIME8
  VAR_INPUT
    u         : REAL;
    t_delay_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
    reset     : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    z1        : REAL; (* tap chain: z1 newest ... z8 oldest (scalar form for the array-free dialect) *)
    z2        : REAL;
    z3        : REAL;
    z4        : REAL;
    z5        : REAL;
    z6        : REAL;
    z7        : REAL;
    z8        : REAL;
    phase_s   : REAL; (* phase within the current sampling interval, manual timing *)
    period_s  : REAL; (* sampling interval = t_delay_s / 8 *)
    shifts    : REAL; (* shift count this scan (capped at 8: chain already saturated) *)
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF reset OR NOT init_done THEN
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_delay_s <= 0.0 THEN
    (* Pass-through; taps fill with u, so when the delay is put into service online it starts bumplessly from the current value *)
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  period_s := t_delay_s / 8.0;
  phase_s := phase_s + dt_s;

  (* Shift the whole chain once per elapsed sampling interval; capped at 8/scan (when
     dt_s spans several intervals, catch up; after 8 the chain is full of u and further
     shifts are meaningless) -- the WHILE is bounded *)
  shifts := 0.0;
  WHILE (phase_s >= period_s) AND (shifts < 8.0) DO
    z8 := z7;
    z7 := z6;
    z6 := z5;
    z5 := z4;
    z4 := z3;
    z3 := z2;
    z2 := z1;
    z1 := u;
    phase_s := phase_s - period_s;
    shifts := shifts + 1.0;
  END_WHILE;
  IF phase_s >= period_s THEN
    phase_s := 0.0; (* degenerate case dt_s >= t_delay_s: chain is already u, clear phase to prevent accumulation *)
  END_IF;

  (* Interpolate between the two oldest taps by phase fraction: at the shift instant frac=0 -> out=z8(old z7), continuous *)
  out := z8 + (z7 - z8) * phase_s / period_s;
END_FUNCTION_BLOCK
