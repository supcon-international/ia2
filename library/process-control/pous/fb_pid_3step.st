(*
  FB_PID_3STEP — step (3-step) PID: open/close pulse control of a motorized actuator,
  supporting both no-position-feedback (travel-time integral estimate) and
  with-feedback wiring.

  ~ Standard step PID / 3-step output controller (step controller / 3-step PID, driving
  integrating-type actuators such as motorized control valves and motorized dampers),
  found in every major DCS/PLC standard library.

  Purpose: closed-loop control of a motorized actuator (motorized valve / motorized
  damper, which is itself an integrating element: energized it runs open/close, de-energized
  it stops in place). Without a position transmitter, estimate position by integrating
  travel time (pos_fb < 0); with a 4-20 mA position feedback, use it directly.

  Behavior:
    1) Position estimate pos_est: pos_fb >= 0 takes the feedback directly (clamped
       0..100); otherwise integrate from the previous scan's output and the travel time:
       an open pulse adds +100*dt_s/motor_time_s, a close pulse decrements likewise
       (clamped 0..100).
    2) An internal positional PID (reverse-acting e = sp - pv, D on PV with automatic
       3*dt_s PT1 filtering) computes the target position target 0..100; integral
       back-calculation anti-windup; in manual, target tracks pos_est so the return to
       auto is bumpless. For direct-acting scenarios, invert pv/sp externally.
    3) Three-step output: position deviation = target - pos_est;
       > deadband_pct emits an open pulse, < -deadband_pct emits a close pulse, within
       the deadband stop. In auto, once a pulse is emitted it is held at least
       min_pulse_s (to avoid short pulses hammering the contactor); manual jogs
       man_open/man_close pass through (no minimum pulse width, mutually interlocked
       when both are commanded), constrained only by the travel limits. A reversal must
       pass through one scan fully stopped (stop before reversing). At a travel boundary
       (pos_est reaching 0 or 100), stop that direction's pulse immediately.
    Tuning tip: deadband_pct should be larger than the travel corresponding to the
    minimum pulse width (min_pulse_s / motor_time_s * 100%), otherwise it will still
    overshoot and oscillate within the deadband.

  Inputs:
    pv           REAL  process value
    sp           REAL  setpoint
    kp           REAL  proportional gain
    ti_s         REAL  integral time s (0 = no integral)
    td_s         REAL  derivative time s (0 = derivative off)
    dt_s         REAL  sample period s (<= 0 holds this scan, no action)
    motor_time_s REAL  actuator full-travel time s (0->100%; used for estimation and limits)
    min_pulse_s  REAL  minimum pulse width in auto s
    deadband_pct REAL  position deadband % (no action within the deviation deadband)
    auto         BOOL  TRUE auto / FALSE manual jog
    man_open     BOOL  manual open jog (interlocked, no action when commanded with man_close)
    man_close    BOOL  manual close jog
    pos_fb       REAL  position feedback 0..100 % (< 0 = no feedback, use travel estimate)
  Outputs:
    q_open  BOOL  open-direction pulse output
    q_close BOOL  close-direction pulse output
    pos_est REAL  position estimate/feedback 0..100 %
