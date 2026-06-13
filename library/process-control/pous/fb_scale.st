(*
  FB_SCALE — linear span conversion: raw counts -> engineering units, with NE43-style open-circuit flag.

  Purpose: analog input scaling. For example, a 4-20 mA signal sampled as raw
  counts (S7-style 5530..27648) is converted to an engineering value such as
  0..100 % or 0..5 m.

  Formula: out := out_min + (raw - raw_min) * (out_max - out_min) / span
           where span = MAX(raw_max - raw_min, 1e-6) (inline IF, divide-by-zero guard).
  Assumes raw_max > raw_min (forward span).

  Inputs:
    raw      REAL  raw count value (e.g. AI channel raw value)
    raw_min  REAL  raw value at span low (4 mA point)
    raw_max  REAL  raw value at span high (20 mA point)
    out_min  REAL  engineering span low
    out_max  REAL  engineering span high
    clamp    BOOL  TRUE clamps output to [out_min, out_max] (reverse engineering span supported)
  Outputs:
    out      REAL  engineering value
    fault    BOOL  raw value out of range >5% of span (NE43-style open-circuit/over-range flag, not latched)
*)
FUNCTION_BLOCK FB_SCALE
  VAR_INPUT
    raw     : REAL;
    raw_min : REAL := 0.0;
    raw_max : REAL := 27648.0;
    out_min : REAL := 0.0;
    out_max : REAL := 100.0;
    clamp   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out   : REAL;
    fault : BOOL;
  END_VAR
  VAR
    span   : REAL;
    margin : REAL;
    lo     : REAL;
    hi     : REAL;
  END_VAR

  (* span = MAX(raw_max - raw_min, 1e-6) — inline IF, divide-by-zero guard *)
  span := raw_max - raw_min;
  IF span < 0.000001 THEN
    span := 0.000001;
  END_IF;

  (* NE43-style out-of-range test: more than 5% beyond raw span = open-circuit/fault *)
  margin := 0.05 * span;
  fault := (raw < raw_min - margin) OR (raw > raw_max + margin);

  out := out_min + (raw - raw_min) * (out_max - out_min) / span;

  IF clamp THEN
    IF out_min <= out_max THEN
      lo := out_min;
      hi := out_max;
    ELSE
      lo := out_max;
      hi := out_min;
    END_IF;
    IF out < lo THEN
      out := lo;
    ELSIF out > hi THEN
      out := hi;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
