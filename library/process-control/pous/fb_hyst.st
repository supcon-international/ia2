(*
  FB_HYST — bidirectional hysteresis (deadband) switch.

  Purpose: on/off control of an analog value with hysteresis, avoiding chatter at the threshold.
  Example: current control of a feeder -- current above on_sp stops feeding, below
  off_sp resumes (low-side-on direction); or temperature above on_sp starts a fan, below
  off_sp stops it (high-side-on direction).

  Behavior (direction decided automatically from the setpoints):
    on_sp > off_sp (high-side-on): pv >= on_sp sets; pv <= off_sp resets; holds in between.
    on_sp < off_sp (low-side-on):  pv <= on_sp sets; pv >= off_sp resets; holds in between.
    on_sp = off_sp (degenerate):   out := pv >= on_sp (plain comparator, no hysteresis).

  Inputs:
    pv     REAL  process value
    on_sp  REAL  switch-on setpoint
    off_sp REAL  switch-off setpoint
  Outputs:
    out    BOOL  switch output
*)
FUNCTION_BLOCK FB_HYST
  VAR_INPUT
    pv     : REAL;
    on_sp  : REAL;
    off_sp : REAL;
  END_VAR
  VAR_OUTPUT
    out : BOOL;
  END_VAR

  IF on_sp > off_sp THEN
    (* High-side-on *)
    IF pv >= on_sp THEN
      out := TRUE;
    ELSIF pv <= off_sp THEN
      out := FALSE;
    END_IF;
  ELSIF on_sp < off_sp THEN
    (* Low-side-on *)
    IF pv <= on_sp THEN
      out := TRUE;
    ELSIF pv >= off_sp THEN
      out := FALSE;
    END_IF;
  ELSE
    (* Setpoints equal: degenerate to a plain comparator *)
    out := pv >= on_sp;
  END_IF;
END_FUNCTION_BLOCK
