(*
  FB_SQRT_FLOW — differential-pressure flow square-root extraction.

  ~ DP-flow square-root extraction + low-flow cutoff, a customary block for DCS analog-signal processing.

  Purpose: orifice/venturi/nozzle DP flow measurement: flow is proportional to the
  square root of differential pressure.
  Formula: flow := flow_max * SQRT(dp_pct / 100.0); dp_pct <= cutoff_pct -> flow := 0.
  Notes:
    cutoff_pct is the low-flow cutoff: in the low-DP region the square-root gain tends
    to infinity and noise is amplified severely, so the convention is to cut to zero.
    Default 6.25% (DP) <-> 25% (flow) cutoff point.
    dp_pct <= 0 is treated as 0; dp_pct > 100 is not clipped (the square-root extension
    beyond range is allowed; clamp on the caller side if a hard limit is needed).
    SQRT() is a built-in of this dialect (verified usable through cs project check including codegen).

  Inputs:
    dp_pct     REAL  differential pressure % (0..100, typically fed from FB_SCALE output)
    flow_max   REAL  flow engineering value at 100% DP
    cutoff_pct REAL  low-flow cutoff threshold % (as a DP percentage)
  Outputs:
    flow       REAL  flow engineering value
*)
FUNCTION_BLOCK FB_SQRT_FLOW
  VAR_INPUT
    dp_pct     : REAL;
    flow_max   : REAL := 100.0;
    cutoff_pct : REAL := 6.25;
  END_VAR
  VAR_OUTPUT
    flow : REAL;
  END_VAR

  IF dp_pct <= cutoff_pct OR dp_pct <= 0.0 THEN
    flow := 0.0; (* low-flow cutoff *)
    RETURN;
  END_IF;

  flow := flow_max * SQRT(dp_pct / 100.0);
END_FUNCTION_BLOCK
