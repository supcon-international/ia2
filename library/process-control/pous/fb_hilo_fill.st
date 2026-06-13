(*
  FB_HILO_FILL — high-close, low-open tank/silo replenishment control.

  Purpose: below the low setpoint open the supply valve, above the high setpoint close
  it, hold the current state in between (equivalent to a low-side-on hysteresis switch,
  given a dedicated name for readability in configuration).
  Example: circulating-tank make-up: pv <= l_sp opens the make-up valve, pv >= h_sp closes it.

  Behavior:
    pv <= l_sp -> valve_open := TRUE (low-open)
    pv >= h_sp -> valve_open := FALSE (high-close)
    l_sp < pv < h_sp -> hold.
  Assumes l_sp < h_sp; if reversed, the high test takes priority (opens then is
  immediately closed, output FALSE).

  Inputs:
    pv   REAL  level process value
    h_sp REAL  high setpoint (close)
    l_sp REAL  low setpoint (open)
  Outputs:
    valve_open BOOL  supply-valve output (TRUE open)
*)
FUNCTION_BLOCK FB_HILO_FILL
  VAR_INPUT
    pv   : REAL;
    h_sp : REAL := 85.0;
    l_sp : REAL := 15.0;
  END_VAR
  VAR_OUTPUT
    valve_open : BOOL;
  END_VAR

  IF pv >= h_sp THEN
    valve_open := FALSE; (* high-close, priority *)
  ELSIF pv <= l_sp THEN
    valve_open := TRUE;  (* low-open *)
  END_IF;
  (* Hold in between *)
END_FUNCTION_BLOCK
