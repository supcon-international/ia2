(*
  FB_MOTOR — motor start/stop control, with run-feedback monitoring and fault latch.

  Purpose: standard start/stop wrapper for single-speed motors such as belt conveyors,
  fans and pumps.
  Example: in remote mode an HMI/supervisory start/stop command -> contactor output,
  and a timed mismatch of the run feedback (contactor aux contact / current relay)
  faults the motor to stop.

  Behavior:
    1) Fault monitoring: a mismatch between out_run and run_fb persisting for
       fb_timeout_s (manual dt_s accumulation) -> fault latched; an external fault
       fault_in (thermal overload / drive fault, etc.) -> fault. reset with fault_in
       already cleared clears the latch.
    2) Command latch: cmd_start (level or momentary) sets out_run when remote and
       fault-free; cmd_stop or fault resets it (stop takes priority).
    3) Status word status: 0 stopped, 1 starting (output on, feedback not yet seen),
       2 running, 3 faulted.

  Inputs:
    cmd_start    BOOL  start command
    cmd_stop     BOOL  stop command (priority)
    remote       BOOL  remote permissive (gates start only; does not force a stop)
    run_fb       BOOL  run feedback
    fault_in     BOOL  external fault input
    reset        BOOL  fault reset
    fb_timeout_s REAL  feedback-mismatch tolerance time s
    dt_s         REAL  sample period s
  Outputs:
    out_run BOOL  run output (contactor / drive start)
    fault   BOOL  fault (latched)
    status  INT   0 stopped / 1 starting / 2 running / 3 faulted
*)
FUNCTION_BLOCK FB_MOTOR
  VAR_INPUT
    cmd_start    : BOOL;
    cmd_stop     : BOOL;
    remote       : BOOL := TRUE;
    run_fb       : BOOL;
    fault_in     : BOOL;
    reset        : BOOL;
    fb_timeout_s : REAL := 5.0;
    dt_s         : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out_run : BOOL;
    fault   : BOOL;
    status  : INT;
  END_VAR
  VAR
    mism_t : REAL; (* feedback-mismatch accumulated time s *)
  END_VAR

  (* 1) Fault monitoring *)
  IF out_run <> run_fb THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF fault_in THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT fault_in THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) Command latch (stop/fault priority) *)
  IF cmd_start AND remote AND NOT fault THEN
    out_run := TRUE;
  END_IF;
  IF cmd_stop OR fault THEN
    out_run := FALSE;
  END_IF;

  (* 3) Status word *)
  IF fault THEN
    status := 3;
  ELSIF out_run AND run_fb THEN
    status := 2;
  ELSIF out_run THEN
    status := 1;
  ELSE
    status := 0;
  END_IF;
END_FUNCTION_BLOCK
