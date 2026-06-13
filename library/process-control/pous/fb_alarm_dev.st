(*
  FB_ALARM_DEV — deviation alarm (pv over/under deviation from sp), deadband + delay + acknowledge latch.

  ~ Deviation alarm (over/under deviation), standard for loop monitoring.

  Purpose: loop-performance monitoring: alarm on a control deviation that persistently
  exceeds its limit (a stuck valve, an abrupt load change, lost tuning, a failed actuator).
  Behavior (follows the FB_ALARM_HL deadband+delay+latch pattern):
    dev := pv - sp.
    Over deviation:  dev > dev_h sets; dev < dev_h - deadband resets.
    Under deviation: dev < -dev_l sets; dev > -dev_l + deadband resets (dev_l supplied positive).
    Each trips after persisting for delay_s; output = trip OR (latched AND NOT ack)
    -- while the condition exists the alarm stays on, after it clears it is held until acknowledged.

  Inputs:
    pv       REAL  process value
    sp       REAL  setpoint (feed the same sp source as FB_PID; if the PID uses sp_rate,
                   prefer feeding the actual working sp after the ramp)
    dev_h    REAL  over-deviation limit (allowed amount pv may exceed sp)
    dev_l    REAL  under-deviation limit (allowed amount pv may fall below sp, supplied positive)
    deadband REAL  deadband (hysteresis, engineering units)
    delay_s  REAL  alarm delay s (0 = immediate)
    dt_s     REAL  sample period s
    ack      BOOL  acknowledge (level-active)
  Outputs:
    h_alm   BOOL  over-deviation alarm (latched)
    l_alm   BOOL  under-deviation alarm (latched)
    any_alm BOOL  any deviation alarm
*)
FUNCTION_BLOCK FB_ALARM_DEV
  VAR_INPUT
    pv       : REAL;
    sp       : REAL;
    dev_h    : REAL := 10.0;
    dev_l    : REAL := 10.0;
    deadband : REAL := 0.0;
    delay_s  : REAL := 0.0;
    dt_s     : REAL := 0.1;
    ack      : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    h_alm   : BOOL;
    l_alm   : BOOL;
    any_alm : BOOL;
  END_VAR
  VAR
    dv     : REAL; (* pv - sp *)
    h_cond : BOOL;
    l_cond : BOOL;
    h_t    : REAL; (* delay accumulator, manual timing in place of TON *)
    l_t    : REAL;
    h_trip : BOOL;
    l_trip : BOOL;
  END_VAR

  dv := pv - sp;

  (* ---- Over deviation ---- *)
  IF dv > dev_h THEN
    h_cond := TRUE;
  ELSIF dv < dev_h - deadband THEN
    h_cond := FALSE;
  END_IF;
  IF h_cond THEN
    h_t := h_t + dt_s;
  ELSE
    h_t := 0.0;
  END_IF;
  h_trip := h_cond AND (h_t >= delay_s);
  h_alm  := h_trip OR (h_alm AND NOT ack);

  (* ---- Under deviation ---- *)
  IF dv < 0.0 - dev_l THEN
    l_cond := TRUE;
  ELSIF dv > 0.0 - dev_l + deadband THEN
    l_cond := FALSE;
  END_IF;
  IF l_cond THEN
    l_t := l_t + dt_s;
  ELSE
    l_t := 0.0;
  END_IF;
  l_trip := l_cond AND (l_t >= delay_s);
  l_alm  := l_trip OR (l_alm AND NOT ack);

  any_alm := h_alm OR l_alm;
END_FUNCTION_BLOCK
