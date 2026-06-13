(*
  FB_ALARM_ROC — rate-of-change alarm, with derivative filtering, delay and acknowledge latch.

  ~ Rate-of-change alarm, with the derivative denoised by a first-order filter.

  Purpose: monitoring abnormal rates of change such as temperature runaway, abrupt
  pressure change, level collapse (leak/pipe break).
  Behavior:
    Raw derivative raw_roc := (pv - pv_prev) / dt_s;
    roc is raw_roc through an internal PT1 filter (time constant fixed at 3*dt_s,
    settling in about 4 scans) -- a single-scan difference is extremely sensitive to
    quantization/noise and will false-alarm without filtering; for stronger denoising
    put FB_LAG ahead of pv.
    |roc| > roc_limit trips after persisting for delay_s; latched until ack (same as the
    FB_ALARM_HL pattern: while the condition exists the alarm stays on, after it clears
    it is held until acknowledged).
    roc_limit <= 0 -> monitoring disabled (trip condition always FALSE; a latched output
    still needs ack to clear).
    The first scan only aligns pv_prev and does not evaluate the alarm (avoiding a
    spurious startup derivative); dt_s <= 0 -> hold.

  Inputs:
    pv        REAL  process value
    roc_limit REAL  rate-of-change limit |units|/s (bidirectional)
    delay_s   REAL  alarm delay s (0 = immediate)
    dt_s      REAL  sample period s
    ack       BOOL  acknowledge (level-active)
  Outputs:
    alm BOOL  rate-of-change alarm (latched)
    roc REAL  filtered rate of change units/s (signed, usable for trend display)
*)
FUNCTION_BLOCK FB_ALARM_ROC
  VAR_INPUT
    pv        : REAL;
    roc_limit : REAL := 10.0;
    delay_s   : REAL := 0.0;
    dt_s      : REAL := 0.1;
    ack       : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    alm : BOOL;
    roc : REAL;
  END_VAR
  VAR
    pv_prev   : REAL;
    raw_roc   : REAL;
    t_f       : REAL; (* derivative filter time constant = 3*dt_s *)
    cond      : BOOL;
    trip      : BOOL;
    alm_t     : REAL; (* delay accumulator *)
    init_done : BOOL;
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* hold *)
  END_IF;

  IF NOT init_done THEN
    pv_prev := pv; (* first-scan alignment, no alarm evaluation *)
    roc := 0.0;
    init_done := TRUE;
    RETURN;
  END_IF;

  raw_roc := (pv - pv_prev) / dt_s;
  pv_prev := pv;

  (* Internal PT1 denoising: t_f = 3*dt_s fixed (absorbs 1/4 of the new value each scan) *)
  t_f := 3.0 * dt_s;
  roc := roc + dt_s / (t_f + dt_s) * (raw_roc - roc);

  cond := FALSE;
  IF roc_limit > 0.0 THEN
    IF roc > roc_limit OR roc < 0.0 - roc_limit THEN
      cond := TRUE;
    END_IF;
  END_IF;

  IF cond THEN
    alm_t := alm_t + dt_s;
  ELSE
    alm_t := 0.0;
  END_IF;
  trip := cond AND (alm_t >= delay_s);
  alm := trip OR (alm AND NOT ack);
END_FUNCTION_BLOCK
