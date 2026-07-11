(* ============================================================================
  demo_main — mini carbonation-tower loop demonstration PROGRAM.

  The demo instantiates the process-control function blocks that each live in
  their own sibling file (fb_scale.st, fb_pid.st, fb_alarm_hl.st, ...). It is
  meant to be checked together with them: `cs check pous/*.st` resolves the FB
  types across all the files at once, so no block declaration is repeated here.

  Loop wiring (demonstration; variables can be bound to real channels via iomap.toml):
    lt0101_raw (AI raw counts) -> FB_SCALE -> level_pct (tower level %)
    level_pct / level_sp     -> FB_PID (reverse-acting, legacy-signature call, verifies backward compatibility)
                               -> valve_pct (feed control-valve opening)
    level_pct                -> FB_ALARM_HL -> four-level level alarms + any_alm
    hammer_enable            -> FB_PULSER -> hammer_q (periodic air-cannon purge)
    level_pct                -> FB_HILO_FILL -> fill_open (high-close low-open make-up)
    -- the following are extension loops --
    level HH / open-circuit / tower-temp HH / dual-pump fault -> FB_INTERLOCK8 -> ilk_trip + first-out number
    pump_run_fb_a/b          -> FB_RUNTIME x2 -> dual-pump hours / start counts
    fill_open AND NOT ilk    -> FB_DUTY2 (hour balancing) -> dual make-up pumps duty/standby
    gas_flow (leading flow)  -> FB_RATIO -> milk_sp (lime-milk ratio setpoint)
    milk_flow / milk_sp      -> FB_PID (feedforward ff + SP ramp + interlock track forcing)
                               -> FB_SPLIT_RANGE (sequential split range) -> small/large valve
    -- the following demonstrate the math/dynamics blocks (steam-heating cascade + FOPDT plant simulation) --
    steam_valve -> FB_DEADTIME8(20 s) -> manual PT1(30 s) -> tower temp temp_a
                               (dead time + first-order lag = FOPDT plant simulation)
    temp_a / temp_b / bad point -> FB_MID3 (two good one bad, take the average) -> FB_LIMIT -> voted tower temp
    voted tower temp          -> FB_DERIV -> rate of change degC/s;
                               -> FB_STATS (full-run statistics) + FB_AVG_TIME (60 s tumbling window)
    tower-temp master FB_PID -> FB_CASCADE -> steam-flow slave FB_PID -> steam_valve
                               (when not cascaded the master tracks via track, CAS transfer bumpless)
    master PID deviation dev  -> FB_INTEG (interlock hold, clamped +/-1000) -> deviation integral degC*s
    -- the following demonstrate the 0.3.0 loop/device/signal extension blocks (sections 17~26) --
    voted tower temp + q_fail -> FB_PV_HOLD (bad value held 5 s first then hold/substitute) -> temp_held
    heat-up/soak 4-segment curve -> FB_RAMPSOAK8 -> rs_sp (program setpoint)
    temp_held                -> FB_GAIN_SCHED3 (three-zone parameters + boundary transition)
                               -> FB_PID_POS (program SP + scheduled parameters, observable p/i/d)
                               -> FB_MANSTATION (manual station, balance bumpless) -> heat_out
    temp_held / rs_sp        -> FB_PID_3STEP (motorized-actuator pulse control, for observation)
    agitator fwd/rev buttons  -> FB_MOTOR_REV (reversal interlock 2 s, feedback echo simulation)
    fill_open                -> FB_MOV (motorized make-up valve, 30 s simulated travel, dual limits)
    dose start               -> FB_DOSE (coarse 8 / fine 1.5 units/s simulated flow)
    gas_flow + operating p/t  -> FB_FLOW_COMP (square-root DP pressure/temperature compensation) -> gas_flow_std
  scan_s must match the tasks.toml interval_ms (default 100 ms = 0.1 s).
============================================================================ *)

