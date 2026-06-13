(*
  FB_PID_POS — positional (absolute-output) PID controller with observable p/i/d terms,
  SP weighting, derivative-on-PV (D on PV + PT1 filter), back-calculation anti-windup
  and bumpless transfer.

  ~ Standard positional PID (absolute-output form, observable P/I/D terms, SP weighting
  b, derivative acting on PV), found in every major DCS/PLC standard library.

  Purpose: complements the velocity-form FB_PID -- for loops that need to observe/debug
  the integral term, want SP weighting (b < 1 suppresses the proportional kick on a
  setpoint step), or need an absolute-quantity output downstream. For incremental
  output / cascade relay scenarios, FB_PID is still recommended.

  Algorithm (positional):
    e   := sp - pv (reverse-acting, default); direct = TRUE gives e := pv - sp
    e_p := b*sp - pv (the SP-weighted deviation used only by the P term; reversed when direct)
    p_part := kp * e_p
    i_part := i_part + kp*dt_s/ti_s * e (accumulated when ti_s > 0)
    d_part := -kp*td_s * d(pv_f)/dt (derivative acts on PV: no kick on an SP step;
              pv first passes a PT1 filter, t_dfilt_s <= 0 takes 3*dt_s automatically;
              sign goes positive when direct)
    out := p_part + i_part + d_part + ff, clamped to [out_min, out_max]
  Anti-windup and bumpless (back-calculation): each scan wraps up by back-calculating i_part as
    i_part := out - p_part - d_part - ff
  -- on clamping the integral is pulled back (no windup); in manual/track the output
  follows the supplied value and i_part back-calculates in step, so the return to auto
  has no jump. When ti_s = 0 (no integral), i_part is back-calculated only during
  manual/track (where it is a fixed bias / manual-reset term, not overwritten by
  clamping events during auto, keeping the offset characteristic of P-type control
  stable).
  Mode priority: track > NOT auto (manual) > auto (same as FB_PID).
  Identity: in every mode out = p_part + i_part + d_part + ff (after clamping, except
  on an auto clamping scan with ti_s = 0) -- the sum of the terms is the output,
  intuitive for debugging.

  Inputs:
    pv          REAL  process value
    sp          REAL  setpoint
    kp          REAL  proportional gain
    ti_s        REAL  integral time s (0 = no integral)
    td_s        REAL  derivative time s (0 = derivative off)
    t_dfilt_s   REAL  derivative pre-filter PT1 time constant s (<= 0 = auto 3*dt_s)
    sp_weight_b REAL  SP weighting coefficient b (acts on the P term only; 1 = no weighting,
                      0 = the P term sees pv only -- an SP step gives no proportional kick at all)
    dt_s        REAL  sample period s (<= 0 holds this scan, no action)
    direct      BOOL  TRUE = direct-acting (pv up -> out up), FALSE = reverse-acting
    auto        BOOL  TRUE auto / FALSE manual
    manual_out  REAL  manual output value
    out_min     REAL  output low limit (assumes out_min < out_max)
    out_max     REAL  output high limit
    ff          REAL  feedforward (added to output before clamp)
    track       BOOL  output-tracking enable (highest priority)
    track_value REAL  tracking value
  Outputs:
    out    REAL  control output (clamped in every mode)
    dev    REAL  deviation (reverse-acting sp-pv / direct-acting pv-sp, without SP weighting)
    p_part REAL  proportional term (includes SP weighting)
    i_part REAL  integral term (includes back-calculation correction)
    d_part REAL  derivative term
*)
FUNCTION_BLOCK FB_PID_POS
  VAR_INPUT
    pv          : REAL;
    sp          : REAL;
    kp          : REAL := 1.0;
    ti_s        : REAL := 0.0;
    td_s        : REAL := 0.0;
    t_dfilt_s   : REAL := 0.0;
    sp_weight_b : REAL := 1.0;
    dt_s        : REAL := 0.1;
    direct      : BOOL := FALSE;
    auto        : BOOL := TRUE;
    manual_out  : REAL := 0.0;
    out_min     : REAL := 0.0;
    out_max     : REAL := 100.0;
    ff          : REAL := 0.0;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    dev    : REAL;
    p_part : REAL;
    i_part : REAL;
    d_part : REAL;
  END_VAR
  VAR
    pv_f      : REAL; (* PV after the D-term PT1 filter *)
    pv_f_prev : REAL; (* previous-scan filtered value, for the backward difference *)
    t_f       : REAL; (* filter time constant actually used *)
    d_pv      : REAL; (* rate of change of the filtered PV, units/s *)
    e         : REAL; (* control deviation (includes the action direction) *)
    e_p       : REAL; (* P-term deviation (SP weighting b) *)
    init_done : BOOL; (* filter-state first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold output and history unchanged *)
  END_IF;

  IF NOT init_done THEN
    pv_f := pv; (* align the filter state: no spurious startup derivative *)
    pv_f_prev := pv;
    init_done := TRUE;
  END_IF;

  (* D-term pre-filter PT1 + backward difference (same core as FB_DERIV) *)
  t_f := t_dfilt_s;
  IF t_f <= 0.0 THEN
    t_f := 3.0 * dt_s;
  END_IF;
  pv_f := pv_f + dt_s / (t_f + dt_s) * (pv - pv_f);
  d_pv := (pv_f - pv_f_prev) / dt_s;
  pv_f_prev := pv_f;

  (* Deviation (by action direction) and the SP-weighted P deviation *)
  IF direct THEN
    e := pv - sp;
    e_p := pv - sp_weight_b * sp;
  ELSE
    e := sp - pv;
    e_p := sp_weight_b * sp - pv;
  END_IF;
  dev := e;

  p_part := kp * e_p;

  d_part := 0.0;
  IF td_s > 0.0 THEN
    (* Derivative acts on PV (no kick on an SP step): reverse-acting, rising pv should push the output down -> negative sign *)
    IF direct THEN
      d_part := kp * td_s * d_pv;
    ELSE
      d_part := 0.0 - kp * td_s * d_pv;
    END_IF;
  END_IF;

  IF track THEN
    (* Output tracking: follows track_value unconditionally, highest priority *)
    out := track_value;
  ELSIF NOT auto THEN
    (* Manual: follows manual_out; the i_part back-calculation guarantees a bumpless return to auto *)
    out := manual_out;
  ELSE
    IF ti_s > 0.0 THEN
      i_part := i_part + kp * dt_s / ti_s * e;
    END_IF;
    out := p_part + i_part + d_part + ff;
  END_IF;

  (* Output clamp *)
  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;

  (* Back-calculate i_part: makes P+I+D+ff always equal the clamped output -> clamping
     gives anti-windup and manual/track gives bumpless alignment. When ti_s = 0,
     back-calculate only during manual/track (see header note) *)
  IF ti_s > 0.0 OR track OR NOT auto THEN
    i_part := out - p_part - d_part - ff;
  END_IF;
END_FUNCTION_BLOCK
