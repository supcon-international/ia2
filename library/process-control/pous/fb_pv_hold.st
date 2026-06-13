(*
  FB_PV_HOLD — bad-PV hold / substitute.

  ~ Standard bad-value handling block (last-good hold / substitute value, including
  bad-value timeout degradation), the standard line of defense before a measurement
  enters a control loop; every major DCS/PLC input-processing chain has an equivalent
  mechanism.

  Purpose: on transmitter open-circuit/over-range (e.g. FB_SCALE.fault, or the inverse
  of FB_MID3.q_fail used as good), give the downstream loop a controlled PV: hold the
  last good value first to ride out chatter, then on timeout hold/substitute/follow as
  configured, while emitting q_bad/q_timeout for alarming and loop-to-manual logic.

  Behavior:
    good = TRUE: out := u, remember the last good value, clear q_bad/q_timeout, clear
    the bad-value timer.
    good = FALSE: q_bad := TRUE, accumulate the bad-value timer (manual dt_s):
      before timeout_s: out := last good value (short chatter rides through on hold);
      at timeout_s (timeout_s = 0 immediately): q_timeout := TRUE, then by mode:
        mode = 0: keep holding the last good value;
        mode = 1: out := sub_value (substitute value, e.g. a safe-side setting);
        mode = 2: out := u (follow the raw value -- just raise the alarm flag without
                  intervening, downstream handles it).
    When bad on the very first scan, the last good value takes the current u (no better history).
    dt_s <= 0 -> the bad-value timer does not advance (when timeout_s > 0, the hold segment is maintained).

  Inputs:
    u         REAL  input (raw PV)
    good      BOOL  input valid (FALSE = bad value; feed the inverse of a quality bit / open-circuit decision)
    mode      REAL  0 = hold last good value / 1 = substitute value / 2 = follow raw value
    sub_value REAL  substitute value (used by mode = 1)
    timeout_s REAL  bad-value persistence timeout s (0 = act by mode immediately; > 0 hold first)
    dt_s      REAL  sample period s
  Outputs:
    out       REAL  processed PV
    q_bad     BOOL  in bad value (level)
    q_timeout BOOL  bad value has timed out (mode in effect)
*)
FUNCTION_BLOCK FB_PV_HOLD
  VAR_INPUT
    u         : REAL;
    good      : BOOL := TRUE;
    mode      : REAL := 0.0;
    sub_value : REAL := 0.0;
    timeout_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out       : REAL;
    q_bad     : BOOL;
    q_timeout : BOOL;
  END_VAR
  VAR
    last_good : REAL; (* last good value *)
    bad_t     : REAL; (* bad-value accumulated persistence time s *)
    init_done : BOOL; (* first-scan seeding flag *)
  END_VAR

  IF NOT init_done THEN
    last_good := u; (* even when bad on the first scan, take the current u as the held value *)
    init_done := TRUE;
  END_IF;

  IF good THEN
    last_good := u;
    out := u;
    q_bad := FALSE;
    q_timeout := FALSE;
    bad_t := 0.0;
    RETURN;
  END_IF;

  q_bad := TRUE;

  (* Before timeout: hold the last good value (the segment that rides out short chatter) *)
  IF timeout_s > 0.0 THEN
    IF dt_s > 0.0 THEN
      bad_t := bad_t + dt_s;
    END_IF;
    IF bad_t < timeout_s THEN
      out := last_good;
      q_timeout := FALSE;
      RETURN;
    END_IF;
  END_IF;

  (* On timeout (or immediately when timeout_s = 0): act by mode *)
  q_timeout := TRUE;
  IF mode >= 1.5 THEN
    out := u;         (* 2 = follow raw value *)
  ELSIF mode >= 0.5 THEN
    out := sub_value; (* 1 = substitute value *)
  ELSE
    out := last_good; (* 0 = hold last good value *)
  END_IF;
END_FUNCTION_BLOCK
