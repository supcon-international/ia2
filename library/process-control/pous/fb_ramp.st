(*
  FB_RAMP — setpoint ramp generator (rate-limited approach to a fixed target).

  ~ Standard setpoint ramp generator.

  Purpose: gentle setpoint changes (avoiding step shocks to actuators/process),
  segmented heat-up/pressure-up curves.
  Difference from FB_RATE_LIMIT: this block moves toward a fixed target and reports
  ramping; RATE_LIMIT continuously follows a changing input and only clamps the
  per-second change.

  Behavior:
    out approaches target at rate_up (upward) / rate_dn (downward, supplied positive)
    units per second, and on arrival ramping := FALSE. A rate of 0 means a direct
    step in that direction.
    track level-active: out follows track_value unconditionally and ramping := FALSE;
    after track is released, the ramp continues from the current value toward target
    (the bumpless-startup idiom: track-align to the current process value first, then
    release to start ramping).
    dt_s <= 0 -> hold. Note out initializes to 0: on the first startup scan, prefer
    aligning with track.

  Inputs:
    target      REAL  target value
    rate_up     REAL  upward rate units/s (0 = step)
    rate_dn     REAL  downward rate units/s (supplied positive; 0 = step)
    dt_s        REAL  sample period s
    track       BOOL  tracking enable (level-active)
    track_value REAL  tracking value
  Outputs:
    out     REAL  ramp output
    ramping BOOL  ramp in progress (target not yet reached)
*)
FUNCTION_BLOCK FB_RAMP
  VAR_INPUT
    target      : REAL;
    rate_up     : REAL := 0.0;
    rate_dn     : REAL := 0.0;
    dt_s        : REAL := 0.1;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out     : REAL;
    ramping : BOOL;
  END_VAR
  VAR
    delta : REAL;
    step  : REAL;
  END_VAR

  IF track THEN
    out := track_value; (* bumpless: after release the ramp continues from here *)
    ramping := FALSE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* hold *)
  END_IF;

  delta := target - out;
  IF delta > 0.0 THEN
    step := rate_up * dt_s;
    IF rate_up <= 0.0 OR step >= delta THEN
      out := target; (* rate 0 step / reachable this scan *)
      ramping := FALSE;
    ELSE
      out := out + step;
      ramping := TRUE;
    END_IF;
  ELSIF delta < 0.0 THEN
    step := rate_dn * dt_s;
    IF rate_dn <= 0.0 OR step >= 0.0 - delta THEN
      out := target;
      ramping := FALSE;
    ELSE
      out := out - step;
      ramping := TRUE;
    END_IF;
  ELSE
    ramping := FALSE;
  END_IF;
END_FUNCTION_BLOCK
