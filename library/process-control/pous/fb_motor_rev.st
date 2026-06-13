(*
  FB_MOTOR_REV — bidirectional (forward/reverse) motor control, with reversal interlock,
  run-feedback monitoring and fault latch.

  ~ Standard reversing-motor block (reversing motor / forward-reverse starter, including
  the reversal-interlock delay), found in every major DCS/PLC standard library.

  Purpose: standard wrapper for reversible-running equipment such as agitators, hoists,
  bidirectional conveyors and gate-opening motors (pattern following FB_MOTOR: command
  latch + feedback-timeout fault latch).

  Behavior:
    1) Fault monitoring: fb_fwd and fb_rev TRUE at once -> feedback/contactor fault,
       immediate fault; an output-feedback mismatch (q_fwd<>fb_fwd or q_rev<>fb_rev)
       persisting for fb_timeout_s (manual dt_s accumulation) -> fault. All latched;
       cleared by reset with the dual-feedback condition cleared.
    2) Direction command latch (stop/fault priority): cmd_fwd / cmd_rev set the
       corresponding direction request (a both-direction command in the same scan is
       treated as a conflict, no action); stop or fault clears all requests.
    3) Reversal interlock: on a reversal (the reverse request is still present, or the
       reverse feedback is still turning), both outputs are forced fully stopped, and a
       new-direction output is allowed only after reversal_delay_s -- the contactor
       arc-quench / mechanical run-down time. reversal_delay_s should be less than
       fb_timeout_s (residual feedback rotation during the interlock wait is covered by
       the timeout tolerance).
    4) Status word state: 0 stopped / 1 forward / 2 reverse / 3 faulted / 4 reversal wait.

  Inputs:
    cmd_fwd          BOOL  forward command
    cmd_rev          BOOL  reverse command
    stop             BOOL  stop command (priority)
    fb_fwd           BOOL  forward run feedback
    fb_rev           BOOL  reverse run feedback
    fb_timeout_s     REAL  feedback-mismatch tolerance time s
    reversal_delay_s REAL  reversal full-stop interlock time s
    reset            BOOL  fault reset
    dt_s             REAL  sample period s
  Outputs:
    q_fwd BOOL  forward output
    q_rev BOOL  reverse output
    fault BOOL  fault (latched)
    state REAL  0 stopped / 1 forward / 2 reverse / 3 faulted / 4 reversal wait
*)
FUNCTION_BLOCK FB_MOTOR_REV
  VAR_INPUT
    cmd_fwd          : BOOL;
    cmd_rev          : BOOL;
    stop             : BOOL;
    fb_fwd           : BOOL;
    fb_rev           : BOOL;
    fb_timeout_s     : REAL := 5.0;
    reversal_delay_s : REAL := 2.0;
    reset            : BOOL;
    dt_s             : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_fwd : BOOL;
    q_rev : BOOL;
    fault : BOOL;
    state : REAL;
  END_VAR
  VAR
    want_fwd : BOOL; (* latched direction request *)
    want_rev : BOOL;
    wait_t   : REAL; (* reversal-interlock remaining time s *)
    mism_t   : REAL; (* feedback-mismatch accumulated time s *)
    conflict : BOOL; (* both-direction command in the same scan *)
  END_VAR

  (* 1) Fault monitoring *)
  IF fb_fwd AND fb_rev THEN
    fault := TRUE; (* both feedbacks active at once: welded contactor / feedback-circuit fault *)
  END_IF;
  IF (q_fwd <> fb_fwd) OR (q_rev <> fb_rev) THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT (fb_fwd AND fb_rev) THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) Direction command latch (stop/fault priority; a both-direction command in the same scan conflicts, no action) *)
  conflict := cmd_fwd AND cmd_rev;
  IF cmd_fwd AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_rev OR fb_rev THEN
      wait_t := reversal_delay_s; (* reverse still commanded / still turning: full-stop interlock first *)
    END_IF;
    want_fwd := TRUE;
    want_rev := FALSE;
  ELSIF cmd_rev AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_fwd OR fb_fwd THEN
      wait_t := reversal_delay_s;
    END_IF;
    want_rev := TRUE;
    want_fwd := FALSE;
  END_IF;
  IF stop OR fault THEN
    want_fwd := FALSE;
    want_rev := FALSE;
  END_IF;

  (* 3) Reversal-interlock timing (manual dt_s decrement) *)
  IF wait_t > 0.0 THEN
    wait_t := wait_t - dt_s;
    IF wait_t < 0.0 THEN
      wait_t := 0.0;
    END_IF;
  END_IF;

  (* 4) Output: during the interlock both outputs are forced fully stopped *)
  q_fwd := want_fwd AND wait_t <= 0.0;
  q_rev := want_rev AND wait_t <= 0.0;

  (* 5) Status word *)
  IF fault THEN
    state := 3.0;
  ELSIF wait_t > 0.0 AND (want_fwd OR want_rev) THEN
    state := 4.0;
  ELSIF q_fwd THEN
    state := 1.0;
  ELSIF q_rev THEN
    state := 2.0;
  ELSE
    state := 0.0;
  END_IF;
END_FUNCTION_BLOCK
