(*
  FB_SPLIT_RANGE — split-range output: one controller output distributed to two valves at the split point.

  ~ Split range: one control output drives two valves by segment (heating/cooling, large/small valves).

  Purpose: one PID drives two actuators: large/small valves extending the range
  (sequential split range, reverse_b := FALSE), or a heating/cooling or
  feed/reflux complementary split range (reverse_b := TRUE, tuned per the actuator's
  air-to-open/air-to-close action and the PID action direction).

  Behavior (u is clamped 0..100 first, both outputs also clamped 0..100):
    out_a: linear 0->100 for u in [0, split], holds 100 once u >= split.
    out_b: linear 0->100 for u in [split, 100], 0 when u <= split;
           when reverse_b = TRUE, segment B reverses to 100->0 (with the end value held
           outside the segment, i.e. out_b = 100 when u <= split, output continuous with
           no jump).
    Curve sketch (split = 50):
      reverse_b = FALSE (large/small valve sequential split range):
        u:     0 .... 50 .... 100
        out_a: 0 -> 100 | holds 100
        out_b: 0   0    | 0 -> 100
      reverse_b = TRUE (complementary, B fully open first then closing):
        u:     0 .... 50 .... 100
        out_a: 0 -> 100 | holds 100
        out_b: 100 100  | 100 -> 0
    Degenerate: split <= 0 -> segment A has no range (u > 0 means 100); split >= 100 ->
    segment B has no range (acts only once u >= 100). split should be set within (0, 100).

  Inputs:
    u         REAL  controller output 0..100
    split     REAL  split point % (segment B start)
    reverse_b BOOL  TRUE = valve B is reverse-acting within its split segment (100->0)
  Outputs:
    out_a     REAL  valve A opening 0..100
    out_b     REAL  valve B opening 0..100
*)
FUNCTION_BLOCK FB_SPLIT_RANGE
  VAR_INPUT
    u         : REAL;
    split     : REAL := 50.0;
    reverse_b : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out_a : REAL;
    out_b : REAL;
  END_VAR
  VAR
    uc     : REAL; (* clamped input *)
    span_b : REAL;
  END_VAR

  (* Input clamp 0..100 *)
  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  (* Valve A: 0->100 for u in [0, split] *)
  IF split > 0.0 THEN
    out_a := uc * 100.0 / split;
  ELSE
    (* Degenerate: A has no split segment *)
    IF uc > 0.0 THEN
      out_a := 100.0;
    ELSE
      out_a := 0.0;
    END_IF;
  END_IF;
  IF out_a < 0.0 THEN
    out_a := 0.0;
  ELSIF out_a > 100.0 THEN
    out_a := 100.0;
  END_IF;

  (* Valve B: 0->100 for u in [split, 100] *)
  span_b := 100.0 - split;
  IF span_b > 0.0 THEN
    out_b := (uc - split) * 100.0 / span_b;
  ELSE
    (* Degenerate: B has no split segment *)
    IF uc >= split THEN
      out_b := 100.0;
    ELSE
      out_b := 0.0;
    END_IF;
  END_IF;
  IF out_b < 0.0 THEN
    out_b := 0.0;
  ELSIF out_b > 100.0 THEN
    out_b := 100.0;
  END_IF;

  (* Segment B reverse action (reverse after clamping, hold the end value outside the segment, output continuous) *)
  IF reverse_b THEN
    out_b := 100.0 - out_b;
  END_IF;
END_FUNCTION_BLOCK
