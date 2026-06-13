(*
  FB_RATE_LIMIT — rate limiter (slope clamp that follows the input).

  ~ Standard rate limiter (velocity/slew limiter).

  Purpose: limit the per-second change of a continuously varying signal: valve-command
  anti-chatter, gentle drive-reference changes, feedforward smoothing.
  Difference from FB_RAMP: this block follows a continuously changing input u (no
  target concept) and only clamps |delta|/s; RAMP moves toward a fixed target.

  Behavior:
    delta := u - out is clamped to +/-rate_s*dt_s and then accumulated: out := out + delta_clamped.
    rate_s <= 0 -> pass-through; dt_s <= 0 -> hold;
    reset or first scan -> out := u (alignment, avoiding a full-speed climb from 0 at startup).

  Inputs:
    u      REAL  input
    rate_s REAL  maximum rate of change |units|/s (0 = pass-through)
    dt_s   REAL  sample period s
    reset  BOOL  reset/align (level-active: out follows u)
  Outputs:
    out    REAL  rate-limited output
*)
FUNCTION_BLOCK FB_RATE_LIMIT
  VAR_INPUT
    u      : REAL;
    rate_s : REAL := 0.0;
    dt_s   : REAL := 0.1;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    delta     : REAL;
    max_step  : REAL;
    init_done : BOOL;
  END_VAR

  IF reset OR NOT init_done THEN
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF rate_s <= 0.0 THEN
    out := u; (* pass-through *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* hold *)
  END_IF;

  delta := u - out;
  max_step := rate_s * dt_s;
  IF delta > max_step THEN
    delta := max_step;
  ELSIF delta < 0.0 - max_step THEN
    delta := 0.0 - max_step;
  END_IF;
  out := out + delta;
END_FUNCTION_BLOCK
