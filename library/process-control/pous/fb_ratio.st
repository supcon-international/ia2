(*
  FB_RATIO — ratio station (ratio setpoint generator).

  ~ Ratio station: the customary block for proportioning/blending loops, output feeds the downstream PID's sp.

  Purpose: ratio control: the follower flow setpoint = wild (leading) flow * ratio +
  bias, and the sp output feeds the sp input of a downstream FB_PID to form the ratio
  loop.
  Example: a follower stream tracks the leading flow at a 0.85 proportion.

  Formula: sp := clamp(ratio, ratio_min, ratio_max) * wild_flow + bias.
  Note: only ratio is clamped (a safety limit on the operator-adjustable ratio), sp is
  not clamped -- sp's dimension follows wild_flow, and the downstream PID has its own
  output clamp.

  Inputs:
    wild_flow REAL  leading flow (the "wild" flow not controlled by this loop)
    ratio     REAL  ratio
    ratio_min REAL  ratio low limit
    ratio_max REAL  ratio high limit
    bias      REAL  bias (engineering units)
  Outputs:
    sp        REAL  follower-loop setpoint
*)
FUNCTION_BLOCK FB_RATIO
  VAR_INPUT
    wild_flow : REAL;
    ratio     : REAL := 1.0;
    ratio_min : REAL := 0.0;
    ratio_max : REAL := 10.0;
    bias      : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    sp : REAL;
  END_VAR
  VAR
    ratio_used : REAL;
  END_VAR

  ratio_used := ratio;
  IF ratio_used < ratio_min THEN
    ratio_used := ratio_min;
  ELSIF ratio_used > ratio_max THEN
    ratio_used := ratio_max;
  END_IF;

  sp := ratio_used * wild_flow + bias;
END_FUNCTION_BLOCK
