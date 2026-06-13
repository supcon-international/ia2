(*
  FB_AVG_TIME — tumbling-window time average.

  ~ Standard time-average block, the customary form for DCS reporting / steady-quantity monitoring.

  Purpose: full-window averages for reports (e.g. per-minute average flow), noise-robust
  readings of slowly varying quantities, drift monitoring by comparison against the
  instantaneous value.
  Behavior:
    Within the window, sum / count accumulate each scan; avg_run := sum / count is the
    running mean of the current window; when the window is full (accumulated time
    reaches window_s) avg_run is copied into avg, ready is set, and then the
    accumulators are cleared to start the next window (phase subtraction wraps so the
    long-term cadence does not drift).
    While the first window is not yet full, avg temporarily tracks avg_run (to avoid
    outputting 0 at startup) and ready = FALSE flags "not yet a complete-window result";
    after the first window completes ready stays TRUE and avg refreshes only on the
    window-full scan.
    reset level-active: clears the accumulators and ready, restarting the window.
    window_s <= 0 -> pass-through (avg := avg_run := u, ready := TRUE);
    dt_s <= 0 -> hold (no action).
    Note: this block is a tumbling window (one value per window), not a sliding window
    -- a true sliding window needs an array buffer (in this dialect an FB-scope array
    cannot be codegen'd, see README gotcha 7); for continuous "smoothed every scan"
    output, use FB_LAG exponential smoothing as an approximation.

  Inputs:
    u        REAL  input
    window_s REAL  averaging window length s (<= 0 pass-through)
    dt_s     REAL  sample period s
    reset    BOOL  reset (level-active: clear the window and restart)
  Outputs:
    avg     REAL  average of the previous complete window (tracks avg_run while the first window is incomplete)
    avg_run REAL  running mean of the current window
    ready   BOOL  first complete window has finished (avg is trustworthy)
*)
FUNCTION_BLOCK FB_AVG_TIME
  VAR_INPUT
    u        : REAL;
    window_s : REAL := 60.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    avg     : REAL;
    avg_run : REAL;
    ready   : BOOL;
  END_VAR
  VAR
    sum       : REAL; (* current-window accumulated sum *)
    count     : REAL; (* current-window sample count *)
    elapsed_s : REAL; (* current-window accumulated time, manual timing *)
  END_VAR

  IF reset THEN
    sum := 0.0;
    count := 0.0;
    elapsed_s := 0.0;
    avg := 0.0;
    avg_run := 0.0;
    ready := FALSE;
    RETURN;
  END_IF;

  IF window_s <= 0.0 THEN
    avg := u; (* invalid window length: pass-through *)
    avg_run := u;
    ready := TRUE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  sum := sum + u;
  count := count + 1.0;
  elapsed_s := elapsed_s + dt_s;
  avg_run := sum / count;

  IF elapsed_s >= window_s THEN
    (* Window full: copy the result, clear to start the next window (subtraction wrap keeps the long-term cadence) *)
    avg := avg_run;
    ready := TRUE;
    sum := 0.0;
    count := 0.0;
    elapsed_s := elapsed_s - window_s;
  ELSIF NOT ready THEN
    avg := avg_run; (* first window not yet full: avg tracks the running mean to avoid outputting 0 at startup *)
  END_IF;
END_FUNCTION_BLOCK
