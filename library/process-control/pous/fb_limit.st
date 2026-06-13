(*
  FB_LIMIT — value limiter (high-low clamp + limit indication).

  ~ Standard limiter (limiter / high-low clamp), a staple of every major platform's standard library.

  Purpose: safe clamping of setpoint/output values, cascade outer-loop output limiting,
  operator-input validation, the last line of protection for a bare downstream actuator.
  Behavior (pure combinational logic, no internal state):
    out := u clamped to [lo, hi]; q_hi / q_lo indicate clamping in progress (u over the high/low limit).
    Configuration error lo > hi -> out := (lo + hi) / 2, with q_hi and q_lo both TRUE
    -- both limit flags lit at once cannot occur physically and serves purely as a
    configuration-error signal, easy for a supervisory alarm to catch.

  Inputs:
    u  REAL  input
    lo REAL  low limit
    hi REAL  high limit (assumes lo <= hi; reversed triggers the configuration-error signal)
  Outputs:
    out  REAL  clamped output
    q_hi BOOL  over high limit (clamping at hi)
    q_lo BOOL  over low limit (clamping at lo; TRUE together with q_hi = lo > hi configuration error)
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
    (* Configuration error: output the midpoint, both limit flags lit as the error signal *)
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
