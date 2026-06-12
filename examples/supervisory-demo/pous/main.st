(* Supervisory demo over a (fake) DCS via OPC UA — a slice of the
   jet-mill interlock: the classifier motor current back-controls
   the feeder with hysteresis (>40 A stop, <30 A start), and a flow
   setpoint written from the platform (MQTT northbound) passes through
   to a DCS valve command tag.

   ft0202_pv / m0204_amps come FROM the DCS (iomap inputs);
   fv0203_cmd / feeder_run go TO the DCS (iomap outputs);
   sp_flow arrives via the northbound write topic. *)
PROGRAM main
  VAR
    ft0202_pv   : REAL := 0.0;   (* feed flow PV, m3/h *)
    m0204_amps  : REAL := 0.0;   (* classifier current, A *)
    sp_flow     : REAL := 0.0;   (* platform-written setpoint *)
    fv0203_cmd  : REAL := 0.0;   (* valve command to DCS *)
    feeder_run  : BOOL := FALSE; (* feeder command to DCS *)
    high_amps   : BOOL := FALSE; (* hysteresis state *)
  END_VAR

  (* Hysteresis: current > 40 A -> stop feeder; < 30 A -> run again. *)
  IF m0204_amps > 40.0 THEN
    high_amps := TRUE;
  ELSIF m0204_amps < 30.0 THEN
    high_amps := FALSE;
  END_IF;
  feeder_run := NOT high_amps;

  (* Platform setpoint passes through to the DCS valve command. *)
  fv0203_cmd := sp_flow;
END_PROGRAM
