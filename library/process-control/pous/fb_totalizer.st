(*
  FB_TOTALIZER — flow totalizer: m3/h instantaneous flow -> m3 accumulated quantity.

  Purpose: feed-water / feed-flow totalizing, shift/batch metering.
  The caller may place the instance under VAR RETAIN so that total is retained with the
  instance across power loss (IA2 snapshots RETAIN variables to state/retain.json).

  Formula: total := total + flow * dt_s / 3600.0 (reset takes priority, no accumulation during a reset scan).
  Note: a negative flow accumulates as usual (bidirectional metering allowed); dt_s <= 0 does not accumulate.

  Inputs:
    flow  REAL  instantaneous flow m3/h
    dt_s  REAL  sample period s
    reset BOOL  clear the total (level-active, takes priority over accumulation)
  Outputs:
    total REAL  accumulated quantity m3
*)
FUNCTION_BLOCK FB_TOTALIZER
  VAR_INPUT
    flow  : REAL;
    dt_s  : REAL := 0.1;
    reset : BOOL;
  END_VAR
  VAR_OUTPUT
    total : REAL;
  END_VAR

  IF reset THEN
    total := 0.0;
  ELSIF dt_s > 0.0 THEN
    total := total + flow * dt_s / 3600.0;
  END_IF;
END_FUNCTION_BLOCK
