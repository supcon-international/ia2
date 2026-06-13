(*
  FB_STATS — running statistics: min/max/mean/standard deviation (Welford single-pass recurrence).

  ~ Standard statistics block (min/max/average/standard deviation), found in every
  major platform's standard library; this block uses Welford's single-pass
  recurrence -- no arrays, numerically stable.

  Purpose: quality-data statistics (batch temperature mean/spread), measurement
  noise-level assessment, commissioning-time signal profiling (feed FB_AVG_TIME for
  the time average, feed this block for the full-run distribution).
  Behavior (counted by "sample", not consuming dt_s: each scan with enable TRUE counts as 1 sample):
    Welford recurrence (scalar, single-pass, avoiding the precision disaster of
    subtracting squares of large numbers):
      n := n + 1; delta := u - avg; avg := avg + delta / n;
      m2 := m2 + delta * (u - avg)
    sd := SQRT(m2 / n) -- population standard deviation (divide by n, not n-1):
    process monitoring cares about the spread magnitude, the two are indistinguishable
    at large sample counts, and dividing by n is the most direct form in a single-pass
    recurrence.
    reset level-active: clears and re-seeds on the next enabled sample (vmin/vmax/avg
    take that sample's value).
    Note: n is a REAL count; in single precision, after about 1.6E7 samples (about 19
    days at a 100 ms sample) the +1 stops taking effect and the statistics freeze --
    for long-term running, reset per shift / per day.

  Inputs:
    u      REAL  input sample
    enable BOOL  sampling enable (a FALSE scan counts no sample, outputs hold)
    reset  BOOL  clear (level-active, takes priority; re-seeds on the next enabled sample)
  Outputs:
    vmin REAL  minimum value
    vmax REAL  maximum value
    avg  REAL  arithmetic mean
    sd   REAL  population standard deviation
    n    REAL  samples counted so far
*)
FUNCTION_BLOCK FB_STATS
  VAR_INPUT
    u      : REAL;
    enable : BOOL := TRUE;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    vmin : REAL;
    vmax : REAL;
    avg  : REAL;
    sd   : REAL;
    n    : REAL;
  END_VAR
  VAR
    m2     : REAL; (* Welford second-order accumulator sum of (u-avg)*(u-avg') *)
    delta  : REAL;
    delta2 : REAL;
  END_VAR

  IF reset THEN
    n := 0.0;
    avg := 0.0;
    m2 := 0.0;
    sd := 0.0;
    vmin := 0.0;
    vmax := 0.0;
    RETURN;
  END_IF;

  IF NOT enable THEN
    RETURN; (* counts no sample: outputs hold *)
  END_IF;

  IF n <= 0.0 THEN
    (* Seed on the first sample *)
    n := 1.0;
    avg := u;
    m2 := 0.0;
    sd := 0.0;
    vmin := u;
    vmax := u;
    RETURN;
  END_IF;

  n := n + 1.0;
  delta := u - avg;
  avg := avg + delta / n;
  delta2 := u - avg;
  m2 := m2 + delta * delta2;
  sd := SQRT(m2 / n);

  IF u < vmin THEN
    vmin := u;
  END_IF;
  IF u > vmax THEN
    vmax := u;
  END_IF;
END_FUNCTION_BLOCK
