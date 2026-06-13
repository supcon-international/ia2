(*
  FB_RUNTIME — equipment run-hours + start-count statistics.

  ~ Equipment run statistics (accumulated run hours + start count), standard
  maintenance data.

  Purpose: pump/fan wear balancing (feed hours to FB_DUTY2's hours_a/b with alt_mode=1),
  maintenance-due reminders, start/stop frequency statistics.
  Behavior:
    while running is TRUE, hours := hours + dt_s/3600;
    on a running rising edge, starts := starts + 1;
    reset level-active clears both (priority, no accumulation during a reset scan).
    The caller may place the instance under VAR RETAIN for retention across power loss
    (same caveat as FB_TOTALIZER: IA2 retain stores as i32, so the retention precision
    of the REAL hours is limited by this; starts is an integer count and is unaffected).

  Inputs:
    running BOOL  equipment running (feed the run feedback, debounce with FB_DEBOUNCE if needed)
    reset   BOOL  clear (level-active, priority; used after maintenance / pump replacement)
    dt_s    REAL  sample period s
  Outputs:
    hours  REAL  accumulated run hours
    starts DINT  accumulated start count
*)
FUNCTION_BLOCK FB_RUNTIME
  VAR_INPUT
    running : BOOL;
    reset   : BOOL := FALSE;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    hours  : REAL;
    starts : DINT;
  END_VAR
  VAR
    prev_run : BOOL; (* rising-edge memory *)
  END_VAR

  IF reset THEN
    hours := 0.0;
    starts := 0;
  ELSE
    IF running AND NOT prev_run THEN
      starts := starts + 1;
    END_IF;
    IF running AND dt_s > 0.0 THEN
      hours := hours + dt_s / 3600.0;
    END_IF;
  END_IF;
  prev_run := running;
END_FUNCTION_BLOCK
