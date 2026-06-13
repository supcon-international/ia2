(*
  FB_CHAR — piecewise-linear signal characterization (linear interpolation, up to 8 points).

  ~ Piecewise-linear characterizer, the customary block for valve-characteristic linearization / nonlinear sensors.

  Purpose: valve-characteristic linearization (equal-percentage valve-opening
  compensation), nonlinear sensor calibration, level-to-volume conversion, and any
  monotonic piecewise-linear transform.

  Behavior:
    Take the first n points (x1,y1)..(xn,yn) (n clamped to 2..8); interpolate
    linearly when u falls in segment (xk, xk+1]; u <= x1 -> out := y1; u >= xn ->
    out := yn (clamped at both ends).
    Convention: x must be ascending (no hard check here; a degenerate non-ascending
    segment yields the right-end y).
    In this dialect an ARRAY in FUNCTION_BLOCK scope cannot be codegen'd (see the
    README dialect gotchas), so scalar point pairs x1..x8 / y1..y8 + n are used.

  Inputs:
    u          REAL  input
    n          INT   number of points in use 2..8 (auto-clamped)
    x1..x8     REAL  breakpoint x coordinates (ascending)
    y1..y8     REAL  breakpoint y coordinates
  Outputs:
    out        REAL  characterized output (clamped at both ends)
*)
FUNCTION_BLOCK FB_CHAR
  VAR_INPUT
    u  : REAL;
    n  : INT := 2;
    x1 : REAL := 0.0;
    y1 : REAL := 0.0;
    x2 : REAL := 100.0;
    y2 : REAL := 100.0;
    x3 : REAL := 0.0;
    y3 : REAL := 0.0;
    x4 : REAL := 0.0;
    y4 : REAL := 0.0;
    x5 : REAL := 0.0;
    y5 : REAL := 0.0;
    x6 : REAL := 0.0;
    y6 : REAL := 0.0;
    x7 : REAL := 0.0;
    y7 : REAL := 0.0;
    x8 : REAL := 0.0;
    y8 : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    nn    : INT;  (* clamped point count *)
    xa    : REAL; (* selected segment left end *)
    ya    : REAL;
    xb    : REAL; (* selected segment right end *)
    yb    : REAL;
    span  : REAL;
    found : BOOL;
  END_VAR

  nn := n;
  IF nn < 2 THEN
    nn := 2;
  ELSIF nn > 8 THEN
    nn := 8;
  END_IF;

  (* Low-end clamp *)
  IF u <= x1 THEN
    out := y1;
    RETURN;
  END_IF;

  (* Segment search: take the segment u falls into, (xk, xk+1]; only the first nn points are considered *)
  found := FALSE;
  IF u <= x2 THEN
    xa := x1; ya := y1; xb := x2; yb := y2; found := TRUE;
  ELSIF nn >= 3 AND u <= x3 THEN
    xa := x2; ya := y2; xb := x3; yb := y3; found := TRUE;
  ELSIF nn >= 4 AND u <= x4 THEN
    xa := x3; ya := y3; xb := x4; yb := y4; found := TRUE;
  ELSIF nn >= 5 AND u <= x5 THEN
    xa := x4; ya := y4; xb := x5; yb := y5; found := TRUE;
  ELSIF nn >= 6 AND u <= x6 THEN
    xa := x5; ya := y5; xb := x6; yb := y6; found := TRUE;
  ELSIF nn >= 7 AND u <= x7 THEN
    xa := x6; ya := y6; xb := x7; yb := y7; found := TRUE;
  ELSIF nn >= 8 AND u <= x8 THEN
    xa := x7; ya := y7; xb := x8; yb := y8; found := TRUE;
  END_IF;

  IF found THEN
    span := xb - xa;
    IF span > 0.0 THEN
      out := ya + (u - xa) * (yb - ya) / span;
    ELSE
      out := yb; (* degenerate segment (x not ascending): take the right end *)
    END_IF;
    RETURN;
  END_IF;

  (* High-end clamp: u greater than the nn-th point *)
  CASE nn OF
    3:
      out := y3;
    4:
      out := y4;
    5:
      out := y5;
    6:
      out := y6;
    7:
      out := y7;
    8:
      out := y8;
  ELSE
    out := y2;
  END_CASE;
END_FUNCTION_BLOCK
