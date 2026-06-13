(*
  FB_PID — incremental (velocity-form) PID controller with anti-windup,
  bumpless transfer, feedforward, output tracking and setpoint ramp.

  ~ Universal PID essentials: SP ramp / output tracking / feedforward / auto-manual
    bumpless transfer (every major DCS/PLC standard library has an equivalent block).

  Typical use: a flow PID driving a variable-speed pump, direct-acting, where the
  output limits are the drive min/max frequency (e.g. 10..45 Hz — flow is pv, the
  drive frequency reference is out: out_min:=10.0, out_max:=45.0, direct := TRUE
  direct-acting — rising pv raises output).

  Algorithm (incremental + feedforward):
    sp_int := sp through an sp_rate-limited ramp (sp_rate <= 0 takes sp directly; first-scan alignment)
    e      := sp_int - pv (reverse-acting, default); direct=TRUE gives e := pv - sp_int
    dpid   := kp*(e - e_prev)                            -- proportional increment
            + kp*dt_s/ti_s * e (when ti_s>0)             -- integral increment
            + kp*td_s/dt_s * (e - 2*e_prev + e_prev2) (when td_s>0) -- derivative increment
    acc    := acc + dpid; out := acc + ff, clamped to [out_min, out_max]
    wrap-up  acc := out - ff (back-calculation)
  Anti-windup: output clamp + back-calculation + conditional integration (when the
  output is already at a limit and the integral increment keeps pushing toward that
  limit, the integral term is dropped).
  Mode priority: track (output tracking, for interlock forcing / external takeover)
  > NOT auto (manual) > auto. In track/manual the output follows the supplied value
  (clamped), the error history keeps updating, and returning to auto is bumpless.
  ff adds directly onto the pre-clamp output (feedforward takes effect immediately;
  back-calculation keeps ff changes, clamping and mode switching all bumpless).
  Backward compatible: the added inputs (ff/track/track_value/sp_rate) all have
  defaults, and at their defaults behavior is scan-for-scan identical to the old
  version, so existing calls need no changes; the added output dev does not affect
  legacy wiring.

  Inputs:
    pv          REAL  process value
    sp          REAL  setpoint
    kp          REAL  proportional gain
    ti_s        REAL  integral time s (0 = integral off)
    td_s        REAL  derivative time s (0 = derivative off, derivative term unfiltered)
    dt_s        REAL  sample period s (<=0 holds this scan, no action)
    direct      BOOL  TRUE = direct-acting (pv up -> out up), FALSE = reverse-acting
    auto        BOOL  TRUE auto / FALSE manual
    manual_out  REAL  manual output value
    out_min     REAL  output low limit (assumes out_min < out_max)
    out_max     REAL  output high limit
    ff          REAL  feedforward (added to output before clamp; feed via FB_LEADLAG for dynamic shaping)
    track       BOOL  output-tracking enable (highest priority, overrides auto/manual)
    track_value REAL  tracking value
    sp_rate     REAL  setpoint ramp rate limit units/s (0 = no rate limit)
  Outputs:
    out         REAL  control output (clamped in every mode)
    dev         REAL  deviation pv - sp_int (actual working deviation after the SP ramp)
*)
FUNCTION_BLOCK FB_PID
  VAR_INPUT
    pv          : REAL;
    sp          : REAL;
    kp          : REAL := 1.0;
    ti_s        : REAL := 0.0;
    td_s        : REAL := 0.0;
    dt_s        : REAL := 0.1;
    direct      : BOOL := FALSE;
    auto        : BOOL := TRUE;
    manual_out  : REAL := 0.0;
    out_min     : REAL := 0.0;
    out_max     : REAL := 100.0;
    ff          : REAL := 0.0;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
    sp_rate     : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out : REAL;
    dev : REAL;
  END_VAR
  VAR
    sp_int    : REAL; (* internal working setpoint after the ramp *)
    acc       : REAL; (* PID accumulator (excludes ff) *)
    e         : REAL;
    e_prev    : REAL;
    e_prev2   : REAL;
    p_term    : REAL;
    i_term    : REAL;
    d_term    : REAL;
    sp_step   : REAL;
    sp_delta  : REAL;
    init_done : BOOL; (* sp_int first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold output and history unchanged *)
  END_IF;

  (* Setpoint ramp: when sp_rate > 0, sp_int approaches sp at the rate limit; first scan aligns directly *)
  IF NOT init_done THEN
    sp_int := sp;
    init_done := TRUE;
  ELSIF sp_rate > 0.0 THEN
    sp_step := sp_rate * dt_s;
    sp_delta := sp - sp_int;
    IF sp_delta > sp_step THEN
      sp_int := sp_int + sp_step;
    ELSIF sp_delta < 0.0 - sp_step THEN
      sp_int := sp_int - sp_step;
    ELSE
      sp_int := sp;
    END_IF;
  ELSE
    sp_int := sp;
  END_IF;

  dev := pv - sp_int;

  (* Control deviation; direct-acting flips the sign *)
  IF direct THEN
    e := pv - sp_int;
  ELSE
    e := sp_int - pv;
  END_IF;

  IF track THEN
    (* Output tracking: follows track_value unconditionally (interlock forcing / override takeover), highest priority *)
    out := track_value;
  ELSIF NOT auto THEN
    (* Manual: follows manual_out; the incremental form makes the return to auto naturally bumpless *)
    out := manual_out;
  ELSE
    p_term := kp * (e - e_prev);

    i_term := 0.0;
    IF ti_s > 0.0 THEN
      i_term := kp * dt_s / ti_s * e;
      (* Conditional integration: when the output is saturated and the integral still pushes toward the limit, drop the integral term this scan *)
      IF (out >= out_max AND i_term > 0.0) OR (out <= out_min AND i_term < 0.0) THEN
        i_term := 0.0;
      END_IF;
    END_IF;

    d_term := 0.0;
    IF td_s > 0.0 THEN
      d_term := kp * td_s / dt_s * (e - 2.0 * e_prev + e_prev2);
    END_IF;

    acc := acc + p_term + i_term + d_term;
    out := acc + ff; (* feedforward added before clamp *)
  END_IF;

  (* Output clamp (in the incremental form this also provides anti-windup) *)
  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;

  (* Back-calculation: acc always = clamped output - feedforward -> clamping / ff changes / manual / tracking all bumpless *)
  acc := out - ff;

  e_prev2 := e_prev;
  e_prev  := e;
END_FUNCTION_BLOCK