(* ---------------------------------------------------------------------------
   PROGRAM demo_main — wiring layer
--------------------------------------------------------------------------- *)
PROGRAM demo_main
  VAR
    (* -- field signals (bound via iomap; given initial values for the demo) -- *)
    lt0101_raw    : REAL := 13824.0; (* level transmitter raw counts (4-20mA -> 5530..27648) *)
    level_sp      : REAL := 60.0;    (* level setpoint % *)
    alm_ack       : BOOL := FALSE;   (* alarm acknowledge button *)
    hammer_enable : BOOL := TRUE;    (* air cannon in service *)
    gas_flow      : REAL := 1200.0;  (* inlet gas flow m3/h (leading flow FT0201) *)
    milk_flow     : REAL := 950.0;   (* lime-milk flow m3/h (follower pv, FT0202) *)
    milk_ratio    : REAL := 0.85;    (* lime-milk / inlet-gas ratio *)
    pump_run_fb_a : BOOL := FALSE;   (* lime-milk pump 1 run feedback *)
    pump_run_fb_b : BOOL := FALSE;   (* lime-milk pump 2 run feedback *)
    pump_fault_a  : BOOL := FALSE;   (* pump 1 fault (protection relay / thermal overload) *)
    pump_fault_b  : BOOL := FALSE;   (* pump 2 fault *)
    temp_hh       : BOOL := FALSE;   (* tower temperature high-high (external alarm bit) *)
    ilk_reset     : BOOL := FALSE;   (* interlock reset button *)
    rt_reset      : BOOL := FALSE;   (* run-hours clear (after maintenance) *)

    scan_s : REAL := 0.1; (* sample period s, must match tasks.toml interval_ms *)

    (* -- FB instances -- *)
    scale_lt      : FB_SCALE;
    pid_level     : FB_PID;
    alm_level     : FB_ALARM_HL;
    pulser_hammer : FB_PULSER;
    fill_ctl      : FB_HILO_FILL;
    ilk_tower     : FB_INTERLOCK8;
    rt_pump_a     : FB_RUNTIME;
    rt_pump_b     : FB_RUNTIME;
    duty_pumps    : FB_DUTY2;
    ratio_milk    : FB_RATIO;
    pid_milk      : FB_PID;
    split_milk    : FB_SPLIT_RANGE;

    (* -- output / intermediate variables (bound to AO/DO via iomap) -- *)
    level_pct   : REAL; (* tower level % *)
    lt_fault    : BOOL; (* transmitter open-circuit *)
    valve_pct   : REAL; (* feed control-valve opening % *)
    lv_any_alm  : BOOL; (* any level alarm *)
    hammer_q    : BOOL; (* air-cannon solenoid *)
    fill_open   : BOOL; (* make-up demand (high-close low-open) *)
    ilk_trip    : BOOL; (* tower interlock acted *)
    ilk_first   : INT;  (* interlock first-out channel *)
    pump_cmd_a  : BOOL; (* pump 1 start command *)
    pump_cmd_b  : BOOL; (* pump 2 start command *)
    duty_a      : BOOL; (* current duty pump is pump 1 *)
    milk_sp     : REAL; (* lime-milk flow setpoint after ratioing m3/h *)
    milk_dev    : REAL; (* lime-milk loop working deviation (after the SP ramp) *)
    valve_small : REAL; (* lime-milk small-valve opening % (split segment A, fine trim) *)
    valve_big   : REAL; (* lime-milk large-valve opening % (split segment B, range extension) *)

    (* -- math/dynamics block demo: steam-heating cascade + FOPDT plant simulation (sections 12~16) -- *)
    cas_enable     : BOOL := TRUE;  (* cascade-enable switch *)
    temp_sp        : REAL := 75.0;  (* tower-temperature setpoint degC (cascade master SP) *)
    steam_local_sp : REAL := 800.0; (* operator-given steam-flow local SP kg/h when not cascaded *)
    stats_reset    : BOOL := FALSE; (* statistics / deviation-integral clear button *)

    dly_obj      : FB_DEADTIME8; (* plant dead time 20 s *)
    mid_temp     : FB_MID3;      (* tower-temperature median of three (two good one bad) *)
    lim_temp     : FB_LIMIT;     (* voted tower-temperature limit 0..150 degC *)
    deriv_temp   : FB_DERIV;     (* tower-temperature rate of change *)
    casc_temp    : FB_CASCADE;   (* cascade coupling: master output -> steam SP *)
    pid_tc       : FB_PID;       (* cascade master loop: tower temperature *)
    pid_fc_steam : FB_PID;       (* cascade slave loop: steam flow *)
    integ_dev    : FB_INTEG;     (* master-loop deviation integral *)
    stats_temp   : FB_STATS;     (* tower-temperature full-run statistics *)
    avgt_temp    : FB_AVG_TIME;  (* tower-temperature 60 s tumbling-window average *)

    sim_state     : REAL := 40.0; (* FOPDT plant first-order state (manual PT1 discretization, T = 30 s) *)
    temp_a        : REAL;         (* tower-temperature transmitter 1 degC (plant-simulation output) *)
    temp_b        : REAL;         (* redundant point 2 degC (simulated +0.5 degC bias) *)
    temp_voted    : REAL;         (* tower temperature after voting + limiting degC *)
    temp_roc      : REAL;         (* tower-temperature rate of change degC/s *)
    steam_sp_used : REAL;         (* steam slave-loop working SP kg/h (fed back to slave_sp_now) *)
    steam_flow    : REAL;         (* steam flow pv kg/h (simulated static valve characteristic) *)
    steam_valve   : REAL;         (* steam-valve opening % (slave-loop output) *)
    temp_dev_int  : REAL;         (* tower-temperature deviation integral degC*s *)

    (* -- 0.3.0 extension block demo: program setpoint + positional PID chain, device layer,
          compensation (sections 17~26) -- *)
    rs_run       : BOOL := TRUE;  (* program setpoint run *)
    rs_hold      : BOOL := FALSE; (* program setpoint pause *)
    rs_advance   : BOOL := FALSE; (* program setpoint jump-segment button *)
    batch_reset  : BOOL := FALSE; (* program / batch reset button *)
    msta_auto    : BOOL := TRUE;  (* manual station auto in service *)
    msta_man_out : REAL := 0.0;   (* manual station manual output % *)
    agit_fwd_btn : BOOL := FALSE; (* agitator forward button *)
    agit_rev_btn : BOOL := FALSE; (* agitator reverse button *)
    dose_start   : BOOL := FALSE; (* dose start button *)
    dose_abort   : BOOL := FALSE; (* dose abort button *)

    hold_temp  : FB_PV_HOLD;     (* voted tower-temperature bad-value hold *)
    rs_temp    : FB_RAMPSOAK8;   (* heat-up/soak program setpoint *)
    gsch_temp  : FB_GAIN_SCHED3; (* three-zone gain scheduling *)
    pidp_temp  : FB_PID_POS;     (* positional tower-temperature PID (shadow loop) *)
    msta_temp  : FB_MANSTATION;  (* heating-output manual station *)
    pid3s_temp : FB_PID_3STEP;   (* step PID (motorized actuator, for observation) *)
    mrev_agit  : FB_MOTOR_REV;   (* bidirectional agitator motor *)
    mov_fill   : FB_MOV;         (* motorized make-up valve *)
    dose_lime  : FB_DOSE;        (* limestone batch dosing *)
    comp_gas   : FB_FLOW_COMP;   (* inlet-gas pressure/temperature compensation *)

    temp_held    : REAL;          (* tower temperature after bad-value hold degC *)
    rs_sp        : REAL;          (* program setpoint SP degC *)
    heat_out_pos : REAL;          (* positional PID output % *)
    heat_out     : REAL;          (* final heating output after the manual station % *)
    agit_fb_f    : BOOL;          (* agitator forward feedback (output-echo simulation) *)
    agit_fb_r    : BOOL;          (* agitator reverse feedback *)
    mov_sim_pos  : REAL := 0.0;   (* motorized-valve simulated travel 0..100 % *)
    mov_zso      : BOOL := FALSE; (* motorized-valve open limit (simulated) *)
    mov_zsc      : BOOL := TRUE;  (* motorized-valve close limit (simulated, initially fully closed) *)
    mov_q_open   : BOOL;          (* motorized-valve open output (local copy, gotcha 8) *)
    mov_q_close  : BOOL;          (* motorized-valve close output *)
    dose_flow    : REAL := 0.0;   (* dose simulated flow kg/s *)
    dose_q_coar  : BOOL;          (* coarse-feed valve (local copy, gotcha 8) *)
    dose_q_fine  : BOOL;          (* fine-feed valve *)
    gas_flow_std : REAL;          (* standard-condition inlet-gas flow after compensation m3/h *)
  END_VAR

  (* 1) Span conversion: raw counts -> level % *)
  scale_lt(raw := lt0101_raw, raw_min := 5530.0, raw_max := 27648.0,
           out_min := 0.0, out_max := 100.0, clamp := TRUE);
  level_pct := scale_lt.out;
  lt_fault  := scale_lt.fault;

  (* 2) Four-level level alarm, 2% deadband, 3 s delay *)
  alm_level(pv := level_pct, h_sp := 80.0, l_sp := 20.0,
            hh_sp := 90.0, ll_sp := 10.0,
            deadband := 2.0, delay_s := 3.0, dt_s := scan_s, ack := alm_ack);
  lv_any_alm := alm_level.any_alm;

  (* 3) Level PID (reverse-acting: high level -> close the feed valve); on transmitter
        open-circuit, switch to manual and drop to 0. Note: kept as a legacy-signature
        call -- verifies the backward compatibility after the FB_PID upgrade *)
  pid_level(pv := level_pct, sp := level_sp,
            kp := 2.0, ti_s := 120.0, td_s := 0.0, dt_s := scan_s,
            direct := FALSE, auto := NOT lt_fault, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0);
  valve_pct := pid_level.out;

  (* 4) Air cannon: a 2 s purge every 300 s *)
  pulser_hammer(enable := hammer_enable, period_s := 300.0,
                pulse_len_s := 2.0, dt_s := scan_s);
  hammer_q := pulser_hammer.q;

  (* 5) Make-up demand: below 15% needs water, above 85% stops *)
  fill_ctl(pv := level_pct, h_sp := 85.0, l_sp := 15.0);
  fill_open := fill_ctl.valve_open;

  (* 6) Tower interlock summary + first-out: 1=level HH 2=transmitter open-circuit
        3=tower-temp HH 4=dual-pump fault; 5..8 spare, bypassed *)
  ilk_tower(c1 := alm_level.hh_alm,
            c2 := lt_fault,
            c3 := temp_hh,
            c4 := pump_fault_a AND pump_fault_b,
            enable5 := FALSE, enable6 := FALSE,
            enable7 := FALSE, enable8 := FALSE,
            reset := ilk_reset);
  ilk_trip  := ilk_tower.tripped;
  ilk_first := ilk_tower.first_out;

  (* 7) Dual-pump run statistics (hours feed FB_DUTY2 hour balancing; instances may go in VAR RETAIN) *)
  rt_pump_a(running := pump_run_fb_a, reset := rt_reset, dt_s := scan_s);
  rt_pump_b(running := pump_run_fb_b, reset := rt_reset, dt_s := scan_s);

  (* 8) Dual make-up pumps duty/standby: select duty by hour balancing; on interlock action, withdraw demand and stop all *)
  duty_pumps(demand := fill_open AND NOT ilk_trip, alt_mode := 1,
             run_fb_a := pump_run_fb_a, run_fb_b := pump_run_fb_b,
             fault_a := pump_fault_a, fault_b := pump_fault_b,
             hours_a := rt_pump_a.hours, hours_b := rt_pump_b.hours,
             dt_s := scan_s);
  pump_cmd_a := duty_pumps.cmd_a;
  pump_cmd_b := duty_pumps.cmd_b;
  duty_a     := duty_pumps.duty_is_a;

  (* 9) Ratio station: lime-milk flow setpoint = inlet-gas flow x 0.85 (ratio clamped 0.5..1.2) *)
  ratio_milk(wild_flow := gas_flow, ratio := milk_ratio,
             ratio_min := 0.5, ratio_max := 1.2, bias := 0.0);
  milk_sp := ratio_milk.sp;

  (* 10) Lime-milk flow PID: inlet-gas static feedforward (gain 0.01 -> % opening),
         SP ramp 50 (m3/h)/s to avoid a ratio step, interlock action forces the valve
         closed via track *)
  pid_milk(pv := milk_flow, sp := milk_sp,
           kp := 0.8, ti_s := 30.0, td_s := 0.0, dt_s := scan_s,
           direct := FALSE, auto := TRUE, manual_out := 0.0,
           out_min := 0.0, out_max := 100.0,
           ff := gas_flow * 0.01,
           track := ilk_trip, track_value := 0.0,
           sp_rate := 50.0);
  milk_dev := pid_milk.dev;

  (* 11) Split range: PID output 0..50% drives the small valve for fine trim, 50..100%
         the large valve relays (sequential split range, no reversal) *)
  split_milk(u := pid_milk.out, split := 50.0, reverse_b := FALSE);
  valve_small := split_milk.out_a;
  valve_big   := split_milk.out_b;

  (* 12) FOPDT plant simulation: steam-valve opening (previous scan) -> FB_DEADTIME8 dead
         time 20 s -> manual PT1 (T = 30 s, same core as FB_LAG) -> tower temperature.
         Dead time + first-order lag, these two blocks form the most common
         simulation/tuning plant *)
  dly_obj(u := steam_valve, t_delay_s := 20.0, dt_s := scan_s);
  sim_state := sim_state + scan_s / (30.0 + scan_s) * (dly_obj.out - sim_state);
  temp_a := 40.0 + 0.5 * sim_state; (* 0..100 % valve opening -> 40..90 degC *)
  temp_b := temp_a + 0.5;           (* redundant point 2: demo +0.5 degC bias *)

  (* 13) Median of three: point 3's bad value is excluded (good_c := FALSE) -> two good,
         take the average; good channels differing by > 5 degC raise q_dev *)
  mid_temp(a := temp_a, b := temp_b, c := 0.0,
           good_a := TRUE, good_b := TRUE, good_c := FALSE,
           dev_max := 5.0);

  (* 14) Limit the voted value 0..150 degC + filtered derivative (rate of change degC/s for trend/feedforward) *)
  lim_temp(u := mid_temp.out, lo := 0.0, hi := 150.0);
  temp_voted := lim_temp.out;
  deriv_temp(u := temp_voted, t_filt_s := 2.0, dt_s := scan_s);
  temp_roc := deriv_temp.out;

  (* 15) Cascade: tower-temperature master PID (0..100 %) -> FB_CASCADE maps to steam-flow
         SP (0..2000 kg/h) -> steam-flow slave PID -> steam valve. When not cascaded
         (cas_enable = FALSE) the slave uses its local SP and the master tracks the
         equivalent percentage via track, so CAS transfer is bumpless. slave_sp_now is
         fed the previous scan's working SP (standard wiring) *)
  casc_temp(master_out := pid_tc.out, cas_on := cas_enable,
            sp_min := 0.0, sp_max := 2000.0,
            slave_sp_local := steam_local_sp, slave_sp_now := steam_sp_used);
  steam_sp_used := casc_temp.slave_sp;
  pid_tc(pv := temp_voted, sp := temp_sp,
         kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
         direct := FALSE, auto := TRUE, manual_out := 0.0,
         out_min := 0.0, out_max := 100.0,
         track := casc_temp.master_track,
         track_value := casc_temp.master_track_value);
  pid_fc_steam(pv := steam_flow, sp := steam_sp_used,
               kp := 0.5, ti_s := 10.0, td_s := 0.0, dt_s := scan_s,
               direct := FALSE, auto := TRUE, manual_out := 0.0,
               out_min := 0.0, out_max := 100.0);
  steam_valve := pid_fc_steam.out;
  steam_flow := steam_valve * 20.0; (* simulated static valve characteristic: 0..100 % -> 0..2000 kg/h *)

  (* 16) Shared-signal (voted tower temperature) statistics trio + deviation integral:
         full-run min/max/mean/standard deviation, 60 s tumbling-window average;
         master-loop deviation integral degC*s (hold on interlock action, clamped +/-1000) *)
  stats_temp(u := temp_voted, enable := TRUE, reset := stats_reset);
  avgt_temp(u := temp_voted, window_s := 60.0, dt_s := scan_s, reset := stats_reset);
  integ_dev(u := pid_tc.dev, k := 1.0, dt_s := scan_s,
            hold := ilk_trip, reset := stats_reset,
            preset := FALSE, preset_value := 0.0,
            out_min := -1000.0, out_max := 1000.0);
  temp_dev_int := integ_dev.out;

  (* 17) Bad-value hold: when the vote is all bad (q_fail), hold the last good value 5 s
         first, then keep holding (mode 0); q_bad/q_timeout feed alarming and loop-to-manual *)
  hold_temp(u := mid_temp.out, good := NOT mid_temp.q_fail,
            mode := 0.0, sub_value := 60.0, timeout_s := 5.0,
            dt_s := scan_s);
  temp_held := hold_temp.out;

  (* 18) 8-segment program setpoint (using 4 segments): a heat-up/soak/cool-down curve
         starting at 40 degC; hold pauses, advance jumps segment, batch_reset resets *)
  rs_temp(sp1 := 55.0, ramp1_s := 120.0, soak1_s := 60.0,
          sp2 := 70.0, ramp2_s := 180.0, soak2_s := 120.0,
          sp3 := 85.0, ramp3_s := 120.0, soak3_s := 300.0,
          sp4 := 40.0, ramp4_s := 240.0, soak4_s := 0.0,
          n_seg := 4.0, run := rs_run, hold := rs_hold,
          advance := rs_advance, reset := batch_reset,
          cyclic := FALSE, sp_init := 40.0, dt_s := scan_s);
  rs_sp := rs_temp.sp;

  (* 19) Three-zone gain scheduling: output PID parameters by zone of the held tower
         temperature (strong in the low-temp zone, weak in the high-temp zone), with a
         +/-2 degC linear transition at the boundaries (interp) *)
  gsch_temp(u_sched := temp_held, x12 := 55.0, x23 := 75.0,
            hyst := 2.0, interp := TRUE,
            kp1 := 2.0, ti1_s := 90.0, td1_s := 0.0,
            kp2 := 1.5, ti2_s := 60.0, td2_s := 0.0,
            kp3 := 1.0, ti3_s := 45.0, td3_s := 0.0);

  (* 20) Positional PID (shadow tower-temperature loop): program setpoint SP + bad-value-held
         PV + scheduled parameters, observable p/i/d; interlock forces 0 via track *)
  pidp_temp(pv := temp_held, sp := rs_sp,
            kp := gsch_temp.kp_out, ti_s := gsch_temp.ti_out_s,
            td_s := gsch_temp.td_out_s, t_dfilt_s := 2.0,
            sp_weight_b := 1.0, dt_s := scan_s,
            direct := FALSE, auto := TRUE, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0,
            ff := 0.0, track := ilk_trip, track_value := 0.0);
  heat_out_pos := pidp_temp.out;

  (* 21) Manual station: in auto pass through the positional PID output (rate-limited
         20 %/s), in manual set directly; balance gives a bumpless return to auto *)
  msta_temp(auto_in := heat_out_pos, auto := msta_auto,
            manual_out := msta_man_out, bias := 0.0, rate := 20.0,
            out_min := 0.0, out_max := 100.0, balance := TRUE,
            dt_s := scan_s);
  heat_out := msta_temp.out;

  (* 22) Step PID (for observation): the same loop driving a motorized actuator without
         position feedback, full travel 60 s, minimum pulse 0.5 s, deadband 1 %, travel estimate pos_est *)
  pid3s_temp(pv := temp_held, sp := rs_sp,
             kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
             motor_time_s := 60.0, min_pulse_s := 0.5,
             deadband_pct := 1.0, auto := TRUE,
             man_open := FALSE, man_close := FALSE, pos_fb := -1.0);

  (* 23) Bidirectional agitator motor: forward/reverse buttons, interlock stop has
         priority, reversal interlock 2 s; feedback uses output-echo simulation (one-scan
         lag, never times out) *)
  mrev_agit(cmd_fwd := agit_fwd_btn, cmd_rev := agit_rev_btn,
            stop := ilk_trip, fb_fwd := agit_fb_f, fb_rev := agit_fb_r,
            fb_timeout_s := 5.0, reversal_delay_s := 2.0,
            reset := ilk_reset, dt_s := scan_s);
  agit_fb_f := mrev_agit.q_fwd; (* simulated feedback: output echo (effective next scan) *)
  agit_fb_r := mrev_agit.q_rev;

  (* 24) Motorized make-up valve: make-up demand opens / withdrawal closes (interlock
         forces close), a 30 s simulated travel drives the dual limits; travel-timeout limit 60 s *)
  mov_fill(cmd_open := fill_open AND NOT ilk_trip,
           cmd_close := NOT fill_open OR ilk_trip,
           cmd_stop := FALSE, zso := mov_zso, zsc := mov_zsc,
           travel_s := 60.0, reset := ilk_reset, dt_s := scan_s);
  mov_q_open  := mov_fill.q_open;  (* copy to a local first, then use as an IF condition (gotcha 8) *)
  mov_q_close := mov_fill.q_close;
  IF mov_q_open THEN
    mov_sim_pos := mov_sim_pos + 100.0 * scan_s / 30.0; (* 30 s full-travel simulation *)
  ELSIF mov_q_close THEN
    mov_sim_pos := mov_sim_pos - 100.0 * scan_s / 30.0;
  END_IF;
  IF mov_sim_pos > 100.0 THEN
    mov_sim_pos := 100.0;
  ELSIF mov_sim_pos < 0.0 THEN
    mov_sim_pos := 0.0;
  END_IF;
  mov_zso := mov_sim_pos >= 100.0;
  mov_zsc := mov_sim_pos <= 0.0;

  (* 25) Batch dosing: target 100 kg, fine stage 15 kg, in-flight 2 kg; simulated flow
         coarse 8 / fine 1.5 kg/s (valve state echoed from the previous scan, with a one-scan lag) *)
  dose_lime(start := dose_start, abort := dose_abort,
            flow := dose_flow, target := 100.0,
            fine_qty := 15.0, inflight := 2.0,
            dt_s := scan_s, reset := batch_reset);
  dose_q_coar := dose_lime.q_coarse; (* copy to a local first, then use as an IF condition (gotcha 8) *)
  dose_q_fine := dose_lime.q_fine;
  dose_flow := 0.0;
  IF dose_q_coar THEN
    dose_flow := 8.0;
  ELSIF dose_q_fine THEN
    dose_flow := 1.5;
  END_IF;

  (* 26) Gas pressure/temperature compensation: correct the inlet-gas flow from operating
         150 kPa(g)/45 degC to design conditions (square-root DP, mode 0) -> standard-condition inlet-gas flow *)
  comp_gas(q_raw := gas_flow, p_gauge_kpa := 150.0, t_c := 45.0,
           p_ref_kpa_abs := 201.325, t_ref_c := 20.0,
           atm_kpa := 101.325, mode := 0.0);
  gas_flow_std := comp_gas.q_comp;
END_PROGRAM
