(*
  FB_RAMPSOAK8 — 8-segment setpoint programmer (ramp-soak setpoint programmer).

  ~ Standard program-setpoint / curve-setpoint block (ramp-soak / setpoint programmer:
  heat-up curves, heat-treatment curves, batch temperature programs), found in every
  major DCS/PLC and program controller.

  Purpose: generate a setpoint from a segmented "ramp to target + soak" curve, feeding
  the sp input of FB_PID / FB_PID_POS: heat-treatment/curing/reactor heat-up programs,
  startup/shutdown load ramp curves. Segment parameters are scalar-expanded sp1..sp8 /
  ramp1_s..ramp8_s / soak1_s..soak8_s (in this dialect an FB-scope array cannot be
  codegen'd, see README gotcha 7; same as FB_CHAR).

  Behavior:
    Each segment N: sp ramps linearly from the previous segment's level (segment 1 from
    sp_init) over rampN_s to spN (rampN_s <= 0 steps), then soaks for soakN_s
    (soaking = TRUE; soakN_s <= 0 passes immediately). Segment parameters latch on the
    segment-entry scan -- changing parameters while running takes effect on the next segment.
    run level-active: TRUE starts and advances the program; FALSE -> return to the start
    (seg := 0, sp := sp_init, done cleared), and setting TRUE again restarts from segment 1.
    hold level-active: freeze sp and the in-segment timer (advance has no effect during hold).
    advance rising edge: jump to the next segment immediately (continue ramping from the
    current sp, curve continuous); advance in the last segment -> done (or cyclic wrap).
    On the last segment's soak end (n_seg clamped 2..8):
      cyclic = FALSE -> done := TRUE, sp pinned at the last segment's target, seg := 0,
      held until reset or run is withdrawn;
      cyclic = TRUE  -> wrap to segment 1 (continue ramping from the current sp, long-term cycling).
    reset level-active (priority): return to start, same as withdrawing run.
    dt_s <= 0 -> no in-segment advance (start/advance edges still handled).

  Inputs:
    sp1..sp8       REAL  per-segment target values (scalar-expanded)
    ramp1_s..ramp8_s REAL  per-segment ramp times s (<= 0 = step)
    soak1_s..soak8_s REAL  per-segment soak times s (<= 0 = no soak)
    n_seg          REAL  number of segments in use 2..8 (auto-clamped)
    run            BOOL  run (level; withdrawing returns to start)
    hold           BOOL  pause (level; freeze sp and the timer)
    advance        BOOL  jump segment (rising edge)
    reset          BOOL  reset (level, priority)
    cyclic         BOOL  TRUE = wrap to segment 1 and cycle after the last segment completes
    sp_init        REAL  start value (the launch level of the segment-1 ramp)
    dt_s           REAL  sample period s
  Outputs:
    sp      REAL  program setpoint output
    seg     REAL  current segment 1..8 (0 = not running / completed)
    soaking BOOL  soak segment in progress
    done    BOOL  program complete (non-cyclic; held until reset / run withdrawn)
*)
FUNCTION_BLOCK FB_RAMPSOAK8
  VAR_INPUT
    sp1     : REAL := 0.0;
    sp2     : REAL := 0.0;
    sp3     : REAL := 0.0;
    sp4     : REAL := 0.0;
    sp5     : REAL := 0.0;
    sp6     : REAL := 0.0;
    sp7     : REAL := 0.0;
    sp8     : REAL := 0.0;
    ramp1_s : REAL := 0.0;
    ramp2_s : REAL := 0.0;
    ramp3_s : REAL := 0.0;
    ramp4_s : REAL := 0.0;
    ramp5_s : REAL := 0.0;
    ramp6_s : REAL := 0.0;
    ramp7_s : REAL := 0.0;
    ramp8_s : REAL := 0.0;
    soak1_s : REAL := 0.0;
    soak2_s : REAL := 0.0;
    soak3_s : REAL := 0.0;
    soak4_s : REAL := 0.0;
    soak5_s : REAL := 0.0;
    soak6_s : REAL := 0.0;
    soak7_s : REAL := 0.0;
    soak8_s : REAL := 0.0;
    n_seg   : REAL := 2.0;
    run     : BOOL := FALSE;
    hold    : BOOL := FALSE;
    advance : BOOL := FALSE;
    reset   : BOOL := FALSE;
    cyclic  : BOOL := FALSE;
    sp_init : REAL := 0.0;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    sp      : REAL;
    seg     : REAL;
    soaking : BOOL;
    done    : BOOL;
  END_VAR
  VAR
    nn         : REAL; (* clamped segment count 2..8 *)
    elapsed_t  : REAL; (* in-segment phase timer s (ramp/soak each start from 0) *)
    level_from : REAL; (* this segment's ramp launch level *)
    seg_sp     : REAL; (* segment parameters latched on entry *)
    seg_ramp_s : REAL;
    seg_soak_s : REAL;
    enter_seg  : REAL; (* segment number to enter this scan (0 = no entry) *)
    go_next    : BOOL; (* segment complete / jump this scan *)
    adv_edge   : BOOL;
    prev_adv   : BOOL; (* advance rising-edge memory *)
  END_VAR

  IF reset THEN
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    elapsed_t := 0.0;
    prev_adv := advance;
    RETURN;
  END_IF;

  adv_edge := advance AND NOT prev_adv;
  prev_adv := advance;

  IF NOT run THEN
    (* Stopped: return to start; re-running starts from segment 1 *)
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    RETURN;
  END_IF;

  IF done THEN
    RETURN; (* Completion hold: sp pinned at the last segment's target, awaiting reset / run withdrawal *)
  END_IF;

  nn := n_seg;
  IF nn < 2.0 THEN
    nn := 2.0;
  ELSIF nn > 8.0 THEN
    nn := 8.0;
  END_IF;

  enter_seg := 0.0;
  go_next := FALSE;

  IF seg < 0.5 THEN
    (* Start: enter segment 1 from sp_init *)
    sp := sp_init;
    enter_seg := 1.0;
  ELSIF hold THEN
    RETURN; (* Freeze: sp and timer held (advance has no effect during hold) *)
  ELSIF adv_edge THEN
    go_next := TRUE; (* Jump segment: continue from the current sp *)
  ELSE
    IF dt_s <= 0.0 THEN
      RETURN; (* invalid sample period: no in-segment advance *)
    END_IF;
    elapsed_t := elapsed_t + dt_s;
    IF NOT soaking THEN
      (* Ramp phase *)
      IF seg_ramp_s <= 0.0 THEN
        sp := seg_sp; (* step segment *)
        soaking := TRUE;
        elapsed_t := 0.0;
      ELSIF elapsed_t >= seg_ramp_s THEN
        sp := seg_sp;
        soaking := TRUE;
        elapsed_t := elapsed_t - seg_ramp_s; (* carry the cross-phase remainder forward *)
      ELSE
        sp := level_from + (seg_sp - level_from) * elapsed_t / seg_ramp_s;
      END_IF;
    END_IF;
    IF soaking AND elapsed_t >= seg_soak_s THEN
      go_next := TRUE; (* soak ended *)
    END_IF;
  END_IF;

  (* Segment-advance decision: last segment -> complete/wrap; otherwise enter the next segment *)
  IF go_next THEN
    IF seg >= nn - 0.5 THEN
      IF cyclic THEN
        enter_seg := 1.0; (* Wrap: segment 1 continues ramping from the current sp *)
      ELSE
        done := TRUE;
        sp := seg_sp; (* Complete: pin at the last segment's target (an early jump also lands here directly) *)
        seg := 0.0;
        soaking := FALSE;
      END_IF;
    ELSE
      enter_seg := seg + 1.0;
    END_IF;
  END_IF;

  (* Segment entry: latch the segment parameters, ramp launches from the current sp *)
  IF enter_seg > 0.5 THEN
    seg := enter_seg;
    level_from := sp;
    elapsed_t := 0.0;
    soaking := FALSE;
    IF seg <= 1.5 THEN
      seg_sp := sp1;
      seg_ramp_s := ramp1_s;
      seg_soak_s := soak1_s;
    ELSIF seg <= 2.5 THEN
      seg_sp := sp2;
      seg_ramp_s := ramp2_s;
      seg_soak_s := soak2_s;
    ELSIF seg <= 3.5 THEN
      seg_sp := sp3;
      seg_ramp_s := ramp3_s;
      seg_soak_s := soak3_s;
    ELSIF seg <= 4.5 THEN
      seg_sp := sp4;
      seg_ramp_s := ramp4_s;
      seg_soak_s := soak4_s;
    ELSIF seg <= 5.5 THEN
      seg_sp := sp5;
      seg_ramp_s := ramp5_s;
      seg_soak_s := soak5_s;
    ELSIF seg <= 6.5 THEN
      seg_sp := sp6;
      seg_ramp_s := ramp6_s;
      seg_soak_s := soak6_s;
    ELSIF seg <= 7.5 THEN
      seg_sp := sp7;
      seg_ramp_s := ramp7_s;
      seg_soak_s := soak7_s;
    ELSE
      seg_sp := sp8;
      seg_ramp_s := ramp8_s;
      seg_soak_s := soak8_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