*)
FUNCTION_BLOCK FB_PID_3STEP
  VAR_INPUT
    pv           : REAL;
    sp           : REAL;
    kp           : REAL := 1.0;
    ti_s         : REAL := 0.0;
    td_s         : REAL := 0.0;
    dt_s         : REAL := 0.1;
    motor_time_s : REAL := 60.0;
    min_pulse_s  : REAL := 0.5;
    deadband_pct : REAL := 1.0;
    auto         : BOOL := TRUE;
    man_open     : BOOL := FALSE;
    man_close    : BOOL := FALSE;
    pos_fb       : REAL := -1.0;
  END_VAR
  VAR_OUTPUT
    q_open  : BOOL;
    q_close : BOOL;
    pos_est : REAL;
  END_VAR
  VAR
    i_acc     : REAL; (* positional-PID integral term (back-calculation bumpless) *)
    pv_f      : REAL; (* D-term PT1 filter state *)
    pv_f_prev : REAL;
    t_f       : REAL;
    e         : REAL;
    p_term    : REAL;
    d_term    : REAL;
    target    : REAL; (* PID target position 0..100 *)
    err_pos   : REAL; (* position deviation target - pos_est *)
    pulse_t   : REAL; (* elapsed duration of the current pulse s *)
    open_req  : BOOL; (* open-direction demand this scan *)
    close_req : BOOL;
    init_done : BOOL; (* filter-state first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  IF NOT init_done THEN
    pv_f := pv;
    pv_f_prev := pv;
    init_done := TRUE;
  END_IF;

  (* 1) Position estimate: use feedback directly when available; otherwise integrate from the previous scan's output over the travel time *)
  IF pos_fb >= 0.0 THEN
    pos_est := pos_fb;
  ELSE
    IF motor_time_s > 0.0 THEN
      IF q_open THEN
        pos_est := pos_est + 100.0 * dt_s / motor_time_s;
      END_IF;
      IF q_close THEN
        pos_est := pos_est - 100.0 * dt_s / motor_time_s;
      END_IF;
    END_IF;
  END_IF;
  IF pos_est > 100.0 THEN
    pos_est := 100.0;
  ELSIF pos_est < 0.0 THEN
    pos_est := 0.0;
  END_IF;

  (* 2) Positional PID -> target position (reverse-acting; D on PV, automatic 3*dt_s filtering) *)
  t_f := 3.0 * dt_s;
  pv_f := pv_f + dt_s / (t_f + dt_s) * (pv - pv_f);
  e := sp - pv;
  p_term := kp * e;
  d_term := 0.0;
  IF td_s > 0.0 THEN
    d_term := 0.0 - kp * td_s * (pv_f - pv_f_prev) / dt_s;
  END_IF;
  pv_f_prev := pv_f;

  IF auto THEN
    IF ti_s > 0.0 THEN
      i_acc := i_acc + kp * dt_s / ti_s * e;
    END_IF;
    target := p_term + i_acc + d_term;
  ELSE
    target := pos_est; (* Manual: target tracks the current position, bumpless return to auto *)
  END_IF;
  IF target > 100.0 THEN
    target := 100.0;
  ELSIF target < 0.0 THEN
    target := 0.0;
  END_IF;
  IF ti_s > 0.0 OR NOT auto THEN
    i_acc := target - p_term - d_term; (* Back-calculation: anti-windup + bumpless *)
  END_IF;

  (* 3) Three-step output demand *)
  IF auto THEN
    err_pos := target - pos_est;
    open_req := err_pos > deadband_pct;
    close_req := err_pos < 0.0 - deadband_pct;
  ELSE
    open_req := man_open AND NOT man_close; (* Manual jog passes through, interlocked when both commanded *)
    close_req := man_close AND NOT man_open;
  END_IF;

  (* Travel limits: at a boundary, stop emitting that direction's pulse (same in manual/auto) *)
  IF pos_est >= 100.0 THEN
    open_req := FALSE;
  END_IF;
  IF pos_est <= 0.0 THEN
    close_req := FALSE;
  END_IF;

  (* Pulse timing and output: in auto, dropping the demand must satisfy the minimum
     pulse width; in manual, dropping stops immediately; a reversal must pass through
     one scan fully stopped (the current direction drops first, the reverse is allowed
     only on the next scan) *)
  IF q_open OR q_close THEN
    pulse_t := pulse_t + dt_s;
  END_IF;

  IF q_open THEN
    IF pos_est >= 100.0 THEN
      q_open := FALSE; (* at limit, stop *)
    ELSIF NOT open_req AND (NOT auto OR pulse_t >= min_pulse_s) THEN
      q_open := FALSE;
    END_IF;
  ELSIF q_close THEN
    IF pos_est <= 0.0 THEN
      q_close := FALSE;
    ELSIF NOT close_req AND (NOT auto OR pulse_t >= min_pulse_s) THEN
      q_close := FALSE;
    END_IF;
  ELSE
    pulse_t := 0.0;
    IF open_req THEN
      q_open := TRUE;
    ELSIF close_req THEN
      q_close := TRUE;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
