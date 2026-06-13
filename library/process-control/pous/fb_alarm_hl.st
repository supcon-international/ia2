(*
  FB_ALARM_HL — four-level high/low alarm (H/L/HH/LL) with deadband, delay and simplified ISA-18.2 acknowledge latch.

  Purpose: high/low/high-high/low-low alarms for analog values such as level/temperature/pressure.
  Example: a vessel level LT alarm, with HH interlocking the feed shutdown.

  Behavior (same for each level, independently timed):
    1) Raw limit decision (with deadband hysteresis):
       High type: pv > sp sets; pv < sp - deadband resets.
       Low type:  pv < sp sets; pv > sp + deadband resets.
    2) Delay: the limit condition must persist for delay_s before tripping (internally
       timed by manual dt_s accumulation, not a TON instance, for easy offline testing
       and portability).
    3) Latch (simplified ISA-18.2): alarm output = trip condition OR (latched AND NOT ack).
       That is, while the condition exists the alarm stays on; after the condition
       clears it is held until ack acknowledges and clears it.

  Inputs:
    pv        REAL  process value
    h_sp      REAL  high alarm setpoint
    l_sp      REAL  low alarm setpoint
    hh_sp     REAL  high-high alarm setpoint
    ll_sp     REAL  low-low alarm setpoint
    deadband  REAL  deadband (hysteresis, engineering units)
    delay_s   REAL  alarm delay s (0 = immediate)
    dt_s      REAL  sample period s
    ack       BOOL  acknowledge (level-active)
  Outputs:
    h_alm / l_alm / hh_alm / ll_alm  BOOL  latched alarm outputs
    any_alm                          BOOL  any alarm
    h_cond / l_cond / hh_cond / ll_cond BOOL raw limit flags (with deadband, undelayed, unlatched)
*)
FUNCTION_BLOCK FB_ALARM_HL
  VAR_INPUT
    pv       : REAL;
    h_sp     : REAL := 80.0;
    l_sp     : REAL := 20.0;
    hh_sp    : REAL := 90.0;
    ll_sp    : REAL := 10.0;
    deadband : REAL := 0.0;
    delay_s  : REAL := 0.0;
    dt_s     : REAL := 0.1;
    ack      : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    h_alm   : BOOL;
    l_alm   : BOOL;
    hh_alm  : BOOL;
    ll_alm  : BOOL;
    any_alm : BOOL;
    h_cond  : BOOL;
    l_cond  : BOOL;
    hh_cond : BOOL;
    ll_cond : BOOL;
  END_VAR
  VAR
    h_t     : REAL; (* per-level delay accumulator, manual timing in place of TON *)
    l_t     : REAL;
    hh_t    : REAL;
    ll_t    : REAL;
    h_trip  : BOOL;
    l_trip  : BOOL;
    hh_trip : BOOL;
    ll_trip : BOOL;
  END_VAR

  (* ---- High alarm H ---- *)
  IF pv > h_sp THEN
    h_cond := TRUE;
  ELSIF pv < h_sp - deadband THEN
    h_cond := FALSE;
  END_IF;
  IF h_cond THEN
    h_t := h_t + dt_s;
  ELSE
    h_t := 0.0;
  END_IF;
  h_trip := h_cond AND (h_t >= delay_s);
  h_alm  := h_trip OR (h_alm AND NOT ack);

  (* ---- Low alarm L ---- *)
  IF pv < l_sp THEN
    l_cond := TRUE;
  ELSIF pv > l_sp + deadband THEN
    l_cond := FALSE;
  END_IF;
  IF l_cond THEN
    l_t := l_t + dt_s;
  ELSE
    l_t := 0.0;
  END_IF;
  l_trip := l_cond AND (l_t >= delay_s);
  l_alm  := l_trip OR (l_alm AND NOT ack);

  (* ---- High-high alarm HH ---- *)
  IF pv > hh_sp THEN
    hh_cond := TRUE;
  ELSIF pv < hh_sp - deadband THEN
    hh_cond := FALSE;
  END_IF;
  IF hh_cond THEN
    hh_t := hh_t + dt_s;
  ELSE
    hh_t := 0.0;
  END_IF;
  hh_trip := hh_cond AND (hh_t >= delay_s);
  hh_alm  := hh_trip OR (hh_alm AND NOT ack);

  (* ---- Low-low alarm LL ---- *)
  IF pv < ll_sp THEN
    ll_cond := TRUE;
  ELSIF pv > ll_sp + deadband THEN
    ll_cond := FALSE;
  END_IF;
  IF ll_cond THEN
    ll_t := ll_t + dt_s;
  ELSE
    ll_t := 0.0;
  END_IF;
  ll_trip := ll_cond AND (ll_t >= delay_s);
  ll_alm  := ll_trip OR (ll_alm AND NOT ack);

  any_alm := h_alm OR l_alm OR hh_alm OR ll_alm;
END_FUNCTION_BLOCK
