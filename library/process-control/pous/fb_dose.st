(*
  FB_DOSE — batch dosing / batch feeding (coarse/fine two stages + in-flight compensation).

  ~ Standard dosing block (dosing / batch weighing, coarse-fine two-speed feeding +
  early in-flight cutoff), the customary block for batch-scale / batch-feed control.

  Purpose: dosing material from a silo into a weigh hopper / reactor: coarse feed
  approaches the target quickly, fine feed trims, the fine valve closes early by the
  in-flight amount (the material still in flight after the valve closes), and after
  settling the completion and overshoot are reported. The flow input may come from a
  weight derivative or a flowmeter.

  Behavior:
    start rising edge (when idle, not complete, and abort not pressed) begins a new
    batch: qty cleared, enter the feed stage. During feeding/settling, accumulate
    qty := qty + flow * dt_s (flow <= 0 not counted, one of the criteria for material
    fully landed).
      Coarse stage: qty < target - fine_qty -> q_coarse = TRUE (both coarse and fine valves open);
      Fine stage: qty < target - inflight -> q_fine = TRUE; at the point, close the fine
      valve (the in-flight reserve inflight is the in-flight amount held back) and enter
      the settling stage.
      Settling stage: wait for flow <= 0 or a fixed 2 s (whichever first) -> done := TRUE,
      overshoot := qty - target (the actual overshoot once the in-flight material has landed).
    abort level-active: stop both valves immediately, exit feeding/settling, qty
    retained (for stocktaking), done not set.
    After done, start has no effect; reset (level-active) is required to clear
    qty/done/overshoot and begin the next batch.
    Assumes fine_qty >= inflight >= 0 (coarse valve closes first, fine valve closes after);
    dribble after done is no longer counted into qty.
    dt_s <= 0 -> hold this scan, no action (command edges still remembered).

  Inputs:
    start    BOOL  start (rising edge begins a new batch)
    abort    BOOL  abort (level-active: stop valves, retain quantity)
    flow     REAL  instantaneous flow units/s (weight derivative or flowmeter)
    target   REAL  target quantity (same unit as the flow integral)
    fine_qty REAL  fine-feed-stage quantity (close the coarse valve with this much remaining)
    inflight REAL  in-flight amount (close the fine valve with this much remaining)
    dt_s     REAL  sample period s
    reset    BOOL  batch reset (level-active: clear qty/done/overshoot)
  Outputs:
    q_coarse  BOOL  coarse-feed valve
    q_fine    BOOL  fine-feed valve
    qty       REAL  this batch's accumulated quantity
    done      BOOL  this batch complete (set after settling, held until reset)
    overshoot REAL  actual - target after completion (basis for tuning the in-flight amount)
*)
FUNCTION_BLOCK FB_DOSE
  VAR_INPUT
    start    : BOOL;
    abort    : BOOL;
    flow     : REAL;
    target   : REAL := 100.0;
    fine_qty : REAL := 10.0;
    inflight : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL;
  END_VAR
  VAR_OUTPUT
    q_coarse  : BOOL;
    q_fine    : BOOL;
    qty       : REAL;
    done      : BOOL;
    overshoot : REAL;
  END_VAR
  VAR
    dosing     : BOOL; (* feed stage (coarse/fine) *)
    settling   : BOOL; (* settling-wait stage *)
    settle_t   : REAL; (* settling timer s *)
    prev_start : BOOL; (* start rising-edge memory *)
  END_VAR

  IF reset THEN
    dosing     := FALSE;
    settling   := FALSE;
    settle_t   := 0.0;
    qty        := 0.0;
    done       := FALSE;
    overshoot  := 0.0;
    q_coarse   := FALSE;
    q_fine     := FALSE;
    prev_start := start;
    RETURN;
  END_IF;

  IF abort THEN
    (* Abort: stop valves and exit the stage, qty retained, done not set *)
    dosing   := FALSE;
    settling := FALSE;
    q_coarse := FALSE;
    q_fine   := FALSE;
  END_IF;

  (* start rising edge, idle and not complete -> begin a new batch *)
  IF start AND NOT prev_start AND NOT abort
     AND NOT dosing AND NOT settling AND NOT done THEN
    dosing    := TRUE;
    qty       := 0.0;
    overshoot := 0.0;
    settle_t  := 0.0;
  END_IF;
  prev_start := start;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  (* Accumulate flow during feeding/settling (negative flow not counted) *)
  IF (dosing OR settling) AND flow > 0.0 THEN
    qty := qty + flow * dt_s;
  END_IF;

  IF dosing THEN
    q_coarse := qty < target - fine_qty;
    q_fine   := qty < target - inflight;
    IF NOT q_fine THEN
      (* At the in-flight reserve point: close the fine valve and enter the settling stage *)
      q_coarse := FALSE;
      dosing   := FALSE;
      settling := TRUE;
      settle_t := 0.0;
    END_IF;
  END_IF;

  IF settling THEN
    settle_t := settle_t + dt_s;
    IF flow <= 0.0 OR settle_t >= 2.0 THEN
      settling  := FALSE;
      done      := TRUE;
      overshoot := qty - target;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
