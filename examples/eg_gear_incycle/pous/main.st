(*=====================================================================
  eg_gear_incycle — in-cycle electronic gear demo (B-tier motion)

  The follower's target_position is generated INSIDE the cyclic loop by
  the gear engine (see devices/servo.toml [[gear]]), strictly SYNC0-
  aligned. This PLC program only:
    - drives the follower's CiA402 enable chain (controlword/statusword),
    - feeds the gear's slow parameters through the gear_* channels,
    - reads back the engine's engaged/trip state.
  It never writes target_position — the loop owns those bytes.

  The engine enforces its own fast-plane safety (documented in
  crates/iomap-ethercat/src/gear.rs): shadow-actual until Operation
  Enabled (zero-jump), engage refused unless max_travel > 0, ratio/phase
  latched at the engage edge, overtravel hard-clamped then tripped,
  re-arm required after an enable loss, non-finite params rejected.

  Try it under nic = "_sim":
    cs project open examples/eg_gear_incycle && cs --project eg_gear_incycle run
    # the sim models an ideal always-enabled follower, so:
    cs --project eg_gear_incycle runtime force max_travel 8388608   # 1 rev limit
    cs --project eg_gear_incycle runtime force master_vel  200      # cnt/cycle
    cs --project eg_gear_incycle runtime force ratio_num   2
    cs --project eg_gear_incycle runtime force ratio_den   1
    cs --project eg_gear_incycle runtime force ratio_step  0.02
    cs --project eg_gear_incycle runtime force engage      1
    # read /status: target_position climbs at ~2x master_vel until the
    # 1-rev limit, then gear_trip latches and it holds.
=====================================================================*)


(*---------------------------------------------------------------------
  FB CIA402_CSP — minimal CiA402 enable state machine (controlword out).
---------------------------------------------------------------------*)
FUNCTION_BLOCK CIA402_CSP
  VAR_INPUT
    status_word : WORD;
    enable      : BOOL;
    reset_fault : BOOL;
  END_VAR
  VAR_OUTPUT
    control_word : WORD;
    ready        : BOOL;
    state        : INT;
  END_VAR
  VAR
    prev_reset : BOOL;
    sw_lo      : WORD;
  END_VAR

  sw_lo := status_word AND WORD#16#6F;

  IF (status_word AND WORD#16#4F) = WORD#16#08 THEN
    state := 5;                    (* Fault *)
  ELSIF (status_word AND WORD#16#4F) = WORD#16#40 THEN
    state := 1;                    (* Switch On Disabled *)
  ELSIF sw_lo = WORD#16#21 THEN
    state := 2;                    (* Ready To Switch On *)
  ELSIF sw_lo = WORD#16#23 THEN
    state := 3;                    (* Switched On *)
  ELSIF sw_lo = WORD#16#27 THEN
    state := 4;                    (* Operation Enabled *)
  ELSE
    state := 0;
  END_IF;

  IF state = 5 THEN
    IF reset_fault AND NOT prev_reset THEN
      control_word := WORD#16#80;
    ELSE
      control_word := WORD#16#00;
    END_IF;
  ELSIF NOT enable THEN
    control_word := WORD#16#06;
  ELSIF state = 1 THEN
    control_word := WORD#16#06;
  ELSIF state = 2 THEN
    control_word := WORD#16#07;
  ELSE
    control_word := WORD#16#0F;
  END_IF;

  ready := (state = 4);
  prev_reset := reset_fault;
END_FUNCTION_BLOCK


(*---------------------------------------------------------------------
  PROGRAM main — enable chain + gear parameter passthrough.
---------------------------------------------------------------------*)
PROGRAM main
  VAR
    (* PDO I/O (iomap). Note: target_position is NOT mapped — the gear
       engine owns it. *)
    status_word  : WORD;
    control_word : WORD;
    actual_pos   : DINT;

    (* Gear slow-plane parameters (mapped to the gear_* channels -> engine). *)
    g_engage     : BOOL;
    g_ratio_num  : REAL;
    g_ratio_den  : REAL;
    g_ratio_step : REAL;
    g_phase      : REAL;
    g_master_vel : REAL;
    g_max_travel : REAL;
    g_engaged_fb : BOOL;   (* engine -> engaged *)
    g_trip_fb    : BOOL;   (* engine -> overtravel trip *)

    (* Operator setpoints (force / bind to HMI). *)
    enable       : BOOL;
    reset_fault  : BOOL;
    engage       : BOOL;
    ratio_num    : REAL;
    ratio_den    : REAL;
    ratio_step   : REAL;
    phase_ofs    : REAL;
    master_vel   : REAL;   (* virtual master speed, counts/cycle *)
    max_travel   : REAL;   (* follower travel limit, counts; <=0 = locked *)

    (* Monitoring. *)
    state        : INT;
    ready        : BOOL;
    fault        : BOOL;

    sm           : CIA402_CSP;
    heartbeat    : UDINT;
  END_VAR

  heartbeat := heartbeat + 1;

  (* CiA402 enable chain for the follower (slow plane; scan rate is fine). *)
  sm(status_word := status_word, enable := enable, reset_fault := reset_fault,
     control_word => control_word, ready => ready, state => state);
  fault := (state = 5);

  (* Pass gear parameters straight through to the engine's channels. All
     the safety interlocks live in the engine (fast plane), so the program
     just forwards operator intent and reads state back. *)
  g_engage     := engage;
  g_ratio_num  := ratio_num;
  g_ratio_den  := ratio_den;
  g_ratio_step := ratio_step;
  g_phase      := phase_ofs;
  g_master_vel := master_vel;
  g_max_travel := max_travel;

END_PROGRAM
