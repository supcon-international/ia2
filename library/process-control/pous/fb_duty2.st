(*
  FB_DUTY2 — dual-pump duty/standby rotation control.

  ~ Dual-pump duty/standby rotation: select the duty pump by start sequence or run-hour
  balancing, with immediate takeover on fault.

  Purpose: two pumps in a duty/standby arrangement: select the duty pump by rotating
  each start or by balancing accumulated run hours; on a duty-pump fault the standby
  takes over immediately; on a dual fault both stop.
  Behavior (scan-deterministic: no internal timing, only edge memory):
    A demand rising edge selects this round's duty pump, by priority:
      1) exactly one is already running (run_fb TRUE on one side) -> adopt it as duty
         directly (bumpless takeover, avoiding stopping it to start the standby);
      2) alt_mode = 1 -> the lower accumulated hours is duty (hours_a/b fed from two FB_RUNTIME);
      3) alt_mode = 0 (default) -> toggle from the current duty position (including the
         state after a fault takeover, giving natural wear balancing).
    While demand persists: a duty-pump fault with the other healthy -> duty switches
    immediately (takeover); a commanded pump that faults gets no command; a dual fault
    -> both cmd FALSE (the duty selection is held).
    demand withdrawn -> both cmd FALSE.
    Wiring suggestion: feed fault_x from FB_MOTOR's fault or the electrical protection
    relay; feed run_fb_x from the run feedback (debounce a chattering contact with
    FB_DEBOUNCE); leave feedback-timeout fault decision to FB_MOTOR, this block does not
    duplicate the timing.

  Inputs:
    demand   BOOL  system needs a pump (level)
    alt_mode INT   0 = rotate each start / 1 = run-hour balancing
    run_fb_a BOOL  pump A run feedback
    run_fb_b BOOL  pump B run feedback
    fault_a  BOOL  pump A fault
    fault_b  BOOL  pump B fault
    hours_a  REAL  pump A accumulated hours (used by alt_mode=1, fed from FB_RUNTIME.hours)
    hours_b  REAL  pump B accumulated hours (used by alt_mode=1)
    dt_s     REAL  sample period s (this block does not currently time; kept for a uniform library interface)
  Outputs:
    cmd_a     BOOL  pump A start command
    cmd_b     BOOL  pump B start command
    duty_is_a BOOL  the current duty pump is A
*)
FUNCTION_BLOCK FB_DUTY2
  VAR_INPUT
    demand   : BOOL;
    alt_mode : INT := 0;
    run_fb_a : BOOL := FALSE;
    run_fb_b : BOOL := FALSE;
    fault_a  : BOOL := FALSE;
    fault_b  : BOOL := FALSE;
    hours_a  : REAL := 0.0;
    hours_b  : REAL := 0.0;
    dt_s     : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    cmd_a     : BOOL;
    cmd_b     : BOOL;
    duty_is_a : BOOL;
  END_VAR
  VAR
    prev_demand : BOOL; (* rising-edge memory *)
    rising      : BOOL;
  END_VAR

  rising := demand AND NOT prev_demand;
  prev_demand := demand;

  IF rising THEN
    IF run_fb_a AND NOT run_fb_b THEN
      duty_is_a := TRUE;  (* the already-running pump becomes duty directly, bumpless takeover *)
    ELSIF run_fb_b AND NOT run_fb_a THEN
      duty_is_a := FALSE;
    ELSIF alt_mode = 1 THEN
      duty_is_a := hours_a <= hours_b; (* hour balancing: the lower is duty *)
    ELSE
      duty_is_a := NOT duty_is_a; (* rotate each start (initial FALSE -> first start selects A) *)
    END_IF;
  END_IF;

  IF demand THEN
    (* Fault takeover: duty faults and the other is healthy -> duty switches immediately *)
    IF duty_is_a AND fault_a AND NOT fault_b THEN
      duty_is_a := FALSE;
    ELSIF NOT duty_is_a AND fault_b AND NOT fault_a THEN
      duty_is_a := TRUE;
    END_IF;
    cmd_a := duty_is_a AND NOT fault_a;
    cmd_b := NOT duty_is_a AND NOT fault_b;
  ELSE
    cmd_a := FALSE;
    cmd_b := FALSE;
  END_IF;
END_FUNCTION_BLOCK
