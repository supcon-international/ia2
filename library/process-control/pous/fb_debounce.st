(*
  FB_DEBOUNCE — discrete-input debounce (independent bidirectional confirmation delay).

  ~ Discrete-input debounce (on/off dual delay), standard for DI processing.

  Purpose: confirming chattering signals such as float level switches, limit switches,
  relay/contactor auxiliary contacts; can also debounce the run_fb/fault inputs of
  FB_DUTY2.
  Behavior:
    u continuously TRUE for t_on_s -> q := TRUE; u continuously FALSE for t_off_s ->
    q := FALSE; any reverse transition in between clears that direction's timer (restart).
    t = 0 means that direction takes effect immediately. The actual confirmation time
    rounds up to a dt_s scan.
    (Manual dt_s accumulation, equivalent to a TON on-confirm + off-confirm, offline-testable.)

  Inputs:
    u       BOOL  raw discrete input
    t_on_s  REAL  set confirmation time s (0 = immediate)
    t_off_s REAL  reset confirmation time s (0 = immediate)
    dt_s    REAL  sample period s
  Outputs:
    q       BOOL  debounced discrete output
*)
FUNCTION_BLOCK FB_DEBOUNCE
  VAR_INPUT
    u       : BOOL;
    t_on_s  : REAL := 0.1;
    t_off_s : REAL := 0.1;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    on_t  : REAL; (* set-direction accumulator *)
    off_t : REAL; (* reset-direction accumulator *)
  END_VAR

  IF u THEN
    off_t := 0.0;
    IF NOT q THEN
      on_t := on_t + dt_s;
      IF on_t >= t_on_s THEN
        q := TRUE;
      END_IF;
    END_IF;
  ELSE
    on_t := 0.0;
    IF q THEN
      off_t := off_t + dt_s;
      IF off_t >= t_off_s THEN
        q := FALSE;
      END_IF;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
