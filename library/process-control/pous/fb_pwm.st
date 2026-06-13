(*
  FB_PWM — time-proportioning output (0..100% command -> fixed-period duty-cycle on/off).

  ~ Time-proportioning (time-proportioning / PWM): continuous quantity -> periodic
  on/off, the customary form for solid-state-relay / solenoid-valve temperature control.

  Purpose: a PID output driving an on/off actuator via a solid-state relay / solenoid
  valve: electric heaters, steam on/off valves, dosing pumps.
  Behavior:
    Phase accumulator as in FB_PULSER: phase_s accumulates by dt_s, wraps by subtraction
    at period_s (so the long-term cadence does not drift); q := phase_s < on_s.
    on_s := u/100 * period_s (u clamped 0..100 first).
    Minimum on/off times (protecting the contactor/SSR/valve, implemented by clamping the effective duty):
      0 < on_s < min_on_s            -> on_s raised to min_on_s;
      0 < period_s - on_s < min_off_s -> on_s squeezed to period_s - min_off_s
      (the off side has higher priority; if min_on_s + min_off_s > period_s the
      parameters are contradictory and the off side wins -- at the extreme it stays
      permanently off, so do not tune it this way).
    u <= 0 -> off for the whole period; u >= 100 -> on for the whole period (full scale
    is unaffected by the minimum on/off clamp).
    NOT enable or period_s/dt_s <= 0 -> q := FALSE and phase cleared (re-enabling starts
    from the period start, on first then off).
    A change in u takes effect immediately (the comparison threshold within the period
    changes at once, no need to wait for the next period).

  Inputs:
    u         REAL  command 0..100 %
    period_s  REAL  PWM period s
    min_on_s  REAL  minimum on time s (0 = no limit)
    min_off_s REAL  minimum off time s (0 = no limit)
    enable    BOOL  enable
    dt_s      REAL  sample period s
  Outputs:
    q         BOOL  time-proportioning output
*)
FUNCTION_BLOCK FB_PWM
  VAR_INPUT
    u         : REAL;
    period_s  : REAL := 10.0;
    min_on_s  : REAL := 0.0;
    min_off_s : REAL := 0.0;
    enable    : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* phase within the period, manual timing in place of TP/TON *)
    uc      : REAL;
    on_s    : REAL; (* effective on-duration this scan *)
    off_s   : REAL;
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  on_s := uc * period_s / 100.0;

  (* Minimum on: raise the effective duty (u=0 unaffected) *)
  IF on_s > 0.0 AND on_s < min_on_s THEN
    on_s := min_on_s;
  END_IF;

  (* Minimum off: lower the effective duty (at u=100, off=0 so it does not trigger, stays permanently on) *)
  off_s := period_s - on_s;
  IF off_s > 0.0 AND off_s < min_off_s THEN
    on_s := period_s - min_off_s;
    IF on_s < 0.0 THEN
      on_s := 0.0;
    END_IF;
  END_IF;

  q := phase_s < on_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* subtraction wrap, keeps the long-term cadence *)
  END_IF;
END_FUNCTION_BLOCK
