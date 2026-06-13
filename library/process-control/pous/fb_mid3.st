(*
  FB_MID3 — median select / 2oo3 analog voting.

  ~ Standard median-select / redundant-measurement voting block (median selector,
  2oo3 voting), the customary block for redundant-transmitter signal selection.

  Purpose: median of three redundant transmitters (rejects single-point drift/jump),
  average of dual redundancy, bad-value degradation of critical measurements (pair
  with the inverse of FB_SCALE fault as good_x).
  Behavior (degrades by the number of good channels n_good):
    3 good -> take the median (naturally rejects single-channel drift);
    2 good -> take the average of the two good channels;
    1 good -> pass that channel through;
    0 good -> out holds the previous scan's value, q_fail = TRUE (level, not latched).
    Deviation monitor: when dev_max > 0, the difference between any two good channels
    > dev_max -> q_dev (signals that drift between redundant channels has widened and
    a transmitter check is due; dev_max <= 0 disables the monitor).
    good_x takes the bad-value decision (transmitter open-circuit/over-range, e.g. the
    inverse of FB_SCALE.fault), defaulting to TRUE = all good.
    Note: when there are 0 good on the very first scan, out is its initial value 0 (no
    history to hold); the held value during q_fail is "the voted value from the last
    time a good channel existed".

  Inputs:
    a, b, c                REAL  three redundant measurements
    good_a, good_b, good_c BOOL  per-channel validity (default TRUE; FALSE = excluded)
    dev_max                REAL  deviation-monitor limit between good channels (<= 0 = disabled)
  Outputs:
    out    REAL  voted output (held when 0 good)
    n_good REAL  current number of good channels 0..3
    q_dev  BOOL  deviation between good channels over limit
    q_fail BOOL  all channels failed (output is in hold)
*)
FUNCTION_BLOCK FB_MID3
  VAR_INPUT
    a       : REAL;
    b       : REAL;
    c       : REAL;
    good_a  : BOOL := TRUE;
    good_b  : BOOL := TRUE;
    good_c  : BOOL := TRUE;
    dev_max : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    n_good : REAL;
    q_dev  : BOOL;
    q_fail : BOOL;
  END_VAR

  n_good := 0.0;
  IF good_a THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_b THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_c THEN
    n_good := n_good + 1.0;
  END_IF;

  q_fail := n_good <= 0.0;

  (* Deviation monitor: any two good channels differ by more than dev_max (inline IF instead of ABS, library convention) *)
  q_dev := FALSE;
  IF dev_max > 0.0 THEN
    IF good_a AND good_b THEN
      IF (a - b > dev_max) OR (b - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_a AND good_c THEN
      IF (a - c > dev_max) OR (c - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_b AND good_c THEN
      IF (b - c > dev_max) OR (c - b > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
  END_IF;

  IF n_good >= 3.0 THEN
    (* 3 good: take the median *)
    IF (a >= b AND a <= c) OR (a <= b AND a >= c) THEN
      out := a;
    ELSIF (b >= a AND b <= c) OR (b <= a AND b >= c) THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  ELSIF n_good >= 2.0 THEN
    (* 2 good: take the average *)
    IF NOT good_a THEN
      out := (b + c) / 2.0;
    ELSIF NOT good_b THEN
      out := (a + c) / 2.0;
    ELSE
      out := (a + b) / 2.0;
    END_IF;
  ELSIF n_good >= 1.0 THEN
    (* 1 good: pass-through *)
    IF good_a THEN
      out := a;
    ELSIF good_b THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  END_IF;
  (* 0 good: out holds the previous scan's value, q_fail signals it *)
END_FUNCTION_BLOCK
