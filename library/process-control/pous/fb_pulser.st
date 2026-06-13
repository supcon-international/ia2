(*
  FB_PULSER — periodic pulse generator.

  Purpose: periodic air-cannon / air-hammer silo purging, periodic lubrication, heartbeat output.
  Example: once enabled, output a pulse_len_s-wide pulse every period_s seconds (the pulse comes at the period start).

  Behavior:
    while enabled, the phase phase_s accumulates by dt_s manually; q := TRUE while
    phase_s < pulse_len_s, wrapping at period_s (subtraction wrap, so the long-term
    cadence does not drift).
    NOT enable or invalid parameters (period_s/dt_s <= 0) -> q := FALSE and phase cleared
    (re-enabling starts from the pulse).
    pulse_len_s >= period_s -> q is always TRUE (continuous output by definition).

  Inputs:
    enable      BOOL  enable
    period_s    REAL  pulse period s
    pulse_len_s REAL  pulse width s
    dt_s        REAL  sample period s
  Outputs:
    q BOOL  pulse output
*)
FUNCTION_BLOCK FB_PULSER
  VAR_INPUT
    enable      : BOOL;
    period_s    : REAL := 60.0;
    pulse_len_s : REAL := 1.0;
    dt_s        : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* phase within the period 0..period_s, manual timing in place of TP/TON *)
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  (* Emit the pulse by the current phase first (TRUE immediately on the first enabled period), then advance the phase *)
  q := phase_s < pulse_len_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* subtraction wrap, keeps the long-term cadence *)
  END_IF;
END_FUNCTION_BLOCK
