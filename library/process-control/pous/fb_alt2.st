(*
  FB_ALT2 — dual-valve (dual-device) timed rotator, with two external permissives.

  Purpose: two diverter valves / two pumps rotating on a fixed period for wear balancing.
  Example: dual-permissive diverting with an iron remover + a packing scale:
  permissive1 (iron remover running) or permissive2 (packing scale ready) lost ->
  both valves close at once; after the permissives recover, rotation continues by its
  timer (the rotation selection is held, the timer is cleared and restarts).

  Behavior:
    ok := enable AND permissive1 AND permissive2.
    NOT ok: q1 := FALSE, q2 := FALSE, rotation timer cleared (the selection bit is held,
    and after recovery continues from the last selected channel).
    ok: the timer accumulates by dt_s manually, and at swap_period_s the selection
    switches and the timer clears; q1/q2 output the currently selected channel mutually
    exclusively. swap_period_s <= 0 means no rotation (hold the current channel).

  Inputs:
    enable        BOOL  enable
    swap_period_s REAL  rotation period s
    dt_s          REAL  sample period s
    permissive1   BOOL  external permissive 1 (e.g. iron remover running)
    permissive2   BOOL  external permissive 2 (e.g. packing scale ready)
  Outputs:
    q1 BOOL  valve 1 output
    q2 BOOL  valve 2 output (mutually exclusive with q1; both FALSE when a permissive is lost)
*)
FUNCTION_BLOCK FB_ALT2
  VAR_INPUT
    enable        : BOOL;
    swap_period_s : REAL := 300.0;
    dt_s          : REAL := 0.1;
    permissive1   : BOOL := TRUE;
    permissive2   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    q1 : BOOL;
    q2 : BOOL;
  END_VAR
  VAR
    ok    : BOOL;
    sel2  : BOOL; (* FALSE selects valve 1 / TRUE selects valve 2, held while disabled *)
    run_t : REAL; (* this channel's elapsed run time s, manual timing *)
  END_VAR

  ok := enable AND permissive1 AND permissive2;

  IF NOT ok THEN
    (* Any permissive lost: both valves close, timer cleared, selection held *)
    q1    := FALSE;
    q2    := FALSE;
    run_t := 0.0;
    RETURN;
  END_IF;

  IF dt_s > 0.0 THEN
    run_t := run_t + dt_s;
  END_IF;
  IF swap_period_s > 0.0 AND run_t >= swap_period_s THEN
    run_t := 0.0;
    sel2  := NOT sel2;
  END_IF;

  q1 := NOT sel2;
  q2 := sel2;
END_FUNCTION_BLOCK
