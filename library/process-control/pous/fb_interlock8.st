(*
  FB_INTERLOCK8 — 8-channel interlock summary + first-out recording.

  ~ Interlock summary + first-out capture (first-out annunciation, after the ISA-18.1 first-out-window concept).

  Purpose: trip-cause tracing for a unit interlock: summarize multiple conditions into a
  single trip bit and latch the channel that acted first, to locate the trip root cause
  afterward (consequential actions after the first-out do not overwrite the record).
  Behavior:
    Effective condition ax := cx AND enablex (enablex default TRUE; setting FALSE = bypass that channel).
    tripped := any effective condition OR (latched AND NOT reset)
    -- while any condition exists the alarm stays on; after all conditions clear it is
    held until reset (equivalent to "reset AND all clear" to reset; reset has no effect
    while a condition is uncleared).
    First-out capture: the scan where tripped goes FALSE->TRUE records the lowest channel
    number acting at that moment (on a tie, the lowest number). During the interlock hold,
    later actions do not overwrite the first-out; on a successful reset (tripped returns
    FALSE) first_out is cleared to 0.
    cx semantics: TRUE = that condition has acted (trip direction). Invert normally-closed
    signals externally.

  Inputs:
    c1..c8           BOOL  interlock conditions (TRUE = acted/over-limit)
    enable1..enable8 BOOL  per-channel in service (default TRUE; FALSE = bypass)
    reset            BOOL  reset (level-active; effective only after all conditions clear)
  Outputs:
    tripped   BOOL  interlock acted (latched)
    first_out INT   first-out channel number 1..8, 0 = none
*)
FUNCTION_BLOCK FB_INTERLOCK8
  VAR_INPUT
    c1      : BOOL := FALSE;
    c2      : BOOL := FALSE;
    c3      : BOOL := FALSE;
    c4      : BOOL := FALSE;
    c5      : BOOL := FALSE;
    c6      : BOOL := FALSE;
    c7      : BOOL := FALSE;
    c8      : BOOL := FALSE;
    enable1 : BOOL := TRUE;
    enable2 : BOOL := TRUE;
    enable3 : BOOL := TRUE;
    enable4 : BOOL := TRUE;
    enable5 : BOOL := TRUE;
    enable6 : BOOL := TRUE;
    enable7 : BOOL := TRUE;
    enable8 : BOOL := TRUE;
    reset   : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    tripped   : BOOL;
    first_out : INT;
  END_VAR
  VAR
    a1    : BOOL; (* effective condition after in-service gating *)
    a2    : BOOL;
    a3    : BOOL;
    a4    : BOOL;
    a5    : BOOL;
    a6    : BOOL;
    a7    : BOOL;
    a8    : BOOL;
    any_c : BOOL;
    was   : BOOL; (* previous-scan tripped, to capture the first-out edge *)
  END_VAR

  a1 := c1 AND enable1;
  a2 := c2 AND enable2;
  a3 := c3 AND enable3;
  a4 := c4 AND enable4;
  a5 := c5 AND enable5;
  a6 := c6 AND enable6;
  a7 := c7 AND enable7;
  a8 := c8 AND enable8;
  any_c := a1 OR a2 OR a3 OR a4 OR a5 OR a6 OR a7 OR a8;

  was := tripped;
  tripped := any_c OR (tripped AND NOT reset);

  IF tripped AND NOT was THEN
    (* New trip event: record the first-out (on a tie, the lowest number) *)
    IF a1 THEN
      first_out := 1;
    ELSIF a2 THEN
      first_out := 2;
    ELSIF a3 THEN
      first_out := 3;
    ELSIF a4 THEN
      first_out := 4;
    ELSIF a5 THEN
      first_out := 5;
    ELSIF a6 THEN
      first_out := 6;
    ELSIF a7 THEN
      first_out := 7;
    ELSIF a8 THEN
      first_out := 8;
    END_IF;
  ELSIF NOT tripped THEN
    first_out := 0; (* successful reset clears the first-out *)
  END_IF;
END_FUNCTION_BLOCK
