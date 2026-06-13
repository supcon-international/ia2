(*
  FB_CASCADE — cascade coupling glue (master output -> slave SP mapping + non-cascade tracking back-calculation).

  ~ Standard cascade-coupling block (cascade SP coupling / master tracking
  back-calculation), the customary glue for cascade-loop configuration: only this one
  block sits between the two PIDs.

  Purpose: cascade-loop configuration (temperature master -> flow slave, and the like),
  mapping the master output 0..100% to the slave SP engineering span, and supplying the
  master a tracking value when not in cascade so that putting CAS into/out of service is
  bumpless.
  Behavior:
    CAS (cas_on = TRUE):
      slave_sp := sp_min + master_out / 100 * (sp_max - sp_min)
      master_track := FALSE (the master controls normally).
    Non-CAS:
      slave_sp := slave_sp_local (the operator's local SP given directly to the slave loop);
      master_track := TRUE, master_track_value :=
        (slave_sp_now - sp_min) / (sp_max - sp_min) * 100, clamped 0..100
      -- the master output is forced to "the equivalent percentage of the current slave
      SP", so on the scan that switches back to CAS slave_sp does not jump (bumpless transfer).
      Span not configured (|sp_max - sp_min| < 1E-6) -> tracking value takes 0 (divide-by-zero guard).
    The tracking value is computed every scan (independent of mode), so it is valid the
    instant CAS is removed.
    Pure combinational logic, no internal state.

  Wiring (cascading two FB_PID instances; slave_sp_now is fed the previous scan's working SP):
    casc(master_out := pid_master.out, cas_on := cas_enable,
         sp_min := 0.0, sp_max := 2000.0,
         slave_sp_local := op_sp, slave_sp_now := slave_sp_used);
    slave_sp_used := casc.slave_sp;
    pid_master(pv := temp_pv, sp := temp_sp, ..., dt_s := scan_s,
               track := casc.master_track,
               track_value := casc.master_track_value);
    pid_slave(pv := flow_pv, sp := slave_sp_used, ..., dt_s := scan_s);

  Inputs:
    master_out     REAL  master output 0..100 %
    cas_on         BOOL  cascade enabled (FALSE = slave uses local SP, master tracks)
    sp_min         REAL  slave SP span low (engineering units)
    sp_max         REAL  slave SP span high
    slave_sp_local REAL  slave-loop local SP when not in cascade
    slave_sp_now   REAL  slave-loop current working SP (feed the previous scan's slave_sp, for the tracking back-calculation)
  Outputs:
    slave_sp           REAL  slave-loop setpoint (to pid_slave.sp)
    master_track       BOOL  master tracking command (to pid_master.track)
    master_track_value REAL  master tracking value 0..100 % (to pid_master.track_value)
*)
FUNCTION_BLOCK FB_CASCADE
  VAR_INPUT
    master_out     : REAL;
    cas_on         : BOOL := FALSE;
    sp_min         : REAL := 0.0;
    sp_max         : REAL := 100.0;
    slave_sp_local : REAL := 0.0;
    slave_sp_now   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    slave_sp           : REAL;
    master_track       : BOOL;
    master_track_value : REAL;
  END_VAR
  VAR
    span : REAL; (* slave-loop SP span width *)
  END_VAR

  span := sp_max - sp_min;

  (* Tracking value computed every scan: the equivalent percentage of the current slave SP (span not configured -> 0, divide-by-zero guard) *)
  IF (span < 0.000001) AND (span > -0.000001) THEN
    master_track_value := 0.0;
  ELSE
    master_track_value := (slave_sp_now - sp_min) / span * 100.0;
    IF master_track_value < 0.0 THEN
      master_track_value := 0.0;
    ELSIF master_track_value > 100.0 THEN
      master_track_value := 100.0;
    END_IF;
  END_IF;

  IF cas_on THEN
    slave_sp := sp_min + master_out / 100.0 * span;
    master_track := FALSE;
  ELSE
    slave_sp := slave_sp_local;
    master_track := TRUE;
  END_IF;
END_FUNCTION_BLOCK
