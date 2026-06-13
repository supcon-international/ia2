(*
  FB_GAIN_SCHED3 — three-zone gain scheduling (gain scheduling, hysteresis zoning +
  optional zone-boundary linear transition).

  ~ Standard gain scheduler / parameter scheduler (switches the PID parameter set by
  zone of an operating variable), the customary block for segmented tuning of a
  nonlinear plant.

  Purpose: when the plant gain varies greatly with operating point (load segments,
  valve-characteristic segments, the strongly nonlinear pH segment), output a kp/ti/td
  parameter set in three zones of a scheduling variable (load / opening / the PV itself);
  the output feeds the parameter inputs of FB_PID / FB_PID_POS directly -- an online
  parameter change takes effect immediately (both blocks compute their increment/terms
  from the current parameters).

  Behavior:
    Zoning (u_sched against the two boundaries x12 / x23, assuming x12 < x23):
      zone 1: u <= x12; zone 2: x12 < u <= x23; zone 3: u > x23.
    Hysteresis: a zone change requires crossing the boundary +/- hyst (preventing
    chatter near a boundary from toggling parameters back and forth); the first scan
    seeds by plain comparison. A large step can jump two zones within one scan (it
    settles by stepwise migration).
    Parameter output:
      interp = FALSE: take the current zone's parameter set (hysteresis switching, with
      stepped parameters -- feeding incremental FB_PID is naturally bumpless, feeding
      FB_PID_POS recomputes the P term immediately at the new kp);
      interp = TRUE and hyst > 0: when u falls within +/-hyst of a boundary the two
      zones' parameters transition linearly (parameter output continuous, determined by
      u_sched alone; zone still gives its discrete indication by hysteresis).
    Note: when interp mixes a zone with ti = 0 (integral off) and one with ti > 0, the
    transition segment produces a very small ti (very strong integral) -- each zone's
    ti/td should be all 0 or all non-zero.

  Inputs:
    u_sched         REAL  scheduling variable (load / valve position / PV, etc.)
    x12             REAL  zone 1/2 boundary
    x23             REAL  zone 2/3 boundary (> x12, and the two transition bands must not overlap)
    hyst            REAL  zone-boundary hysteresis half-width (0 = no hysteresis)
    interp          BOOL  TRUE = linear parameter transition within +/-hyst of a boundary
    kp1, ti1_s, td1_s REAL  zone 1 parameter set
    kp2, ti2_s, td2_s REAL  zone 2 parameter set
    kp3, ti3_s, td3_s REAL  zone 3 parameter set
  Outputs:
    kp_out   REAL  scheduled proportional gain (to the PID's kp)
    ti_out_s REAL  scheduled integral time s
    td_out_s REAL  scheduled derivative time s
    zone     REAL  current zone 1 / 2 / 3 (hysteresis decision)
*)
FUNCTION_BLOCK FB_GAIN_SCHED3
  VAR_INPUT
    u_sched : REAL;
    x12     : REAL := 33.3;
    x23     : REAL := 66.6;
    hyst    : REAL := 0.0;
    interp  : BOOL := FALSE;
    kp1     : REAL := 1.0;
    ti1_s   : REAL := 0.0;
    td1_s   : REAL := 0.0;
    kp2     : REAL := 1.0;
    ti2_s   : REAL := 0.0;
    td2_s   : REAL := 0.0;
    kp3     : REAL := 1.0;
    ti3_s   : REAL := 0.0;
    td3_s   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    kp_out   : REAL;
    ti_out_s : REAL;
    td_out_s : REAL;
    zone     : REAL;
  END_VAR
  VAR
    f : REAL; (* interpolation fraction within the transition band 0..1 *)
  END_VAR

  (* 1) Zone selection (hysteresis): first scan zone = 0 -> seed by plain comparison *)
  IF zone < 0.5 THEN
    IF u_sched <= x12 THEN
      zone := 1.0;
    ELSIF u_sched <= x23 THEN
      zone := 2.0;
    ELSE
      zone := 3.0;
    END_IF;
  END_IF;

  (* Stepwise migration (test 1->2, 3->2 first, then 2->1/2->3: a large step settles in one scan) *)
  IF zone <= 1.5 THEN
    IF u_sched > x12 + hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 2.5 THEN
    IF u_sched < x23 - hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 1.5 AND zone <= 2.5 THEN
    IF u_sched > x23 + hyst THEN
      zone := 3.0;
    ELSIF u_sched < x12 - hyst THEN
      zone := 1.0;
    END_IF;
  END_IF;

  (* 2) Zone parameters (discrete values, hysteresis switching) *)
  IF zone <= 1.5 THEN
    kp_out   := kp1;
    ti_out_s := ti1_s;
    td_out_s := td1_s;
  ELSIF zone <= 2.5 THEN
    kp_out   := kp2;
    ti_out_s := ti2_s;
    td_out_s := td2_s;
  ELSE
    kp_out   := kp3;
    ti_out_s := ti3_s;
    td_out_s := td3_s;
  END_IF;

  (* 3) interp: linear transition within +/-hyst of a boundary (parameters continuous,
        determined by u_sched alone, overriding the discrete values above; the zone
        output is unaffected) *)
  IF interp AND hyst > 0.0 THEN
    IF (u_sched > x12 - hyst) AND (u_sched < x12 + hyst) THEN
      f := (u_sched - (x12 - hyst)) / (2.0 * hyst);
      kp_out   := kp1 + f * (kp2 - kp1);
      ti_out_s := ti1_s + f * (ti2_s - ti1_s);
      td_out_s := td1_s + f * (td2_s - td1_s);
    ELSIF (u_sched > x23 - hyst) AND (u_sched < x23 + hyst) THEN
      f := (u_sched - (x23 - hyst)) / (2.0 * hyst);
      kp_out   := kp2 + f * (kp3 - kp2);
      ti_out_s := ti2_s + f * (ti3_s - ti2_s);
      td_out_s := td2_s + f * (td3_s - td2_s);
    ELSIF u_sched <= x12 - hyst THEN
      kp_out   := kp1;
      ti_out_s := ti1_s;
      td_out_s := td1_s;
    ELSIF u_sched >= x23 + hyst THEN
      kp_out   := kp3;
      ti_out_s := ti3_s;
      td_out_s := td3_s;
    ELSE
      kp_out   := kp2; (* between the two transition bands: pure zone 2 *)
      ti_out_s := ti2_s;
      td_out_s := td2_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
