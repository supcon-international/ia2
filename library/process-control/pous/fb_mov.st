(*
  FB_MOV — motor-operated valve (open/stop/close three commands + ZSO/ZSC dual limits + travel timeout).

  ~ Standard motor-operated-valve block (open/stop/close type, with travel monitoring),
  found in every major DCS/PLC standard library.

  Purpose: standard wrapper for "open-stop-close"-type motor-operated valves such as
  motorized gate/butterfly valves. Difference from FB_VALVE (two-position valve, single
  held output): this block is dual-direction pulse-driven with mid-travel stop, and the
  motor stops automatically at the limit; difference from FB_PID_3STEP (modulating step
  type): this block does only full-open/full-close/mid-stop sequential operations, no
  closed-loop control.

  Behavior:
    1) Fault monitoring: zso and zsc TRUE at once -> limit-switch fault, immediate fault;
       travel (an output active in either direction and the target limit not reached)
       exceeding travel_s -> travel-timeout fault (stuck valve / lost limit), and the
       outputs stop. All latched; cleared by reset with the dual-limit condition cleared
       (the travel timer is cleared in step).
    2) Commands: cmd_stop or fault -> both outputs stop (priority; cmd_open and cmd_close
       commanded together in the same scan is treated as a conflict, handled as stop);
       cmd_open latches the open output, cmd_close latches the close output (mutually
       exclusive, a new command is a new travel, the travel timer is cleared).
    3) Stop-at-position: zso reached during q_open -> stop the open output; zsc reached
       during q_close -> stop the close output.
    4) Status word pos_state: 0 fully closed / 1 opening / 2 fully open / 3 closing /
       4 faulted; a mid-stop (no output and not at a limit) holds the previous status value.

  Inputs:
    cmd_open  BOOL  open command
    cmd_close BOOL  close command
    cmd_stop  BOOL  stop command (priority, mid-stop)
    zso       BOOL  open limit
    zsc       BOOL  close limit
    travel_s  REAL  travel timeout s (<= 0 = not monitored)
    reset     BOOL  fault reset
    dt_s      REAL  sample period s
  Outputs:
    q_open    BOOL  open-direction output
    q_close   BOOL  close-direction output
    pos_state REAL  0 fully closed / 1 opening / 2 fully open / 3 closing / 4 faulted
    fault     BOOL  fault (latched)
*)
FUNCTION_BLOCK FB_MOV
  VAR_INPUT
    cmd_open  : BOOL;
    cmd_close : BOOL;
    cmd_stop  : BOOL;
    zso       : BOOL;
    zsc       : BOOL;
    travel_s  : REAL := 60.0;
    reset     : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_open    : BOOL;
    q_close   : BOOL;
    pos_state : REAL;
    fault     : BOOL;
  END_VAR
  VAR
    travel_t : REAL; (* current-travel accumulated time s *)
  END_VAR

  (* 1) Dual-limit fault (immediate) *)
  IF zso AND zsc THEN
    fault := TRUE; (* both limits active at once: limit-switch fault *)
  END_IF;

  (* Reset (no effect while the dual-limit condition is still present) *)
  IF reset AND NOT (zso AND zsc) THEN
    fault    := FALSE;
    travel_t := 0.0;
  END_IF;

  (* 2) Command latch: stop/fault/both-commands-same-scan -> full stop; open and close mutually exclusive, a new command is a new travel *)
  IF cmd_stop OR fault OR (cmd_open AND cmd_close) THEN
    q_open  := FALSE;
    q_close := FALSE;
  ELSIF cmd_open THEN
    IF NOT q_open THEN
      travel_t := 0.0; (* new travel: clear the timer *)
    END_IF;
    q_open  := TRUE;
    q_close := FALSE;
  ELSIF cmd_close THEN
    IF NOT q_close THEN
      travel_t := 0.0;
    END_IF;
    q_close := TRUE;
    q_open  := FALSE;
  END_IF;

  (* 3) Stop-at-position *)
  IF q_open AND zso THEN
    q_open := FALSE;
  END_IF;
  IF q_close AND zsc THEN
    q_close := FALSE;
  END_IF;

  (* 4) Travel timeout (manual dt_s accumulation) *)
  IF q_open OR q_close THEN
    travel_t := travel_t + dt_s;
    IF travel_s > 0.0 AND travel_t >= travel_s THEN
      fault   := TRUE; (* travel timeout: stuck valve / lost limit *)
      q_open  := FALSE;
      q_close := FALSE;
    END_IF;
  ELSE
    travel_t := 0.0;
  END_IF;

  (* 5) Status word (a mid-stop holds the previous status value) *)
  IF fault THEN
    pos_state := 4.0;
  ELSIF zso THEN
    pos_state := 2.0;
  ELSIF zsc THEN
    pos_state := 0.0;
  ELSIF q_open THEN
    pos_state := 1.0;
  ELSIF q_close THEN
    pos_state := 3.0;
  END_IF;
END_FUNCTION_BLOCK
