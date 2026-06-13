(* ============================================================================
  demo_main — mini carbonation-tower loop demonstration PROGRAM (self-contained file).

  Note: this file inlines verbatim copies of the 28 FB declarations it instantiates
  (FB_SCALE / FB_PID / FB_ALARM_HL / FB_PULSER / FB_HILO_FILL / FB_RATIO /
  FB_SPLIT_RANGE / FB_RUNTIME / FB_DUTY2 / FB_INTERLOCK8 / FB_LIMIT / FB_STATS /
  FB_AVG_TIME / FB_INTEG / FB_DERIV / FB_DEADTIME8 / FB_MID3 / FB_CASCADE /
  FB_PV_HOLD / FB_RAMPSOAK8 / FB_GAIN_SCHED3 / FB_PID_POS / FB_MANSTATION /
  FB_PID_3STEP / FB_MOTOR_REV / FB_MOV / FB_DOSE / FB_FLOW_COMP). Reason: `cs check`
  checks each file independently and does no cross-file type resolution; only a
  project-level compile (cs project check / tasks.toml-scheduled compile) merges and
  resolves all files under pous/. Being self-contained lets `cs check pous/*.st` pass
  as a group.
  Therefore: when importing this file into a real project, do not let it coexist with
  the individual FB files such as fb_scale.st in the same directory (although a project
  compile is observed to tolerate verbatim-duplicate declarations, nothing guards
  against copy drift) -- pick one.

  Loop wiring (demonstration; variables can be bound to real channels via iomap.toml):
    lt0101_raw (AI raw counts) -> FB_SCALE -> level_pct (tower level %)
    level_pct / level_sp     -> FB_PID (reverse-acting, legacy-signature call, verifies backward compatibility)
                               -> valve_pct (feed control-valve opening)
    level_pct                -> FB_ALARM_HL -> four-level level alarms + any_alm
    hammer_enable            -> FB_PULSER -> hammer_q (periodic air-cannon purge)
    level_pct                -> FB_HILO_FILL -> fill_open (high-close low-open make-up)
    -- the following are extension loops --
    level HH / open-circuit / tower-temp HH / dual-pump fault -> FB_INTERLOCK8 -> ilk_trip + first-out number
    pump_run_fb_a/b          -> FB_RUNTIME x2 -> dual-pump hours / start counts
    fill_open AND NOT ilk    -> FB_DUTY2 (hour balancing) -> dual make-up pumps duty/standby
    gas_flow (leading flow)  -> FB_RATIO -> milk_sp (lime-milk ratio setpoint)
    milk_flow / milk_sp      -> FB_PID (feedforward ff + SP ramp + interlock track forcing)
                               -> FB_SPLIT_RANGE (sequential split range) -> small/large valve
    -- the following demonstrate the math/dynamics blocks (steam-heating cascade + FOPDT plant simulation) --
    steam_valve -> FB_DEADTIME8(20 s) -> manual PT1(30 s) -> tower temp temp_a
                               (dead time + first-order lag = FOPDT plant simulation)
    temp_a / temp_b / bad point -> FB_MID3 (two good one bad, take the average) -> FB_LIMIT -> voted tower temp
    voted tower temp          -> FB_DERIV -> rate of change degC/s;
                               -> FB_STATS (full-run statistics) + FB_AVG_TIME (60 s tumbling window)
    tower-temp master FB_PID -> FB_CASCADE -> steam-flow slave FB_PID -> steam_valve
                               (when not cascaded the master tracks via track, CAS transfer bumpless)
    master PID deviation dev  -> FB_INTEG (interlock hold, clamped +/-1000) -> deviation integral degC*s
    -- the following demonstrate the 0.3.0 loop/device/signal extension blocks (sections 17~26) --
    voted tower temp + q_fail -> FB_PV_HOLD (bad value held 5 s first then hold/substitute) -> temp_held
    heat-up/soak 4-segment curve -> FB_RAMPSOAK8 -> rs_sp (program setpoint)
    temp_held                -> FB_GAIN_SCHED3 (three-zone parameters + boundary transition)
                               -> FB_PID_POS (program SP + scheduled parameters, observable p/i/d)
                               -> FB_MANSTATION (manual station, balance bumpless) -> heat_out
    temp_held / rs_sp        -> FB_PID_3STEP (motorized-actuator pulse control, for observation)
    agitator fwd/rev buttons  -> FB_MOTOR_REV (reversal interlock 2 s, feedback echo simulation)
    fill_open                -> FB_MOV (motorized make-up valve, 30 s simulated travel, dual limits)
    dose start               -> FB_DOSE (coarse 8 / fine 1.5 units/s simulated flow)
    gas_flow + operating p/t  -> FB_FLOW_COMP (square-root DP pressure/temperature compensation) -> gas_flow_std
  scan_s must match the tasks.toml interval_ms (default 100 ms = 0.1 s).
============================================================================ *)

(*
  FB_SCALE — linear span conversion: raw counts -> engineering units, with NE43-style open-circuit flag.

  Purpose: analog input scaling. For example, a 4-20 mA signal sampled as raw
  counts (S7-style 5530..27648) is converted to an engineering value such as
  0..100 % or 0..5 m.

  Formula: out := out_min + (raw - raw_min) * (out_max - out_min) / span
           where span = MAX(raw_max - raw_min, 1e-6) (inline IF, divide-by-zero guard).
  Assumes raw_max > raw_min (forward span).

  Inputs:
    raw      REAL  raw count value (e.g. AI channel raw value)
    raw_min  REAL  raw value at span low (4 mA point)
    raw_max  REAL  raw value at span high (20 mA point)
    out_min  REAL  engineering span low
    out_max  REAL  engineering span high
    clamp    BOOL  TRUE clamps output to [out_min, out_max] (reverse engineering span supported)
  Outputs:
    out      REAL  engineering value
    fault    BOOL  raw value out of range >5% of span (NE43-style open-circuit/over-range flag, not latched)
*)
FUNCTION_BLOCK FB_SCALE
  VAR_INPUT
    raw     : REAL;
    raw_min : REAL := 0.0;
    raw_max : REAL := 27648.0;
    out_min : REAL := 0.0;
    out_max : REAL := 100.0;
    clamp   : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out   : REAL;
    fault : BOOL;
  END_VAR
  VAR
    span   : REAL;
    margin : REAL;
    lo     : REAL;
    hi     : REAL;
  END_VAR

  (* span = MAX(raw_max - raw_min, 1e-6) — inline IF, divide-by-zero guard *)
  span := raw_max - raw_min;
  IF span < 0.000001 THEN
    span := 0.000001;
  END_IF;

  (* NE43-style out-of-range test: more than 5% beyond raw span = open-circuit/fault *)
  margin := 0.05 * span;
  fault := (raw < raw_min - margin) OR (raw > raw_max + margin);

  out := out_min + (raw - raw_min) * (out_max - out_min) / span;

  IF clamp THEN
    IF out_min <= out_max THEN
      lo := out_min;
      hi := out_max;
    ELSE
      lo := out_max;
      hi := out_min;
    END_IF;
    IF out < lo THEN
      out := lo;
    ELSIF out > hi THEN
      out := hi;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
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
(*
  FB_PULSER — periodic pulse generator.

  Purpose: periodic air-cannon / air-hammer silo purging, periodic lubrication, heartbeat output.
  Example: once enabled, output a pulse_len_s-wide pulse every period_s seconds (the pulse comes at the period start).

  Behavior:
    while enabled, the phase phase_s accumulates by dt_s manually; q := TRUE while
    phase_s < pulse_len_s, wrapping at period_s (subtraction wrap, so the long-term
    cadence does not drift).
    NOT enable or invalid parameters (period_s/dt_s <= 0) -> q := FALSE and phase cleared
    (re-enabling starts from the pulse).
    pulse_len_s >= period_s -> q is always TRUE (continuous output by definition).

  Inputs:
    enable      BOOL  enable
    period_s    REAL  pulse period s
    pulse_len_s REAL  pulse width s
    dt_s        REAL  sample period s
  Outputs:
    q BOOL  pulse output
*)
FUNCTION_BLOCK FB_PULSER
  VAR_INPUT
    enable      : BOOL;
    period_s    : REAL := 60.0;
    pulse_len_s : REAL := 1.0;
    dt_s        : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* phase within the period 0..period_s, manual timing in place of TP/TON *)
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  (* Emit the pulse by the current phase first (TRUE immediately on the first enabled period), then advance the phase *)
  q := phase_s < pulse_len_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* subtraction wrap, keeps the long-term cadence *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_HILO_FILL — high-close, low-open tank/silo replenishment control.

  Purpose: below the low setpoint open the supply valve, above the high setpoint close
  it, hold the current state in between (equivalent to a low-side-on hysteresis switch,
  given a dedicated name for readability in configuration).
  Example: circulating-tank make-up: pv <= l_sp opens the make-up valve, pv >= h_sp closes it.

  Behavior:
    pv <= l_sp -> valve_open := TRUE (low-open)
    pv >= h_sp -> valve_open := FALSE (high-close)
    l_sp < pv < h_sp -> hold.
  Assumes l_sp < h_sp; if reversed, the high test takes priority (opens then is
  immediately closed, output FALSE).

  Inputs:
    pv   REAL  level process value
    h_sp REAL  high setpoint (close)
    l_sp REAL  low setpoint (open)
  Outputs:
    valve_open BOOL  supply-valve output (TRUE open)
*)
FUNCTION_BLOCK FB_HILO_FILL
  VAR_INPUT
    pv   : REAL;
    h_sp : REAL := 85.0;
    l_sp : REAL := 15.0;
  END_VAR
  VAR_OUTPUT
    valve_open : BOOL;
  END_VAR

  IF pv >= h_sp THEN
    valve_open := FALSE; (* high-close, priority *)
  ELSIF pv <= l_sp THEN
    valve_open := TRUE;  (* low-open *)
  END_IF;
  (* Hold in between *)
END_FUNCTION_BLOCK
(*
  FB_RATIO — ratio station (ratio setpoint generator).

  ~ Ratio station: the customary block for proportioning/blending loops, output feeds the downstream PID's sp.

  Purpose: ratio control: the follower flow setpoint = wild (leading) flow * ratio +
  bias, and the sp output feeds the sp input of a downstream FB_PID to form the ratio
  loop.
  Example: a follower stream tracks the leading flow at a 0.85 proportion.

  Formula: sp := clamp(ratio, ratio_min, ratio_max) * wild_flow + bias.
  Note: only ratio is clamped (a safety limit on the operator-adjustable ratio), sp is
  not clamped -- sp's dimension follows wild_flow, and the downstream PID has its own
  output clamp.

  Inputs:
    wild_flow REAL  leading flow (the "wild" flow not controlled by this loop)
    ratio     REAL  ratio
    ratio_min REAL  ratio low limit
    ratio_max REAL  ratio high limit
    bias      REAL  bias (engineering units)
  Outputs:
    sp        REAL  follower-loop setpoint
*)
FUNCTION_BLOCK FB_RATIO
  VAR_INPUT
    wild_flow : REAL;
    ratio     : REAL := 1.0;
    ratio_min : REAL := 0.0;
    ratio_max : REAL := 10.0;
    bias      : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    sp : REAL;
  END_VAR
  VAR
    ratio_used : REAL;
  END_VAR

  ratio_used := ratio;
  IF ratio_used < ratio_min THEN
    ratio_used := ratio_min;
  ELSIF ratio_used > ratio_max THEN
    ratio_used := ratio_max;
  END_IF;

  sp := ratio_used * wild_flow + bias;
END_FUNCTION_BLOCK
(*
  FB_SPLIT_RANGE — split-range output: one controller output distributed to two valves at the split point.

  ~ Split range: one control output drives two valves by segment (heating/cooling, large/small valves).

  Purpose: one PID drives two actuators: large/small valves extending the range
  (sequential split range, reverse_b := FALSE), or a heating/cooling or
  feed/reflux complementary split range (reverse_b := TRUE, tuned per the actuator's
  air-to-open/air-to-close action and the PID action direction).

  Behavior (u is clamped 0..100 first, both outputs also clamped 0..100):
    out_a: linear 0->100 for u in [0, split], holds 100 once u >= split.
    out_b: linear 0->100 for u in [split, 100], 0 when u <= split;
           when reverse_b = TRUE, segment B reverses to 100->0 (with the end value held
           outside the segment, i.e. out_b = 100 when u <= split, output continuous with
           no jump).
    Curve sketch (split = 50):
      reverse_b = FALSE (large/small valve sequential split range):
        u:     0 .... 50 .... 100
        out_a: 0 -> 100 | holds 100
        out_b: 0   0    | 0 -> 100
      reverse_b = TRUE (complementary, B fully open first then closing):
        u:     0 .... 50 .... 100
        out_a: 0 -> 100 | holds 100
        out_b: 100 100  | 100 -> 0
    Degenerate: split <= 0 -> segment A has no range (u > 0 means 100); split >= 100 ->
    segment B has no range (acts only once u >= 100). split should be set within (0, 100).

  Inputs:
    u         REAL  controller output 0..100
    split     REAL  split point % (segment B start)
    reverse_b BOOL  TRUE = valve B is reverse-acting within its split segment (100->0)
  Outputs:
    out_a     REAL  valve A opening 0..100
    out_b     REAL  valve B opening 0..100
*)
FUNCTION_BLOCK FB_SPLIT_RANGE
  VAR_INPUT
    u         : REAL;
    split     : REAL := 50.0;
    reverse_b : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out_a : REAL;
    out_b : REAL;
  END_VAR
  VAR
    uc     : REAL; (* clamped input *)
    span_b : REAL;
  END_VAR

  (* Input clamp 0..100 *)
  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  (* Valve A: 0->100 for u in [0, split] *)
  IF split > 0.0 THEN
    out_a := uc * 100.0 / split;
  ELSE
    (* Degenerate: A has no split segment *)
    IF uc > 0.0 THEN
      out_a := 100.0;
    ELSE
      out_a := 0.0;
    END_IF;
  END_IF;
  IF out_a < 0.0 THEN
    out_a := 0.0;
  ELSIF out_a > 100.0 THEN
    out_a := 100.0;
  END_IF;

  (* Valve B: 0->100 for u in [split, 100] *)
  span_b := 100.0 - split;
  IF span_b > 0.0 THEN
    out_b := (uc - split) * 100.0 / span_b;
  ELSE
    (* Degenerate: B has no split segment *)
    IF uc >= split THEN
      out_b := 100.0;
    ELSE
      out_b := 0.0;
    END_IF;
  END_IF;
  IF out_b < 0.0 THEN
    out_b := 0.0;
  ELSIF out_b > 100.0 THEN
    out_b := 100.0;
  END_IF;

  (* Segment B reverse action (reverse after clamping, hold the end value outside the segment, output continuous) *)
  IF reverse_b THEN
    out_b := 100.0 - out_b;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_RUNTIME — equipment run-hours + start-count statistics.

  ~ Equipment run statistics (accumulated run hours + start count), standard
  maintenance data.

  Purpose: pump/fan wear balancing (feed hours to FB_DUTY2's hours_a/b with alt_mode=1),
  maintenance-due reminders, start/stop frequency statistics.
  Behavior:
    while running is TRUE, hours := hours + dt_s/3600;
    on a running rising edge, starts := starts + 1;
    reset level-active clears both (priority, no accumulation during a reset scan).
    The caller may place the instance under VAR RETAIN for retention across power loss
    (same caveat as FB_TOTALIZER: IA2 retain stores as i32, so the retention precision
    of the REAL hours is limited by this; starts is an integer count and is unaffected).

  Inputs:
    running BOOL  equipment running (feed the run feedback, debounce with FB_DEBOUNCE if needed)
    reset   BOOL  clear (level-active, priority; used after maintenance / pump replacement)
    dt_s    REAL  sample period s
  Outputs:
    hours  REAL  accumulated run hours
    starts DINT  accumulated start count
*)
FUNCTION_BLOCK FB_RUNTIME
  VAR_INPUT
    running : BOOL;
    reset   : BOOL := FALSE;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    hours  : REAL;
    starts : DINT;
  END_VAR
  VAR
    prev_run : BOOL; (* rising-edge memory *)
  END_VAR

  IF reset THEN
    hours := 0.0;
    starts := 0;
  ELSE
    IF running AND NOT prev_run THEN
      starts := starts + 1;
    END_IF;
    IF running AND dt_s > 0.0 THEN
      hours := hours + dt_s / 3600.0;
    END_IF;
  END_IF;
  prev_run := running;
END_FUNCTION_BLOCK
(*
  FB_DUTY2 — dual-pump duty/standby rotation control.

  ~ Dual-pump duty/standby rotation: select the duty pump by start sequence or run-hour
  balancing, with immediate takeover on fault.

  Purpose: two pumps in a duty/standby arrangement: select the duty pump by rotating
  each start or by balancing accumulated run hours; on a duty-pump fault the standby
  takes over immediately; on a dual fault both stop.
  Behavior (scan-deterministic: no internal timing, only edge memory):
    A demand rising edge selects this round's duty pump, by priority:
      1) exactly one is already running (run_fb TRUE on one side) -> adopt it as duty
         directly (bumpless takeover, avoiding stopping it to start the standby);
      2) alt_mode = 1 -> the lower accumulated hours is duty (hours_a/b fed from two FB_RUNTIME);
      3) alt_mode = 0 (default) -> toggle from the current duty position (including the
         state after a fault takeover, giving natural wear balancing).
    While demand persists: a duty-pump fault with the other healthy -> duty switches
    immediately (takeover); a commanded pump that faults gets no command; a dual fault
    -> both cmd FALSE (the duty selection is held).
    demand withdrawn -> both cmd FALSE.
    Wiring suggestion: feed fault_x from FB_MOTOR's fault or the electrical protection
    relay; feed run_fb_x from the run feedback (debounce a chattering contact with
    FB_DEBOUNCE); leave feedback-timeout fault decision to FB_MOTOR, this block does not
    duplicate the timing.

  Inputs:
    demand   BOOL  system needs a pump (level)
    alt_mode INT   0 = rotate each start / 1 = run-hour balancing
    run_fb_a BOOL  pump A run feedback
    run_fb_b BOOL  pump B run feedback
    fault_a  BOOL  pump A fault
    fault_b  BOOL  pump B fault
    hours_a  REAL  pump A accumulated hours (used by alt_mode=1, fed from FB_RUNTIME.hours)
    hours_b  REAL  pump B accumulated hours (used by alt_mode=1)
    dt_s     REAL  sample period s (this block does not currently time; kept for a uniform library interface)
  Outputs:
    cmd_a     BOOL  pump A start command
    cmd_b     BOOL  pump B start command
    duty_is_a BOOL  the current duty pump is A
*)
FUNCTION_BLOCK FB_DUTY2
  VAR_INPUT
    demand   : BOOL;
    alt_mode : INT := 0;
    run_fb_a : BOOL := FALSE;
    run_fb_b : BOOL := FALSE;
    fault_a  : BOOL := FALSE;
    fault_b  : BOOL := FALSE;
    hours_a  : REAL := 0.0;
    hours_b  : REAL := 0.0;
    dt_s     : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    cmd_a     : BOOL;
    cmd_b     : BOOL;
    duty_is_a : BOOL;
  END_VAR
  VAR
    prev_demand : BOOL; (* rising-edge memory *)
    rising      : BOOL;
  END_VAR

  rising := demand AND NOT prev_demand;
  prev_demand := demand;

  IF rising THEN
    IF run_fb_a AND NOT run_fb_b THEN
      duty_is_a := TRUE;  (* the already-running pump becomes duty directly, bumpless takeover *)
    ELSIF run_fb_b AND NOT run_fb_a THEN
      duty_is_a := FALSE;
    ELSIF alt_mode = 1 THEN
      duty_is_a := hours_a <= hours_b; (* hour balancing: the lower is duty *)
    ELSE
      duty_is_a := NOT duty_is_a; (* rotate each start (initial FALSE -> first start selects A) *)
    END_IF;
  END_IF;

  IF demand THEN
    (* Fault takeover: duty faults and the other is healthy -> duty switches immediately *)
    IF duty_is_a AND fault_a AND NOT fault_b THEN
      duty_is_a := FALSE;
    ELSIF NOT duty_is_a AND fault_b AND NOT fault_a THEN
      duty_is_a := TRUE;
    END_IF;
    cmd_a := duty_is_a AND NOT fault_a;
    cmd_b := NOT duty_is_a AND NOT fault_b;
  ELSE
    cmd_a := FALSE;
    cmd_b := FALSE;
  END_IF;
END_FUNCTION_BLOCK
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
(*
  FB_LIMIT — value limiter (high-low clamp + limit indication).

  ~ Standard limiter (limiter / high-low clamp), a staple of every major platform's standard library.

  Purpose: safe clamping of setpoint/output values, cascade outer-loop output limiting,
  operator-input validation, the last line of protection for a bare downstream actuator.
  Behavior (pure combinational logic, no internal state):
    out := u clamped to [lo, hi]; q_hi / q_lo indicate clamping in progress (u over the high/low limit).
    Configuration error lo > hi -> out := (lo + hi) / 2, with q_hi and q_lo both TRUE
    -- both limit flags lit at once cannot occur physically and serves purely as a
    configuration-error signal, easy for a supervisory alarm to catch.

  Inputs:
    u  REAL  input
    lo REAL  low limit
    hi REAL  high limit (assumes lo <= hi; reversed triggers the configuration-error signal)
  Outputs:
    out  REAL  clamped output
    q_hi BOOL  over high limit (clamping at hi)
    q_lo BOOL  over low limit (clamping at lo; TRUE together with q_hi = lo > hi configuration error)
*)
FUNCTION_BLOCK FB_LIMIT
  VAR_INPUT
    u  : REAL;
    lo : REAL := 0.0;
    hi : REAL := 100.0;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF lo > hi THEN
    (* Configuration error: output the midpoint, both limit flags lit as the error signal *)
    out := (lo + hi) / 2.0;
    q_hi := TRUE;
    q_lo := TRUE;
    RETURN;
  END_IF;

  IF u > hi THEN
    out := hi;
    q_hi := TRUE;
    q_lo := FALSE;
  ELSIF u < lo THEN
    out := lo;
    q_hi := FALSE;
    q_lo := TRUE;
  ELSE
    out := u;
    q_hi := FALSE;
    q_lo := FALSE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_STATS — running statistics: min/max/mean/standard deviation (Welford single-pass recurrence).

  ~ Standard statistics block (min/max/average/standard deviation), found in every
  major platform's standard library; this block uses Welford's single-pass
  recurrence -- no arrays, numerically stable.

  Purpose: quality-data statistics (batch temperature mean/spread), measurement
  noise-level assessment, commissioning-time signal profiling (feed FB_AVG_TIME for
  the time average, feed this block for the full-run distribution).
  Behavior (counted by "sample", not consuming dt_s: each scan with enable TRUE counts as 1 sample):
    Welford recurrence (scalar, single-pass, avoiding the precision disaster of
    subtracting squares of large numbers):
      n := n + 1; delta := u - avg; avg := avg + delta / n;
      m2 := m2 + delta * (u - avg)
    sd := SQRT(m2 / n) -- population standard deviation (divide by n, not n-1):
    process monitoring cares about the spread magnitude, the two are indistinguishable
    at large sample counts, and dividing by n is the most direct form in a single-pass
    recurrence.
    reset level-active: clears and re-seeds on the next enabled sample (vmin/vmax/avg
    take that sample's value).
    Note: n is a REAL count; in single precision, after about 1.6E7 samples (about 19
    days at a 100 ms sample) the +1 stops taking effect and the statistics freeze --
    for long-term running, reset per shift / per day.

  Inputs:
    u      REAL  input sample
    enable BOOL  sampling enable (a FALSE scan counts no sample, outputs hold)
    reset  BOOL  clear (level-active, takes priority; re-seeds on the next enabled sample)
  Outputs:
    vmin REAL  minimum value
    vmax REAL  maximum value
    avg  REAL  arithmetic mean
    sd   REAL  population standard deviation
    n    REAL  samples counted so far
*)
FUNCTION_BLOCK FB_STATS
  VAR_INPUT
    u      : REAL;
    enable : BOOL := TRUE;
    reset  : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    vmin : REAL;
    vmax : REAL;
    avg  : REAL;
    sd   : REAL;
    n    : REAL;
  END_VAR
  VAR
    m2     : REAL; (* Welford second-order accumulator sum of (u-avg)*(u-avg') *)
    delta  : REAL;
    delta2 : REAL;
  END_VAR

  IF reset THEN
    n := 0.0;
    avg := 0.0;
    m2 := 0.0;
    sd := 0.0;
    vmin := 0.0;
    vmax := 0.0;
    RETURN;
  END_IF;

  IF NOT enable THEN
    RETURN; (* counts no sample: outputs hold *)
  END_IF;

  IF n <= 0.0 THEN
    (* Seed on the first sample *)
    n := 1.0;
    avg := u;
    m2 := 0.0;
    sd := 0.0;
    vmin := u;
    vmax := u;
    RETURN;
  END_IF;

  n := n + 1.0;
  delta := u - avg;
  avg := avg + delta / n;
  delta2 := u - avg;
  m2 := m2 + delta * delta2;
  sd := SQRT(m2 / n);

  IF u < vmin THEN
    vmin := u;
  END_IF;
  IF u > vmax THEN
    vmax := u;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_AVG_TIME — tumbling-window time average.

  ~ Standard time-average block, the customary form for DCS reporting / steady-quantity monitoring.

  Purpose: full-window averages for reports (e.g. per-minute average flow), noise-robust
  readings of slowly varying quantities, drift monitoring by comparison against the
  instantaneous value.
  Behavior:
    Within the window, sum / count accumulate each scan; avg_run := sum / count is the
    running mean of the current window; when the window is full (accumulated time
    reaches window_s) avg_run is copied into avg, ready is set, and then the
    accumulators are cleared to start the next window (phase subtraction wraps so the
    long-term cadence does not drift).
    While the first window is not yet full, avg temporarily tracks avg_run (to avoid
    outputting 0 at startup) and ready = FALSE flags "not yet a complete-window result";
    after the first window completes ready stays TRUE and avg refreshes only on the
    window-full scan.
    reset level-active: clears the accumulators and ready, restarting the window.
    window_s <= 0 -> pass-through (avg := avg_run := u, ready := TRUE);
    dt_s <= 0 -> hold (no action).
    Note: this block is a tumbling window (one value per window), not a sliding window
    -- a true sliding window needs an array buffer (in this dialect an FB-scope array
    cannot be codegen'd, see README gotcha 7); for continuous "smoothed every scan"
    output, use FB_LAG exponential smoothing as an approximation.

  Inputs:
    u        REAL  input
    window_s REAL  averaging window length s (<= 0 pass-through)
    dt_s     REAL  sample period s
    reset    BOOL  reset (level-active: clear the window and restart)
  Outputs:
    avg     REAL  average of the previous complete window (tracks avg_run while the first window is incomplete)
    avg_run REAL  running mean of the current window
    ready   BOOL  first complete window has finished (avg is trustworthy)
*)
FUNCTION_BLOCK FB_AVG_TIME
  VAR_INPUT
    u        : REAL;
    window_s : REAL := 60.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    avg     : REAL;
    avg_run : REAL;
    ready   : BOOL;
  END_VAR
  VAR
    sum       : REAL; (* current-window accumulated sum *)
    count     : REAL; (* current-window sample count *)
    elapsed_s : REAL; (* current-window accumulated time, manual timing *)
  END_VAR

  IF reset THEN
    sum := 0.0;
    count := 0.0;
    elapsed_s := 0.0;
    avg := 0.0;
    avg_run := 0.0;
    ready := FALSE;
    RETURN;
  END_IF;

  IF window_s <= 0.0 THEN
    avg := u; (* invalid window length: pass-through *)
    avg_run := u;
    ready := TRUE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  sum := sum + u;
  count := count + 1.0;
  elapsed_s := elapsed_s + dt_s;
  avg_run := sum / count;

  IF elapsed_s >= window_s THEN
    (* Window full: copy the result, clear to start the next window (subtraction wrap keeps the long-term cadence) *)
    avg := avg_run;
    ready := TRUE;
    sum := 0.0;
    count := 0.0;
    elapsed_s := elapsed_s - window_s;
  ELSIF NOT ready THEN
    avg := avg_run; (* first window not yet full: avg tracks the running mean to avoid outputting 0 at startup *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_INTEG — general-purpose integrator (with hold/reset/preset and output clamp).

  ~ Standard integrator block, a staple of every major platform's standard library.

  Purpose: deviation integration (monitoring accumulated quantities such as degC*s),
  simple material/energy totals, the integral element in plant simulation, the
  integral path of a custom control algorithm.
  Difference from FB_TOTALIZER: TOTALIZER is a flow-totalizing block fixed at /3600;
  this block is general integration out := out + k * u * dt_s, with hold/preset/clamp.
  Behavior (command priority reset > preset > hold > integrate):
    reset  level-active: out := 0, limit flags cleared;
    preset level-active: out := preset_value (then clamped as usual);
    hold   level-active: integration paused, output held (clamp and limit flags still refresh);
    otherwise: out := out + k * u * dt_s, clamped to [out_min, out_max].
    q_hi / q_lo indicate the output at the high/low limit (anti-windup: at a limit, a
    reverse input exits immediately).
    dt_s <= 0 -> hold this scan (reset/preset are level commands, not subject to this).

  Inputs:
    u            REAL  integrand input
    k            REAL  integral gain (1/s; dimension defined by the caller)
    dt_s         REAL  sample period s
    hold         BOOL  pause integration (level-active)
    reset        BOOL  reset to 0 (level-active, highest priority)
    preset       BOOL  preset (level-active, takes priority over hold)
    preset_value REAL  preset value
    out_min      REAL  output low limit (default -1.0E38 ~ unlimited)
    out_max      REAL  output high limit (default +1.0E38 ~ unlimited)
  Outputs:
    out  REAL  integral output (clamped)
    q_hi BOOL  at high limit
    q_lo BOOL  at low limit
*)
FUNCTION_BLOCK FB_INTEG
  VAR_INPUT
    u            : REAL;
    k            : REAL := 1.0;
    dt_s         : REAL := 0.1;
    hold         : BOOL := FALSE;
    reset        : BOOL := FALSE;
    preset       : BOOL := FALSE;
    preset_value : REAL := 0.0;
    out_min      : REAL := -1.0E38;
    out_max      : REAL := 1.0E38;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF reset THEN
    out := 0.0;
    q_hi := FALSE;
    q_lo := FALSE;
    RETURN;
  END_IF;

  IF preset THEN
    out := preset_value; (* after preset, fall through to the clamp below as usual *)
  ELSIF NOT hold THEN
    IF dt_s <= 0.0 THEN
      RETURN; (* invalid sample period: hold *)
    END_IF;
    out := out + k * u * dt_s;
  END_IF;
  (* While hold is active: no integration, just refresh the clamp and limit flags as usual *)

  q_hi := FALSE;
  q_lo := FALSE;
  IF out >= out_max THEN
    out := out_max;
    q_hi := TRUE;
  ELSIF out <= out_min THEN
    out := out_min;
    q_lo := TRUE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_DERIV — filtered derivative (du/dt, filter first then difference).

  ~ Standard filtered-derivative block, a staple of every major platform's standard library.

  Purpose: quantifying trend (degC/s, %/s for display/feedforward/custom logic),
  derivative shaping in a feedforward path. Whenever you want "alarm on limit", use
  FB_ALARM_ROC directly (the same filter-difference core + delay/latch).
  Behavior:
    u first passes through an internal PT1 filter (time constant t_filt_s; <= 0 takes
    3*dt_s automatically, settling in about 4 scans), then a backward difference of
    the filtered value:
      u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f)
      out := (u_f - u_f_prev) / dt_s
    A single-scan difference is extremely sensitive to quantization/noise and must be
    filtered, or the output is all spikes -- for stronger noise rejection, increase
    t_filt_s or put FB_LAG ahead of u.
    reset / first scan: the filter state aligns to u, out := 0 (no spurious startup derivative).
    dt_s <= 0 -> hold (no action).

  Inputs:
    u        REAL  input
    t_filt_s REAL  derivative pre-filter time constant s (<= 0 = auto 3*dt_s)
    dt_s     REAL  sample period s
    reset    BOOL  reset/align (level-active: filter state follows u, out cleared)
  Outputs:
    out REAL  filtered rate of change units/s (signed)
*)
FUNCTION_BLOCK FB_DERIV
  VAR_INPUT
    u        : REAL;
    t_filt_s : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    u_f       : REAL; (* PT1-filtered input *)
    u_f_prev  : REAL; (* previous-scan filtered value, for the backward difference *)
    t_f       : REAL; (* filter time constant actually used *)
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  IF reset OR NOT init_done THEN
    u_f := u; (* align the filter state, no spurious startup derivative *)
    u_f_prev := u;
    out := 0.0;
    init_done := TRUE;
    RETURN;
  END_IF;

  t_f := t_filt_s;
  IF t_f <= 0.0 THEN
    t_f := 3.0 * dt_s; (* auto: the minimum noise rejection that settles in about 4 scans *)
  END_IF;

  u_f := u_f + dt_s / (t_f + dt_s) * (u - u_f);
  out := (u_f - u_f_prev) / dt_s;
  u_f_prev := u_f;
END_FUNCTION_BLOCK
(*
  FB_DEADTIME8 — dead time / transport delay, 8 scalar taps + linear interpolation.

  ~ Standard dead-time block (dead time / PTt / transport delay), the customary block
  for plant simulation and feedforward dynamic compensation (Smith-predictor-type structures).

  Purpose: plant simulation (in series with FB_LAG it forms a first-order-lag-plus-dead-time
  FOPDT plant), transport-delay matching in a feedforward path, time alignment of
  belt/pipe-conveying measurements.
  Behavior:
    Tap chain z1..z8 (z1 newest, z8 oldest), sampling interval = t_delay_s / 8:
    the phase phase_s accumulates by dt_s manually, and each time a sampling interval
    elapses the whole chain shifts once (z8<-z7<-...<-z1<-u); at most 8 shifts per scan
    (after 8 the whole chain has been filled by u, so the WHILE is bounded and
    scan-deterministic).
    The output interpolates linearly between the two oldest taps by the phase fraction:
      out := z8 + (z7 - z8) * phase_s / period_s
    -- a step input is traversed as a ramp over one sampling interval, so the output is
       continuous with no steps.
    Resolution = t_delay_s / 8: 8 taps is the compromise in an array-free dialect; the
    staircase/interpolation error of the delay time is about 1/8 of the delay --
    adequate for loop tuning; for finer resolution, put two of these blocks in series
    (each carrying half the delay).
    t_delay_s <= 0 -> pass-through (out := u), and the taps fill with u -- when the
    delay is put into service online it starts smoothly from the current value (bumpless).
    reset / first scan: all taps filled with u, out := u, phase cleared.
    dt_s <= 0 -> hold (no action).

  Inputs:
    u         REAL  input
    t_delay_s REAL  dead time s (<= 0 pass-through; changeable online, advances at the new interval)
    dt_s      REAL  sample period s
    reset     BOOL  reset (level-active: taps filled with u, output follows u)
  Outputs:
    out REAL  delayed output (linearly interpolated, continuous)
*)
FUNCTION_BLOCK FB_DEADTIME8
  VAR_INPUT
    u         : REAL;
    t_delay_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
    reset     : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    z1        : REAL; (* tap chain: z1 newest ... z8 oldest (scalar form for the array-free dialect) *)
    z2        : REAL;
    z3        : REAL;
    z4        : REAL;
    z5        : REAL;
    z6        : REAL;
    z7        : REAL;
    z8        : REAL;
    phase_s   : REAL; (* phase within the current sampling interval, manual timing *)
    period_s  : REAL; (* sampling interval = t_delay_s / 8 *)
    shifts    : REAL; (* shift count this scan (capped at 8: chain already saturated) *)
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF reset OR NOT init_done THEN
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_delay_s <= 0.0 THEN
    (* Pass-through; taps fill with u, so when the delay is put into service online it starts bumplessly from the current value *)
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  period_s := t_delay_s / 8.0;
  phase_s := phase_s + dt_s;

  (* Shift the whole chain once per elapsed sampling interval; capped at 8/scan (when
     dt_s spans several intervals, catch up; after 8 the chain is full of u and further
     shifts are meaningless) -- the WHILE is bounded *)
  shifts := 0.0;
  WHILE (phase_s >= period_s) AND (shifts < 8.0) DO
    z8 := z7;
    z7 := z6;
    z6 := z5;
    z5 := z4;
    z4 := z3;
    z3 := z2;
    z2 := z1;
    z1 := u;
    phase_s := phase_s - period_s;
    shifts := shifts + 1.0;
  END_WHILE;
  IF phase_s >= period_s THEN
    phase_s := 0.0; (* degenerate case dt_s >= t_delay_s: chain is already u, clear phase to prevent accumulation *)
  END_IF;

  (* Interpolate between the two oldest taps by phase fraction: at the shift instant frac=0 -> out=z8(old z7), continuous *)
  out := z8 + (z7 - z8) * phase_s / period_s;
END_FUNCTION_BLOCK
(*
  FB_MID3 — median select / 2oo3 analog voting.

  ~ Standard median-select / redundant-measurement voting block (median selector,
  2oo3 voting), the customary block for redundant-transmitter signal selection.

  Purpose: median of three redundant transmitters (rejects single-point drift/jump),
  average of dual redundancy, bad-value degradation of critical measurements (pair
  with the inverse of FB_SCALE fault as good_x).
  Behavior (degrades by the number of good channels n_good):
    3 good -> take the median (naturally rejects single-channel drift);
    2 good -> take the average of the two good channels;
    1 good -> pass that channel through;
    0 good -> out holds the previous scan's value, q_fail = TRUE (level, not latched).
    Deviation monitor: when dev_max > 0, the difference between any two good channels
    > dev_max -> q_dev (signals that drift between redundant channels has widened and
    a transmitter check is due; dev_max <= 0 disables the monitor).
    good_x takes the bad-value decision (transmitter open-circuit/over-range, e.g. the
    inverse of FB_SCALE.fault), defaulting to TRUE = all good.
    Note: when there are 0 good on the very first scan, out is its initial value 0 (no
    history to hold); the held value during q_fail is "the voted value from the last
    time a good channel existed".

  Inputs:
    a, b, c                REAL  three redundant measurements
    good_a, good_b, good_c BOOL  per-channel validity (default TRUE; FALSE = excluded)
    dev_max                REAL  deviation-monitor limit between good channels (<= 0 = disabled)
  Outputs:
    out    REAL  voted output (held when 0 good)
    n_good REAL  current number of good channels 0..3
    q_dev  BOOL  deviation between good channels over limit
    q_fail BOOL  all channels failed (output is in hold)
*)
FUNCTION_BLOCK FB_MID3
  VAR_INPUT
    a       : REAL;
    b       : REAL;
    c       : REAL;
    good_a  : BOOL := TRUE;
    good_b  : BOOL := TRUE;
    good_c  : BOOL := TRUE;
    dev_max : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    n_good : REAL;
    q_dev  : BOOL;
    q_fail : BOOL;
  END_VAR

  n_good := 0.0;
  IF good_a THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_b THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_c THEN
    n_good := n_good + 1.0;
  END_IF;

  q_fail := n_good <= 0.0;

  (* Deviation monitor: any two good channels differ by more than dev_max (inline IF instead of ABS, library convention) *)
  q_dev := FALSE;
  IF dev_max > 0.0 THEN
    IF good_a AND good_b THEN
      IF (a - b > dev_max) OR (b - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_a AND good_c THEN
      IF (a - c > dev_max) OR (c - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_b AND good_c THEN
      IF (b - c > dev_max) OR (c - b > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
  END_IF;

  IF n_good >= 3.0 THEN
    (* 3 good: take the median *)
    IF (a >= b AND a <= c) OR (a <= b AND a >= c) THEN
      out := a;
    ELSIF (b >= a AND b <= c) OR (b <= a AND b >= c) THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  ELSIF n_good >= 2.0 THEN
    (* 2 good: take the average *)
    IF NOT good_a THEN
      out := (b + c) / 2.0;
    ELSIF NOT good_b THEN
      out := (a + c) / 2.0;
    ELSE
      out := (a + b) / 2.0;
    END_IF;
  ELSIF n_good >= 1.0 THEN
    (* 1 good: pass-through *)
    IF good_a THEN
      out := a;
    ELSIF good_b THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  END_IF;
  (* 0 good: out holds the previous scan's value, q_fail signals it *)
END_FUNCTION_BLOCK
(*
  FB_CASCADE — cascade coupling glue (master output -> slave SP mapping + non-cascade tracking back-calculation).

  ~ Standard cascade-coupling block (cascade SP coupling / master tracking
  back-calculation), the customary glue for cascade-loop configuration: only this one
  block sits between the two PIDs.

  Purpose: cascade-loop configuration (temperature master -> flow slave, and the like),
  mapping the master output 0..100% to the slave SP engineering span, and supplying the
  master a tracking value when not in cascade so that putting CAS into/out of service is
  bumpless.
  Behavior:
    CAS (cas_on = TRUE):
      slave_sp := sp_min + master_out / 100 * (sp_max - sp_min)
      master_track := FALSE (the master controls normally).
    Non-CAS:
      slave_sp := slave_sp_local (the operator's local SP given directly to the slave loop);
      master_track := TRUE, master_track_value :=
        (slave_sp_now - sp_min) / (sp_max - sp_min) * 100, clamped 0..100
      -- the master output is forced to "the equivalent percentage of the current slave
      SP", so on the scan that switches back to CAS slave_sp does not jump (bumpless transfer).
      Span not configured (|sp_max - sp_min| < 1E-6) -> tracking value takes 0 (divide-by-zero guard).
    The tracking value is computed every scan (independent of mode), so it is valid the
    instant CAS is removed.
    Pure combinational logic, no internal state.

  Wiring (cascading two FB_PID instances; slave_sp_now is fed the previous scan's working SP):
    casc(master_out := pid_master.out, cas_on := cas_enable,
         sp_min := 0.0, sp_max := 2000.0,
         slave_sp_local := op_sp, slave_sp_now := slave_sp_used);
    slave_sp_used := casc.slave_sp;
    pid_master(pv := temp_pv, sp := temp_sp, ..., dt_s := scan_s,
               track := casc.master_track,
               track_value := casc.master_track_value);
    pid_slave(pv := flow_pv, sp := slave_sp_used, ..., dt_s := scan_s);

  Inputs:
    master_out     REAL  master output 0..100 %
    cas_on         BOOL  cascade enabled (FALSE = slave uses local SP, master tracks)
    sp_min         REAL  slave SP span low (engineering units)
    sp_max         REAL  slave SP span high
    slave_sp_local REAL  slave-loop local SP when not in cascade
    slave_sp_now   REAL  slave-loop current working SP (feed the previous scan's slave_sp, for the tracking back-calculation)
  Outputs:
    slave_sp           REAL  slave-loop setpoint (to pid_slave.sp)
    master_track       BOOL  master tracking command (to pid_master.track)
    master_track_value REAL  master tracking value 0..100 % (to pid_master.track_value)
*)
FUNCTION_BLOCK FB_CASCADE
  VAR_INPUT
    master_out     : REAL;
    cas_on         : BOOL := FALSE;
    sp_min         : REAL := 0.0;
    sp_max         : REAL := 100.0;
    slave_sp_local : REAL := 0.0;
    slave_sp_now   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    slave_sp           : REAL;
    master_track       : BOOL;
    master_track_value : REAL;
  END_VAR
  VAR
    span : REAL; (* slave-loop SP span width *)
  END_VAR

  span := sp_max - sp_min;

  (* Tracking value computed every scan: the equivalent percentage of the current slave SP (span not configured -> 0, divide-by-zero guard) *)
  IF (span < 0.000001) AND (span > -0.000001) THEN
    master_track_value := 0.0;
  ELSE
    master_track_value := (slave_sp_now - sp_min) / span * 100.0;
    IF master_track_value < 0.0 THEN
      master_track_value := 0.0;
    ELSIF master_track_value > 100.0 THEN
      master_track_value := 100.0;
    END_IF;
  END_IF;

  IF cas_on THEN
    slave_sp := sp_min + master_out / 100.0 * span;
    master_track := FALSE;
  ELSE
    slave_sp := slave_sp_local;
    master_track := TRUE;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_PV_HOLD — bad-PV hold / substitute.

  ~ Standard bad-value handling block (last-good hold / substitute value, including
  bad-value timeout degradation), the standard line of defense before a measurement
  enters a control loop; every major DCS/PLC input-processing chain has an equivalent
  mechanism.

  Purpose: on transmitter open-circuit/over-range (e.g. FB_SCALE.fault, or the inverse
  of FB_MID3.q_fail used as good), give the downstream loop a controlled PV: hold the
  last good value first to ride out chatter, then on timeout hold/substitute/follow as
  configured, while emitting q_bad/q_timeout for alarming and loop-to-manual logic.

  Behavior:
    good = TRUE: out := u, remember the last good value, clear q_bad/q_timeout, clear
    the bad-value timer.
    good = FALSE: q_bad := TRUE, accumulate the bad-value timer (manual dt_s):
      before timeout_s: out := last good value (short chatter rides through on hold);
      at timeout_s (timeout_s = 0 immediately): q_timeout := TRUE, then by mode:
        mode = 0: keep holding the last good value;
        mode = 1: out := sub_value (substitute value, e.g. a safe-side setting);
        mode = 2: out := u (follow the raw value -- just raise the alarm flag without
                  intervening, downstream handles it).
    When bad on the very first scan, the last good value takes the current u (no better history).
    dt_s <= 0 -> the bad-value timer does not advance (when timeout_s > 0, the hold segment is maintained).

  Inputs:
    u         REAL  input (raw PV)
    good      BOOL  input valid (FALSE = bad value; feed the inverse of a quality bit / open-circuit decision)
    mode      REAL  0 = hold last good value / 1 = substitute value / 2 = follow raw value
    sub_value REAL  substitute value (used by mode = 1)
    timeout_s REAL  bad-value persistence timeout s (0 = act by mode immediately; > 0 hold first)
    dt_s      REAL  sample period s
  Outputs:
    out       REAL  processed PV
    q_bad     BOOL  in bad value (level)
    q_timeout BOOL  bad value has timed out (mode in effect)
*)
FUNCTION_BLOCK FB_PV_HOLD
  VAR_INPUT
    u         : REAL;
    good      : BOOL := TRUE;
    mode      : REAL := 0.0;
    sub_value : REAL := 0.0;
    timeout_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out       : REAL;
    q_bad     : BOOL;
    q_timeout : BOOL;
  END_VAR
  VAR
    last_good : REAL; (* last good value *)
    bad_t     : REAL; (* bad-value accumulated persistence time s *)
    init_done : BOOL; (* first-scan seeding flag *)
  END_VAR

  IF NOT init_done THEN
    last_good := u; (* even when bad on the first scan, take the current u as the held value *)
    init_done := TRUE;
  END_IF;

  IF good THEN
    last_good := u;
    out := u;
    q_bad := FALSE;
    q_timeout := FALSE;
    bad_t := 0.0;
    RETURN;
  END_IF;

  q_bad := TRUE;

  (* Before timeout: hold the last good value (the segment that rides out short chatter) *)
  IF timeout_s > 0.0 THEN
    IF dt_s > 0.0 THEN
      bad_t := bad_t + dt_s;
    END_IF;
    IF bad_t < timeout_s THEN
      out := last_good;
      q_timeout := FALSE;
      RETURN;
    END_IF;
  END_IF;

  (* On timeout (or immediately when timeout_s = 0): act by mode *)
  q_timeout := TRUE;
  IF mode >= 1.5 THEN
    out := u;         (* 2 = follow raw value *)
  ELSIF mode >= 0.5 THEN
    out := sub_value; (* 1 = substitute value *)
  ELSE
    out := last_good; (* 0 = hold last good value *)
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_RAMPSOAK8 — 8-segment setpoint programmer (ramp-soak setpoint programmer).

  ~ Standard program-setpoint / curve-setpoint block (ramp-soak / setpoint programmer:
  heat-up curves, heat-treatment curves, batch temperature programs), found in every
  major DCS/PLC and program controller.

  Purpose: generate a setpoint from a segmented "ramp to target + soak" curve, feeding
  the sp input of FB_PID / FB_PID_POS: heat-treatment/curing/reactor heat-up programs,
  startup/shutdown load ramp curves. Segment parameters are scalar-expanded sp1..sp8 /
  ramp1_s..ramp8_s / soak1_s..soak8_s (in this dialect an FB-scope array cannot be
  codegen'd, see README gotcha 7; same as FB_CHAR).

  Behavior:
    Each segment N: sp ramps linearly from the previous segment's level (segment 1 from
    sp_init) over rampN_s to spN (rampN_s <= 0 steps), then soaks for soakN_s
    (soaking = TRUE; soakN_s <= 0 passes immediately). Segment parameters latch on the
    segment-entry scan -- changing parameters while running takes effect on the next segment.
    run level-active: TRUE starts and advances the program; FALSE -> return to the start
    (seg := 0, sp := sp_init, done cleared), and setting TRUE again restarts from segment 1.
    hold level-active: freeze sp and the in-segment timer (advance has no effect during hold).
    advance rising edge: jump to the next segment immediately (continue ramping from the
    current sp, curve continuous); advance in the last segment -> done (or cyclic wrap).
    On the last segment's soak end (n_seg clamped 2..8):
      cyclic = FALSE -> done := TRUE, sp pinned at the last segment's target, seg := 0,
      held until reset or run is withdrawn;
      cyclic = TRUE  -> wrap to segment 1 (continue ramping from the current sp, long-term cycling).
    reset level-active (priority): return to start, same as withdrawing run.
    dt_s <= 0 -> no in-segment advance (start/advance edges still handled).

  Inputs:
    sp1..sp8       REAL  per-segment target values (scalar-expanded)
    ramp1_s..ramp8_s REAL  per-segment ramp times s (<= 0 = step)
    soak1_s..soak8_s REAL  per-segment soak times s (<= 0 = no soak)
    n_seg          REAL  number of segments in use 2..8 (auto-clamped)
    run            BOOL  run (level; withdrawing returns to start)
    hold           BOOL  pause (level; freeze sp and the timer)
    advance        BOOL  jump segment (rising edge)
    reset          BOOL  reset (level, priority)
    cyclic         BOOL  TRUE = wrap to segment 1 and cycle after the last segment completes
    sp_init        REAL  start value (the launch level of the segment-1 ramp)
    dt_s           REAL  sample period s
  Outputs:
    sp      REAL  program setpoint output
    seg     REAL  current segment 1..8 (0 = not running / completed)
    soaking BOOL  soak segment in progress
    done    BOOL  program complete (non-cyclic; held until reset / run withdrawn)
*)
FUNCTION_BLOCK FB_RAMPSOAK8
  VAR_INPUT
    sp1     : REAL := 0.0;
    sp2     : REAL := 0.0;
    sp3     : REAL := 0.0;
    sp4     : REAL := 0.0;
    sp5     : REAL := 0.0;
    sp6     : REAL := 0.0;
    sp7     : REAL := 0.0;
    sp8     : REAL := 0.0;
    ramp1_s : REAL := 0.0;
    ramp2_s : REAL := 0.0;
    ramp3_s : REAL := 0.0;
    ramp4_s : REAL := 0.0;
    ramp5_s : REAL := 0.0;
    ramp6_s : REAL := 0.0;
    ramp7_s : REAL := 0.0;
    ramp8_s : REAL := 0.0;
    soak1_s : REAL := 0.0;
    soak2_s : REAL := 0.0;
    soak3_s : REAL := 0.0;
    soak4_s : REAL := 0.0;
    soak5_s : REAL := 0.0;
    soak6_s : REAL := 0.0;
    soak7_s : REAL := 0.0;
    soak8_s : REAL := 0.0;
    n_seg   : REAL := 2.0;
    run     : BOOL := FALSE;
    hold    : BOOL := FALSE;
    advance : BOOL := FALSE;
    reset   : BOOL := FALSE;
    cyclic  : BOOL := FALSE;
    sp_init : REAL := 0.0;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    sp      : REAL;
    seg     : REAL;
    soaking : BOOL;
    done    : BOOL;
  END_VAR
  VAR
    nn         : REAL; (* clamped segment count 2..8 *)
    elapsed_t  : REAL; (* in-segment phase timer s (ramp/soak each start from 0) *)
    level_from : REAL; (* this segment's ramp launch level *)
    seg_sp     : REAL; (* segment parameters latched on entry *)
    seg_ramp_s : REAL;
    seg_soak_s : REAL;
    enter_seg  : REAL; (* segment number to enter this scan (0 = no entry) *)
    go_next    : BOOL; (* segment complete / jump this scan *)
    adv_edge   : BOOL;
    prev_adv   : BOOL; (* advance rising-edge memory *)
  END_VAR

  IF reset THEN
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    elapsed_t := 0.0;
    prev_adv := advance;
    RETURN;
  END_IF;

  adv_edge := advance AND NOT prev_adv;
  prev_adv := advance;

  IF NOT run THEN
    (* Stopped: return to start; re-running starts from segment 1 *)
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    RETURN;
  END_IF;

  IF done THEN
    RETURN; (* Completion hold: sp pinned at the last segment's target, awaiting reset / run withdrawal *)
  END_IF;

  nn := n_seg;
  IF nn < 2.0 THEN
    nn := 2.0;
  ELSIF nn > 8.0 THEN
    nn := 8.0;
  END_IF;

  enter_seg := 0.0;
  go_next := FALSE;

  IF seg < 0.5 THEN
    (* Start: enter segment 1 from sp_init *)
    sp := sp_init;
    enter_seg := 1.0;
  ELSIF hold THEN
    RETURN; (* Freeze: sp and timer held (advance has no effect during hold) *)
  ELSIF adv_edge THEN
    go_next := TRUE; (* Jump segment: continue from the current sp *)
  ELSE
    IF dt_s <= 0.0 THEN
      RETURN; (* invalid sample period: no in-segment advance *)
    END_IF;
    elapsed_t := elapsed_t + dt_s;
    IF NOT soaking THEN
      (* Ramp phase *)
      IF seg_ramp_s <= 0.0 THEN
        sp := seg_sp; (* step segment *)
        soaking := TRUE;
        elapsed_t := 0.0;
      ELSIF elapsed_t >= seg_ramp_s THEN
        sp := seg_sp;
        soaking := TRUE;
        elapsed_t := elapsed_t - seg_ramp_s; (* carry the cross-phase remainder forward *)
      ELSE
        sp := level_from + (seg_sp - level_from) * elapsed_t / seg_ramp_s;
      END_IF;
    END_IF;
    IF soaking AND elapsed_t >= seg_soak_s THEN
      go_next := TRUE; (* soak ended *)
    END_IF;
  END_IF;

  (* Segment-advance decision: last segment -> complete/wrap; otherwise enter the next segment *)
  IF go_next THEN
    IF seg >= nn - 0.5 THEN
      IF cyclic THEN
        enter_seg := 1.0; (* Wrap: segment 1 continues ramping from the current sp *)
      ELSE
        done := TRUE;
        sp := seg_sp; (* Complete: pin at the last segment's target (an early jump also lands here directly) *)
        seg := 0.0;
        soaking := FALSE;
      END_IF;
    ELSE
      enter_seg := seg + 1.0;
    END_IF;
  END_IF;

  (* Segment entry: latch the segment parameters, ramp launches from the current sp *)
  IF enter_seg > 0.5 THEN
    seg := enter_seg;
    level_from := sp;
    elapsed_t := 0.0;
    soaking := FALSE;
    IF seg <= 1.5 THEN
      seg_sp := sp1;
      seg_ramp_s := ramp1_s;
      seg_soak_s := soak1_s;
    ELSIF seg <= 2.5 THEN
      seg_sp := sp2;
      seg_ramp_s := ramp2_s;
      seg_soak_s := soak2_s;
    ELSIF seg <= 3.5 THEN
      seg_sp := sp3;
      seg_ramp_s := ramp3_s;
      seg_soak_s := soak3_s;
    ELSIF seg <= 4.5 THEN
      seg_sp := sp4;
      seg_ramp_s := ramp4_s;
      seg_soak_s := soak4_s;
    ELSIF seg <= 5.5 THEN
      seg_sp := sp5;
      seg_ramp_s := ramp5_s;
      seg_soak_s := soak5_s;
    ELSIF seg <= 6.5 THEN
      seg_sp := sp6;
      seg_ramp_s := ramp6_s;
      seg_soak_s := soak6_s;
    ELSIF seg <= 7.5 THEN
      seg_sp := sp7;
      seg_ramp_s := ramp7_s;
      seg_soak_s := soak7_s;
    ELSE
      seg_sp := sp8;
      seg_ramp_s := ramp8_s;
      seg_soak_s := soak8_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_GAIN_SCHED3 — three-zone gain scheduling (gain scheduling, hysteresis zoning +
  optional zone-boundary linear transition).

  ~ Standard gain scheduler / parameter scheduler (switches the PID parameter set by
  zone of an operating variable), the customary block for segmented tuning of a
  nonlinear plant.

  Purpose: when the plant gain varies greatly with operating point (load segments,
  valve-characteristic segments, the strongly nonlinear pH segment), output a kp/ti/td
  parameter set in three zones of a scheduling variable (load / opening / the PV itself);
  the output feeds the parameter inputs of FB_PID / FB_PID_POS directly -- an online
  parameter change takes effect immediately (both blocks compute their increment/terms
  from the current parameters).

  Behavior:
    Zoning (u_sched against the two boundaries x12 / x23, assuming x12 < x23):
      zone 1: u <= x12; zone 2: x12 < u <= x23; zone 3: u > x23.
    Hysteresis: a zone change requires crossing the boundary +/- hyst (preventing
    chatter near a boundary from toggling parameters back and forth); the first scan
    seeds by plain comparison. A large step can jump two zones within one scan (it
    settles by stepwise migration).
    Parameter output:
      interp = FALSE: take the current zone's parameter set (hysteresis switching, with
      stepped parameters -- feeding incremental FB_PID is naturally bumpless, feeding
      FB_PID_POS recomputes the P term immediately at the new kp);
      interp = TRUE and hyst > 0: when u falls within +/-hyst of a boundary the two
      zones' parameters transition linearly (parameter output continuous, determined by
      u_sched alone; zone still gives its discrete indication by hysteresis).
    Note: when interp mixes a zone with ti = 0 (integral off) and one with ti > 0, the
    transition segment produces a very small ti (very strong integral) -- each zone's
    ti/td should be all 0 or all non-zero.

  Inputs:
    u_sched         REAL  scheduling variable (load / valve position / PV, etc.)
    x12             REAL  zone 1/2 boundary
    x23             REAL  zone 2/3 boundary (> x12, and the two transition bands must not overlap)
    hyst            REAL  zone-boundary hysteresis half-width (0 = no hysteresis)
    interp          BOOL  TRUE = linear parameter transition within +/-hyst of a boundary
    kp1, ti1_s, td1_s REAL  zone 1 parameter set
    kp2, ti2_s, td2_s REAL  zone 2 parameter set
    kp3, ti3_s, td3_s REAL  zone 3 parameter set
  Outputs:
    kp_out   REAL  scheduled proportional gain (to the PID's kp)
    ti_out_s REAL  scheduled integral time s
    td_out_s REAL  scheduled derivative time s
    zone     REAL  current zone 1 / 2 / 3 (hysteresis decision)
*)
FUNCTION_BLOCK FB_GAIN_SCHED3
  VAR_INPUT
    u_sched : REAL;
    x12     : REAL := 33.3;
    x23     : REAL := 66.6;
    hyst    : REAL := 0.0;
    interp  : BOOL := FALSE;
    kp1     : REAL := 1.0;
    ti1_s   : REAL := 0.0;
    td1_s   : REAL := 0.0;
    kp2     : REAL := 1.0;
    ti2_s   : REAL := 0.0;
    td2_s   : REAL := 0.0;
    kp3     : REAL := 1.0;
    ti3_s   : REAL := 0.0;
    td3_s   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    kp_out   : REAL;
    ti_out_s : REAL;
    td_out_s : REAL;
    zone     : REAL;
  END_VAR
  VAR
    f : REAL; (* interpolation fraction within the transition band 0..1 *)
  END_VAR

  (* 1) Zone selection (hysteresis): first scan zone = 0 -> seed by plain comparison *)
  IF zone < 0.5 THEN
    IF u_sched <= x12 THEN
      zone := 1.0;
    ELSIF u_sched <= x23 THEN
      zone := 2.0;
    ELSE
      zone := 3.0;
    END_IF;
  END_IF;

  (* Stepwise migration (test 1->2, 3->2 first, then 2->1/2->3: a large step settles in one scan) *)
  IF zone <= 1.5 THEN
    IF u_sched > x12 + hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 2.5 THEN
    IF u_sched < x23 - hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 1.5 AND zone <= 2.5 THEN
    IF u_sched > x23 + hyst THEN
      zone := 3.0;
    ELSIF u_sched < x12 - hyst THEN
      zone := 1.0;
    END_IF;
  END_IF;

  (* 2) Zone parameters (discrete values, hysteresis switching) *)
  IF zone <= 1.5 THEN
    kp_out   := kp1;
    ti_out_s := ti1_s;
    td_out_s := td1_s;
  ELSIF zone <= 2.5 THEN
    kp_out   := kp2;
    ti_out_s := ti2_s;
    td_out_s := td2_s;
  ELSE
    kp_out   := kp3;
    ti_out_s := ti3_s;
    td_out_s := td3_s;
  END_IF;

  (* 3) interp: linear transition within +/-hyst of a boundary (parameters continuous,
        determined by u_sched alone, overriding the discrete values above; the zone
        output is unaffected) *)
  IF interp AND hyst > 0.0 THEN
    IF (u_sched > x12 - hyst) AND (u_sched < x12 + hyst) THEN
      f := (u_sched - (x12 - hyst)) / (2.0 * hyst);
      kp_out   := kp1 + f * (kp2 - kp1);
      ti_out_s := ti1_s + f * (ti2_s - ti1_s);
      td_out_s := td1_s + f * (td2_s - td1_s);
    ELSIF (u_sched > x23 - hyst) AND (u_sched < x23 + hyst) THEN
      f := (u_sched - (x23 - hyst)) / (2.0 * hyst);
      kp_out   := kp2 + f * (kp3 - kp2);
      ti_out_s := ti2_s + f * (ti3_s - ti2_s);
      td_out_s := td2_s + f * (td3_s - td2_s);
    ELSIF u_sched <= x12 - hyst THEN
      kp_out   := kp1;
      ti_out_s := ti1_s;
      td_out_s := td1_s;
    ELSIF u_sched >= x23 + hyst THEN
      kp_out   := kp3;
      ti_out_s := ti3_s;
      td_out_s := td3_s;
    ELSE
      kp_out   := kp2; (* between the two transition bands: pure zone 2 *)
      ti_out_s := ti2_s;
      td_out_s := td2_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
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
(*
  FB_MANSTATION — manual loader / bias station.

  ~ Standard manual station + bias station (manual station / auto-manual station with
  bias), the standard operating layer between a controller output and an actuator,
  found in every major DCS/PLC standard library.

  Purpose: placed between a controller output and an actuator: in auto it passes through
  (with optional bias and rate limiting); in manual the operator sets the output
  directly; balance guarantees a bumpless manual->auto transfer. It can also serve
  standalone as a pure manual loader (auto := FALSE, always manual) or a pure bias /
  rate-limit station.

  Behavior:
    Auto (auto = TRUE): out approaches (auto_in + bias + corr) --
      when rate > 0 limited to rate units/second, rate = 0 steps directly; clamped to
      [out_min, out_max]. corr is the internal bias correction back-calculated at a balance transfer.
    Manual (auto = FALSE): out := manual_out (clamped, not rate-limited),
      tracking := TRUE.
    Bumpless transfer (balance = TRUE): the manual->auto transition edge back-calculates
      corr := out - auto_in - bias
    -- out is continuous at the transfer instant; afterward changes in auto_in/bias take
    effect as usual, and corr is held until recomputed at the next transfer. When
    balance = FALSE, corr is always 0 (the transfer step is ridden out by the rate
    limit; rate = 0 means a step).
    First scan: out aligns directly to the current mode's target value (no climb from 0).
    dt_s <= 0 -> hold this scan, no action.
    Note: bias is the nominal bias written by the operator/supervisory level; corr is
    the block's self-trimming correction, and the two add. To "clear" the historical
    correction, set balance := FALSE for one scan.

  Inputs:
    auto_in    REAL  auto input (upstream controller output)
    auto       BOOL  TRUE auto / FALSE manual
    manual_out REAL  manual output value
    bias       REAL  bias (added in auto)
    rate       REAL  auto-output rate limit units/s (0 = no rate limit)
    out_min    REAL  output low limit
    out_max    REAL  output high limit
    balance    BOOL  TRUE = back-calculate the bias correction on a manual->auto transfer (bumpless)
    dt_s       REAL  sample period s
  Outputs:
    out      REAL  station output (clamped in every mode)
    tracking BOOL  in manual (the upstream controller can use this bit for output tracking)
*)
FUNCTION_BLOCK FB_MANSTATION
  VAR_INPUT
    auto_in    : REAL;
    auto       : BOOL := TRUE;
    manual_out : REAL := 0.0;
    bias       : REAL := 0.0;
    rate       : REAL := 0.0;
    out_min    : REAL := 0.0;
    out_max    : REAL := 100.0;
    balance    : BOOL := TRUE;
    dt_s       : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out      : REAL;
    tracking : BOOL;
  END_VAR
  VAR
    corr      : REAL; (* internal bias correction back-calculated by balance *)
    target    : REAL; (* auto target = auto_in + bias + corr *)
    delta     : REAL;
    step      : REAL;
    prev_auto : BOOL; (* transfer-edge memory *)
    init_done : BOOL; (* first-scan alignment flag *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* invalid sample period: hold *)
  END_IF;

  IF NOT balance THEN
    corr := 0.0;
  END_IF;

  IF NOT init_done THEN
    (* First scan: align directly to the current mode's target, no climb from 0 *)
    IF auto THEN
      out := auto_in + bias;
    ELSE
      out := manual_out;
    END_IF;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := NOT auto;
    prev_auto := auto;
    init_done := TRUE;
    RETURN;
  END_IF;

  (* Manual->auto transition edge: balance back-calculates the correction so out is continuous *)
  IF auto AND NOT prev_auto AND balance THEN
    corr := out - auto_in - bias;
  END_IF;
  prev_auto := auto;

  IF NOT auto THEN
    (* Manual: set directly (clamped, not rate-limited) *)
    out := manual_out;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := TRUE;
    RETURN;
  END_IF;
  tracking := FALSE;

  (* Auto: approach auto_in + bias + corr at the rate limit (rate = 0 steps) *)
  target := auto_in + bias + corr;
  IF rate > 0.0 THEN
    delta := target - out;
    step := rate * dt_s;
    IF delta > step THEN
      out := out + step;
    ELSIF delta < 0.0 - step THEN
      out := out - step;
    ELSE
      out := target;
    END_IF;
  ELSE
    out := target;
  END_IF;

  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;
END_FUNCTION_BLOCK
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
(*
  FB_MOTOR_REV — bidirectional (forward/reverse) motor control, with reversal interlock,
  run-feedback monitoring and fault latch.

  ~ Standard reversing-motor block (reversing motor / forward-reverse starter, including
  the reversal-interlock delay), found in every major DCS/PLC standard library.

  Purpose: standard wrapper for reversible-running equipment such as agitators, hoists,
  bidirectional conveyors and gate-opening motors (pattern following FB_MOTOR: command
  latch + feedback-timeout fault latch).

  Behavior:
    1) Fault monitoring: fb_fwd and fb_rev TRUE at once -> feedback/contactor fault,
       immediate fault; an output-feedback mismatch (q_fwd<>fb_fwd or q_rev<>fb_rev)
       persisting for fb_timeout_s (manual dt_s accumulation) -> fault. All latched;
       cleared by reset with the dual-feedback condition cleared.
    2) Direction command latch (stop/fault priority): cmd_fwd / cmd_rev set the
       corresponding direction request (a both-direction command in the same scan is
       treated as a conflict, no action); stop or fault clears all requests.
    3) Reversal interlock: on a reversal (the reverse request is still present, or the
       reverse feedback is still turning), both outputs are forced fully stopped, and a
       new-direction output is allowed only after reversal_delay_s -- the contactor
       arc-quench / mechanical run-down time. reversal_delay_s should be less than
       fb_timeout_s (residual feedback rotation during the interlock wait is covered by
       the timeout tolerance).
    4) Status word state: 0 stopped / 1 forward / 2 reverse / 3 faulted / 4 reversal wait.

  Inputs:
    cmd_fwd          BOOL  forward command
    cmd_rev          BOOL  reverse command
    stop             BOOL  stop command (priority)
    fb_fwd           BOOL  forward run feedback
    fb_rev           BOOL  reverse run feedback
    fb_timeout_s     REAL  feedback-mismatch tolerance time s
    reversal_delay_s REAL  reversal full-stop interlock time s
    reset            BOOL  fault reset
    dt_s             REAL  sample period s
  Outputs:
    q_fwd BOOL  forward output
    q_rev BOOL  reverse output
    fault BOOL  fault (latched)
    state REAL  0 stopped / 1 forward / 2 reverse / 3 faulted / 4 reversal wait
*)
FUNCTION_BLOCK FB_MOTOR_REV
  VAR_INPUT
    cmd_fwd          : BOOL;
    cmd_rev          : BOOL;
    stop             : BOOL;
    fb_fwd           : BOOL;
    fb_rev           : BOOL;
    fb_timeout_s     : REAL := 5.0;
    reversal_delay_s : REAL := 2.0;
    reset            : BOOL;
    dt_s             : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_fwd : BOOL;
    q_rev : BOOL;
    fault : BOOL;
    state : REAL;
  END_VAR
  VAR
    want_fwd : BOOL; (* latched direction request *)
    want_rev : BOOL;
    wait_t   : REAL; (* reversal-interlock remaining time s *)
    mism_t   : REAL; (* feedback-mismatch accumulated time s *)
    conflict : BOOL; (* both-direction command in the same scan *)
  END_VAR

  (* 1) Fault monitoring *)
  IF fb_fwd AND fb_rev THEN
    fault := TRUE; (* both feedbacks active at once: welded contactor / feedback-circuit fault *)
  END_IF;
  IF (q_fwd <> fb_fwd) OR (q_rev <> fb_rev) THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT (fb_fwd AND fb_rev) THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) Direction command latch (stop/fault priority; a both-direction command in the same scan conflicts, no action) *)
  conflict := cmd_fwd AND cmd_rev;
  IF cmd_fwd AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_rev OR fb_rev THEN
      wait_t := reversal_delay_s; (* reverse still commanded / still turning: full-stop interlock first *)
    END_IF;
    want_fwd := TRUE;
    want_rev := FALSE;
  ELSIF cmd_rev AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_fwd OR fb_fwd THEN
      wait_t := reversal_delay_s;
    END_IF;
    want_rev := TRUE;
    want_fwd := FALSE;
  END_IF;
  IF stop OR fault THEN
    want_fwd := FALSE;
    want_rev := FALSE;
  END_IF;

  (* 3) Reversal-interlock timing (manual dt_s decrement) *)
  IF wait_t > 0.0 THEN
    wait_t := wait_t - dt_s;
    IF wait_t < 0.0 THEN
      wait_t := 0.0;
    END_IF;
  END_IF;

  (* 4) Output: during the interlock both outputs are forced fully stopped *)
  q_fwd := want_fwd AND wait_t <= 0.0;
  q_rev := want_rev AND wait_t <= 0.0;

  (* 5) Status word *)
  IF fault THEN
    state := 3.0;
  ELSIF wait_t > 0.0 AND (want_fwd OR want_rev) THEN
    state := 4.0;
  ELSIF q_fwd THEN
    state := 1.0;
  ELSIF q_rev THEN
    state := 2.0;
  ELSE
    state := 0.0;
  END_IF;
END_FUNCTION_BLOCK
(*
  FB_MOV — motor-operated valve (open/stop/close three commands + ZSO/ZSC dual limits + travel timeout).

  ~ Standard motor-operated-valve block (open/stop/close type, with travel monitoring),
  found in every major DCS/PLC standard library.

  Purpose: standard wrapper for "open-stop-close"-type motor-operated valves such as
  motorized gate/butterfly valves. Difference from FB_VALVE (two-position valve, single
  held output): this block is dual-direction pulse-driven with mid-travel stop, and the
  motor stops automatically at the limit; difference from FB_PID_3STEP (modulating step
  type): this block does only full-open/full-close/mid-stop sequential operations, no
  closed-loop control.

  Behavior:
    1) Fault monitoring: zso and zsc TRUE at once -> limit-switch fault, immediate fault;
       travel (an output active in either direction and the target limit not reached)
       exceeding travel_s -> travel-timeout fault (stuck valve / lost limit), and the
       outputs stop. All latched; cleared by reset with the dual-limit condition cleared
       (the travel timer is cleared in step).
    2) Commands: cmd_stop or fault -> both outputs stop (priority; cmd_open and cmd_close
       commanded together in the same scan is treated as a conflict, handled as stop);
       cmd_open latches the open output, cmd_close latches the close output (mutually
       exclusive, a new command is a new travel, the travel timer is cleared).
    3) Stop-at-position: zso reached during q_open -> stop the open output; zsc reached
       during q_close -> stop the close output.
    4) Status word pos_state: 0 fully closed / 1 opening / 2 fully open / 3 closing /
       4 faulted; a mid-stop (no output and not at a limit) holds the previous status value.

  Inputs:
    cmd_open  BOOL  open command
    cmd_close BOOL  close command
    cmd_stop  BOOL  stop command (priority, mid-stop)
    zso       BOOL  open limit
    zsc       BOOL  close limit
    travel_s  REAL  travel timeout s (<= 0 = not monitored)
    reset     BOOL  fault reset
    dt_s      REAL  sample period s
  Outputs:
    q_open    BOOL  open-direction output
    q_close   BOOL  close-direction output
    pos_state REAL  0 fully closed / 1 opening / 2 fully open / 3 closing / 4 faulted
    fault     BOOL  fault (latched)
*)
FUNCTION_BLOCK FB_MOV
  VAR_INPUT
    cmd_open  : BOOL;
    cmd_close : BOOL;
    cmd_stop  : BOOL;
    zso       : BOOL;
    zsc       : BOOL;
    travel_s  : REAL := 60.0;
    reset     : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_open    : BOOL;
    q_close   : BOOL;
    pos_state : REAL;
    fault     : BOOL;
  END_VAR
  VAR
    travel_t : REAL; (* current-travel accumulated time s *)
  END_VAR

  (* 1) Dual-limit fault (immediate) *)
  IF zso AND zsc THEN
    fault := TRUE; (* both limits active at once: limit-switch fault *)
  END_IF;

  (* Reset (no effect while the dual-limit condition is still present) *)
  IF reset AND NOT (zso AND zsc) THEN
    fault    := FALSE;
    travel_t := 0.0;
  END_IF;

  (* 2) Command latch: stop/fault/both-commands-same-scan -> full stop; open and close mutually exclusive, a new command is a new travel *)
  IF cmd_stop OR fault OR (cmd_open AND cmd_close) THEN
    q_open  := FALSE;
    q_close := FALSE;
  ELSIF cmd_open THEN
    IF NOT q_open THEN
      travel_t := 0.0; (* new travel: clear the timer *)
    END_IF;
    q_open  := TRUE;
    q_close := FALSE;
  ELSIF cmd_close THEN
    IF NOT q_close THEN
      travel_t := 0.0;
    END_IF;
    q_close := TRUE;
    q_open  := FALSE;
  END_IF;

  (* 3) Stop-at-position *)
  IF q_open AND zso THEN
    q_open := FALSE;
  END_IF;
  IF q_close AND zsc THEN
    q_close := FALSE;
  END_IF;

  (* 4) Travel timeout (manual dt_s accumulation) *)
  IF q_open OR q_close THEN
    travel_t := travel_t + dt_s;
    IF travel_s > 0.0 AND travel_t >= travel_s THEN
      fault   := TRUE; (* travel timeout: stuck valve / lost limit *)
      q_open  := FALSE;
      q_close := FALSE;
    END_IF;
  ELSE
    travel_t := 0.0;
  END_IF;

  (* 5) Status word (a mid-stop holds the previous status value) *)
  IF fault THEN
    pos_state := 4.0;
  ELSIF zso THEN
    pos_state := 2.0;
  ELSIF zsc THEN
    pos_state := 0.0;
  ELSIF q_open THEN
    pos_state := 1.0;
  ELSIF q_close THEN
    pos_state := 3.0;
  END_IF;
END_FUNCTION_BLOCK
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
(*
  FB_FLOW_COMP — gas-flow pressure/temperature compensation (ideal-gas approximation).

  ~ Standard pressure/temperature compensation block for gas flow, the customary
  block for correcting gas flow to standard / design conditions.

  Purpose: correct gas flow to reference (design) conditions using operating
  pressure/temperature: reading correction for a DP flowmeter calibrated at design
  conditions when operation departs from the design point, and correction of
  volumetric flow to standard conditions. Compensation ratio:
    ratio := (p_abs / p_ref) * (t_ref_k / t_k)   -- density ratio (ideal gas)
    p_abs := p_gauge_kpa + atm_kpa (gauge -> absolute)
    t_k   := t_c + 273.15
  Behavior (mode selects the flowmeter type):
    mode = 0 (square-root DP flowmeter, the flow signal after DP square-root extraction):
      q_comp := q_raw * SQRT(ratio)
      -- DP flow is proportional to SQRT(density), so a density departure from the
         design value is corrected via the square root.
    mode = 1 (linear volumetric, operating volumetric flow corrected to reference conditions):
      q_comp := q_raw * ratio
    Guard: t_k <= 0, t_ref_k <= 0, p_ref_kpa_abs <= 0 or p_abs <= 0
    (invalid measurement/configuration) -> q_comp := q_raw pass-through.
    Pure combinational logic, no internal state.
    Scope note: ideal-gas approximation (no compressibility-factor or humidity
    correction); custody-transfer-grade full-specification energy-metering standard
    algorithms are outside this block's scope.
    Reference conditions default to p_ref = 201.325 kPa(a) (i.e. 100 kPa gauge +
    standard atmosphere), t_ref = 20 degC -- set per the flowmeter design datasheet.

  Inputs:
    q_raw         REAL  raw flow (operating reading)
    p_gauge_kpa   REAL  operating gauge pressure kPa(g)
    t_c           REAL  operating temperature degC
    p_ref_kpa_abs REAL  reference absolute pressure kPa(a) (design condition)
    t_ref_c       REAL  reference temperature degC (design condition)
    atm_kpa       REAL  local atmospheric pressure kPa (gauge -> absolute conversion)
    mode          REAL  0 = square-root DP (multiply by SQRT(ratio)) /
                        1 = linear volumetric (multiply by ratio)
  Outputs:
    q_comp REAL  compensated flow
*)
FUNCTION_BLOCK FB_FLOW_COMP
  VAR_INPUT
    q_raw         : REAL;
    p_gauge_kpa   : REAL;
    t_c           : REAL;
    p_ref_kpa_abs : REAL := 201.325;
    t_ref_c       : REAL := 20.0;
    atm_kpa       : REAL := 101.325;
    mode          : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    q_comp : REAL;
  END_VAR
  VAR
    p_abs_kpa : REAL; (* operating absolute pressure *)
    t_k       : REAL; (* operating absolute temperature K *)
    t_ref_k   : REAL; (* reference absolute temperature K *)
    ratio     : REAL; (* density ratio (ideal gas) *)
  END_VAR

  p_abs_kpa := p_gauge_kpa + atm_kpa;
  t_k := t_c + 273.15;
  t_ref_k := t_ref_c + 273.15;

  IF (t_k <= 0.0) OR (t_ref_k <= 0.0)
     OR (p_ref_kpa_abs <= 0.0) OR (p_abs_kpa <= 0.0) THEN
    q_comp := q_raw; (* invalid measurement/configuration: pass-through *)
    RETURN;
  END_IF;

  ratio := (p_abs_kpa / p_ref_kpa_abs) * (t_ref_k / t_k);

  IF mode >= 0.5 THEN
    q_comp := q_raw * ratio;       (* 1 = linear volumetric *)
  ELSE
    q_comp := q_raw * SQRT(ratio); (* 0 = square-root DP *)
  END_IF;
END_FUNCTION_BLOCK
(* ---------------------------------------------------------------------------
   PROGRAM demo_main — wiring layer
--------------------------------------------------------------------------- *)
PROGRAM demo_main
  VAR
    (* -- field signals (bound via iomap; given initial values for the demo) -- *)
    lt0101_raw    : REAL := 13824.0; (* level transmitter raw counts (4-20mA -> 5530..27648) *)
    level_sp      : REAL := 60.0;    (* level setpoint % *)
    alm_ack       : BOOL := FALSE;   (* alarm acknowledge button *)
    hammer_enable : BOOL := TRUE;    (* air cannon in service *)
    gas_flow      : REAL := 1200.0;  (* inlet gas flow m3/h (leading flow FT0201) *)
    milk_flow     : REAL := 950.0;   (* lime-milk flow m3/h (follower pv, FT0202) *)
    milk_ratio    : REAL := 0.85;    (* lime-milk / inlet-gas ratio *)
    pump_run_fb_a : BOOL := FALSE;   (* lime-milk pump 1 run feedback *)
    pump_run_fb_b : BOOL := FALSE;   (* lime-milk pump 2 run feedback *)
    pump_fault_a  : BOOL := FALSE;   (* pump 1 fault (protection relay / thermal overload) *)
    pump_fault_b  : BOOL := FALSE;   (* pump 2 fault *)
    temp_hh       : BOOL := FALSE;   (* tower temperature high-high (external alarm bit) *)
    ilk_reset     : BOOL := FALSE;   (* interlock reset button *)
    rt_reset      : BOOL := FALSE;   (* run-hours clear (after maintenance) *)

    scan_s : REAL := 0.1; (* sample period s, must match tasks.toml interval_ms *)

    (* -- FB instances -- *)
    scale_lt      : FB_SCALE;
    pid_level     : FB_PID;
    alm_level     : FB_ALARM_HL;
    pulser_hammer : FB_PULSER;
    fill_ctl      : FB_HILO_FILL;
    ilk_tower     : FB_INTERLOCK8;
    rt_pump_a     : FB_RUNTIME;
    rt_pump_b     : FB_RUNTIME;
    duty_pumps    : FB_DUTY2;
    ratio_milk    : FB_RATIO;
    pid_milk      : FB_PID;
    split_milk    : FB_SPLIT_RANGE;

    (* -- output / intermediate variables (bound to AO/DO via iomap) -- *)
    level_pct   : REAL; (* tower level % *)
    lt_fault    : BOOL; (* transmitter open-circuit *)
    valve_pct   : REAL; (* feed control-valve opening % *)
    lv_any_alm  : BOOL; (* any level alarm *)
    hammer_q    : BOOL; (* air-cannon solenoid *)
    fill_open   : BOOL; (* make-up demand (high-close low-open) *)
    ilk_trip    : BOOL; (* tower interlock acted *)
    ilk_first   : INT;  (* interlock first-out channel *)
    pump_cmd_a  : BOOL; (* pump 1 start command *)
    pump_cmd_b  : BOOL; (* pump 2 start command *)
    duty_a      : BOOL; (* current duty pump is pump 1 *)
    milk_sp     : REAL; (* lime-milk flow setpoint after ratioing m3/h *)
    milk_dev    : REAL; (* lime-milk loop working deviation (after the SP ramp) *)
    valve_small : REAL; (* lime-milk small-valve opening % (split segment A, fine trim) *)
    valve_big   : REAL; (* lime-milk large-valve opening % (split segment B, range extension) *)

    (* -- math/dynamics block demo: steam-heating cascade + FOPDT plant simulation (sections 12~16) -- *)
    cas_enable     : BOOL := TRUE;  (* cascade-enable switch *)
    temp_sp        : REAL := 75.0;  (* tower-temperature setpoint degC (cascade master SP) *)
    steam_local_sp : REAL := 800.0; (* operator-given steam-flow local SP kg/h when not cascaded *)
    stats_reset    : BOOL := FALSE; (* statistics / deviation-integral clear button *)

    dly_obj      : FB_DEADTIME8; (* plant dead time 20 s *)
    mid_temp     : FB_MID3;      (* tower-temperature median of three (two good one bad) *)
    lim_temp     : FB_LIMIT;     (* voted tower-temperature limit 0..150 degC *)
    deriv_temp   : FB_DERIV;     (* tower-temperature rate of change *)
    casc_temp    : FB_CASCADE;   (* cascade coupling: master output -> steam SP *)
    pid_tc       : FB_PID;       (* cascade master loop: tower temperature *)
    pid_fc_steam : FB_PID;       (* cascade slave loop: steam flow *)
    integ_dev    : FB_INTEG;     (* master-loop deviation integral *)
    stats_temp   : FB_STATS;     (* tower-temperature full-run statistics *)
    avgt_temp    : FB_AVG_TIME;  (* tower-temperature 60 s tumbling-window average *)

    sim_state     : REAL := 40.0; (* FOPDT plant first-order state (manual PT1 discretization, T = 30 s) *)
    temp_a        : REAL;         (* tower-temperature transmitter 1 degC (plant-simulation output) *)
    temp_b        : REAL;         (* redundant point 2 degC (simulated +0.5 degC bias) *)
    temp_voted    : REAL;         (* tower temperature after voting + limiting degC *)
    temp_roc      : REAL;         (* tower-temperature rate of change degC/s *)
    steam_sp_used : REAL;         (* steam slave-loop working SP kg/h (fed back to slave_sp_now) *)
    steam_flow    : REAL;         (* steam flow pv kg/h (simulated static valve characteristic) *)
    steam_valve   : REAL;         (* steam-valve opening % (slave-loop output) *)
    temp_dev_int  : REAL;         (* tower-temperature deviation integral degC*s *)

    (* -- 0.3.0 extension block demo: program setpoint + positional PID chain, device layer,
          compensation (sections 17~26) -- *)
    rs_run       : BOOL := TRUE;  (* program setpoint run *)
    rs_hold      : BOOL := FALSE; (* program setpoint pause *)
    rs_advance   : BOOL := FALSE; (* program setpoint jump-segment button *)
    batch_reset  : BOOL := FALSE; (* program / batch reset button *)
    msta_auto    : BOOL := TRUE;  (* manual station auto in service *)
    msta_man_out : REAL := 0.0;   (* manual station manual output % *)
    agit_fwd_btn : BOOL := FALSE; (* agitator forward button *)
    agit_rev_btn : BOOL := FALSE; (* agitator reverse button *)
    dose_start   : BOOL := FALSE; (* dose start button *)
    dose_abort   : BOOL := FALSE; (* dose abort button *)

    hold_temp  : FB_PV_HOLD;     (* voted tower-temperature bad-value hold *)
    rs_temp    : FB_RAMPSOAK8;   (* heat-up/soak program setpoint *)
    gsch_temp  : FB_GAIN_SCHED3; (* three-zone gain scheduling *)
    pidp_temp  : FB_PID_POS;     (* positional tower-temperature PID (shadow loop) *)
    msta_temp  : FB_MANSTATION;  (* heating-output manual station *)
    pid3s_temp : FB_PID_3STEP;   (* step PID (motorized actuator, for observation) *)
    mrev_agit  : FB_MOTOR_REV;   (* bidirectional agitator motor *)
    mov_fill   : FB_MOV;         (* motorized make-up valve *)
    dose_lime  : FB_DOSE;        (* limestone batch dosing *)
    comp_gas   : FB_FLOW_COMP;   (* inlet-gas pressure/temperature compensation *)

    temp_held    : REAL;          (* tower temperature after bad-value hold degC *)
    rs_sp        : REAL;          (* program setpoint SP degC *)
    heat_out_pos : REAL;          (* positional PID output % *)
    heat_out     : REAL;          (* final heating output after the manual station % *)
    agit_fb_f    : BOOL;          (* agitator forward feedback (output-echo simulation) *)
    agit_fb_r    : BOOL;          (* agitator reverse feedback *)
    mov_sim_pos  : REAL := 0.0;   (* motorized-valve simulated travel 0..100 % *)
    mov_zso      : BOOL := FALSE; (* motorized-valve open limit (simulated) *)
    mov_zsc      : BOOL := TRUE;  (* motorized-valve close limit (simulated, initially fully closed) *)
    mov_q_open   : BOOL;          (* motorized-valve open output (local copy, gotcha 8) *)
    mov_q_close  : BOOL;          (* motorized-valve close output *)
    dose_flow    : REAL := 0.0;   (* dose simulated flow kg/s *)
    dose_q_coar  : BOOL;          (* coarse-feed valve (local copy, gotcha 8) *)
    dose_q_fine  : BOOL;          (* fine-feed valve *)
    gas_flow_std : REAL;          (* standard-condition inlet-gas flow after compensation m3/h *)
  END_VAR

  (* 1) Span conversion: raw counts -> level % *)
  scale_lt(raw := lt0101_raw, raw_min := 5530.0, raw_max := 27648.0,
           out_min := 0.0, out_max := 100.0, clamp := TRUE);
  level_pct := scale_lt.out;
  lt_fault  := scale_lt.fault;

  (* 2) Four-level level alarm, 2% deadband, 3 s delay *)
  alm_level(pv := level_pct, h_sp := 80.0, l_sp := 20.0,
            hh_sp := 90.0, ll_sp := 10.0,
            deadband := 2.0, delay_s := 3.0, dt_s := scan_s, ack := alm_ack);
  lv_any_alm := alm_level.any_alm;

  (* 3) Level PID (reverse-acting: high level -> close the feed valve); on transmitter
        open-circuit, switch to manual and drop to 0. Note: kept as a legacy-signature
        call -- verifies the backward compatibility after the FB_PID upgrade *)
  pid_level(pv := level_pct, sp := level_sp,
            kp := 2.0, ti_s := 120.0, td_s := 0.0, dt_s := scan_s,
            direct := FALSE, auto := NOT lt_fault, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0);
  valve_pct := pid_level.out;

  (* 4) Air cannon: a 2 s purge every 300 s *)
  pulser_hammer(enable := hammer_enable, period_s := 300.0,
                pulse_len_s := 2.0, dt_s := scan_s);
  hammer_q := pulser_hammer.q;

  (* 5) Make-up demand: below 15% needs water, above 85% stops *)
  fill_ctl(pv := level_pct, h_sp := 85.0, l_sp := 15.0);
  fill_open := fill_ctl.valve_open;

  (* 6) Tower interlock summary + first-out: 1=level HH 2=transmitter open-circuit
        3=tower-temp HH 4=dual-pump fault; 5..8 spare, bypassed *)
  ilk_tower(c1 := alm_level.hh_alm,
            c2 := lt_fault,
            c3 := temp_hh,
            c4 := pump_fault_a AND pump_fault_b,
            enable5 := FALSE, enable6 := FALSE,
            enable7 := FALSE, enable8 := FALSE,
            reset := ilk_reset);
  ilk_trip  := ilk_tower.tripped;
  ilk_first := ilk_tower.first_out;

  (* 7) Dual-pump run statistics (hours feed FB_DUTY2 hour balancing; instances may go in VAR RETAIN) *)
  rt_pump_a(running := pump_run_fb_a, reset := rt_reset, dt_s := scan_s);
  rt_pump_b(running := pump_run_fb_b, reset := rt_reset, dt_s := scan_s);

  (* 8) Dual make-up pumps duty/standby: select duty by hour balancing; on interlock action, withdraw demand and stop all *)
  duty_pumps(demand := fill_open AND NOT ilk_trip, alt_mode := 1,
             run_fb_a := pump_run_fb_a, run_fb_b := pump_run_fb_b,
             fault_a := pump_fault_a, fault_b := pump_fault_b,
             hours_a := rt_pump_a.hours, hours_b := rt_pump_b.hours,
             dt_s := scan_s);
  pump_cmd_a := duty_pumps.cmd_a;
  pump_cmd_b := duty_pumps.cmd_b;
  duty_a     := duty_pumps.duty_is_a;

  (* 9) Ratio station: lime-milk flow setpoint = inlet-gas flow x 0.85 (ratio clamped 0.5..1.2) *)
  ratio_milk(wild_flow := gas_flow, ratio := milk_ratio,
             ratio_min := 0.5, ratio_max := 1.2, bias := 0.0);
  milk_sp := ratio_milk.sp;

  (* 10) Lime-milk flow PID: inlet-gas static feedforward (gain 0.01 -> % opening),
         SP ramp 50 (m3/h)/s to avoid a ratio step, interlock action forces the valve
         closed via track *)
  pid_milk(pv := milk_flow, sp := milk_sp,
           kp := 0.8, ti_s := 30.0, td_s := 0.0, dt_s := scan_s,
           direct := FALSE, auto := TRUE, manual_out := 0.0,
           out_min := 0.0, out_max := 100.0,
           ff := gas_flow * 0.01,
           track := ilk_trip, track_value := 0.0,
           sp_rate := 50.0);
  milk_dev := pid_milk.dev;

  (* 11) Split range: PID output 0..50% drives the small valve for fine trim, 50..100%
         the large valve relays (sequential split range, no reversal) *)
  split_milk(u := pid_milk.out, split := 50.0, reverse_b := FALSE);
  valve_small := split_milk.out_a;
  valve_big   := split_milk.out_b;

  (* 12) FOPDT plant simulation: steam-valve opening (previous scan) -> FB_DEADTIME8 dead
         time 20 s -> manual PT1 (T = 30 s, same core as FB_LAG) -> tower temperature.
         Dead time + first-order lag, these two blocks form the most common
         simulation/tuning plant *)
  dly_obj(u := steam_valve, t_delay_s := 20.0, dt_s := scan_s);
  sim_state := sim_state + scan_s / (30.0 + scan_s) * (dly_obj.out - sim_state);
  temp_a := 40.0 + 0.5 * sim_state; (* 0..100 % valve opening -> 40..90 degC *)
  temp_b := temp_a + 0.5;           (* redundant point 2: demo +0.5 degC bias *)

  (* 13) Median of three: point 3's bad value is excluded (good_c := FALSE) -> two good,
         take the average; good channels differing by > 5 degC raise q_dev *)
  mid_temp(a := temp_a, b := temp_b, c := 0.0,
           good_a := TRUE, good_b := TRUE, good_c := FALSE,
           dev_max := 5.0);

  (* 14) Limit the voted value 0..150 degC + filtered derivative (rate of change degC/s for trend/feedforward) *)
  lim_temp(u := mid_temp.out, lo := 0.0, hi := 150.0);
  temp_voted := lim_temp.out;
  deriv_temp(u := temp_voted, t_filt_s := 2.0, dt_s := scan_s);
  temp_roc := deriv_temp.out;

  (* 15) Cascade: tower-temperature master PID (0..100 %) -> FB_CASCADE maps to steam-flow
         SP (0..2000 kg/h) -> steam-flow slave PID -> steam valve. When not cascaded
         (cas_enable = FALSE) the slave uses its local SP and the master tracks the
         equivalent percentage via track, so CAS transfer is bumpless. slave_sp_now is
         fed the previous scan's working SP (standard wiring) *)
  casc_temp(master_out := pid_tc.out, cas_on := cas_enable,
            sp_min := 0.0, sp_max := 2000.0,
            slave_sp_local := steam_local_sp, slave_sp_now := steam_sp_used);
  steam_sp_used := casc_temp.slave_sp;
  pid_tc(pv := temp_voted, sp := temp_sp,
         kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
         direct := FALSE, auto := TRUE, manual_out := 0.0,
         out_min := 0.0, out_max := 100.0,
         track := casc_temp.master_track,
         track_value := casc_temp.master_track_value);
  pid_fc_steam(pv := steam_flow, sp := steam_sp_used,
               kp := 0.5, ti_s := 10.0, td_s := 0.0, dt_s := scan_s,
               direct := FALSE, auto := TRUE, manual_out := 0.0,
               out_min := 0.0, out_max := 100.0);
  steam_valve := pid_fc_steam.out;
  steam_flow := steam_valve * 20.0; (* simulated static valve characteristic: 0..100 % -> 0..2000 kg/h *)

  (* 16) Shared-signal (voted tower temperature) statistics trio + deviation integral:
         full-run min/max/mean/standard deviation, 60 s tumbling-window average;
         master-loop deviation integral degC*s (hold on interlock action, clamped +/-1000) *)
  stats_temp(u := temp_voted, enable := TRUE, reset := stats_reset);
  avgt_temp(u := temp_voted, window_s := 60.0, dt_s := scan_s, reset := stats_reset);
  integ_dev(u := pid_tc.dev, k := 1.0, dt_s := scan_s,
            hold := ilk_trip, reset := stats_reset,
            preset := FALSE, preset_value := 0.0,
            out_min := -1000.0, out_max := 1000.0);
  temp_dev_int := integ_dev.out;

  (* 17) Bad-value hold: when the vote is all bad (q_fail), hold the last good value 5 s
         first, then keep holding (mode 0); q_bad/q_timeout feed alarming and loop-to-manual *)
  hold_temp(u := mid_temp.out, good := NOT mid_temp.q_fail,
            mode := 0.0, sub_value := 60.0, timeout_s := 5.0,
            dt_s := scan_s);
  temp_held := hold_temp.out;

  (* 18) 8-segment program setpoint (using 4 segments): a heat-up/soak/cool-down curve
         starting at 40 degC; hold pauses, advance jumps segment, batch_reset resets *)
  rs_temp(sp1 := 55.0, ramp1_s := 120.0, soak1_s := 60.0,
          sp2 := 70.0, ramp2_s := 180.0, soak2_s := 120.0,
          sp3 := 85.0, ramp3_s := 120.0, soak3_s := 300.0,
          sp4 := 40.0, ramp4_s := 240.0, soak4_s := 0.0,
          n_seg := 4.0, run := rs_run, hold := rs_hold,
          advance := rs_advance, reset := batch_reset,
          cyclic := FALSE, sp_init := 40.0, dt_s := scan_s);
  rs_sp := rs_temp.sp;

  (* 19) Three-zone gain scheduling: output PID parameters by zone of the held tower
         temperature (strong in the low-temp zone, weak in the high-temp zone), with a
         +/-2 degC linear transition at the boundaries (interp) *)
  gsch_temp(u_sched := temp_held, x12 := 55.0, x23 := 75.0,
            hyst := 2.0, interp := TRUE,
            kp1 := 2.0, ti1_s := 90.0, td1_s := 0.0,
            kp2 := 1.5, ti2_s := 60.0, td2_s := 0.0,
            kp3 := 1.0, ti3_s := 45.0, td3_s := 0.0);

  (* 20) Positional PID (shadow tower-temperature loop): program setpoint SP + bad-value-held
         PV + scheduled parameters, observable p/i/d; interlock forces 0 via track *)
  pidp_temp(pv := temp_held, sp := rs_sp,
            kp := gsch_temp.kp_out, ti_s := gsch_temp.ti_out_s,
            td_s := gsch_temp.td_out_s, t_dfilt_s := 2.0,
            sp_weight_b := 1.0, dt_s := scan_s,
            direct := FALSE, auto := TRUE, manual_out := 0.0,
            out_min := 0.0, out_max := 100.0,
            ff := 0.0, track := ilk_trip, track_value := 0.0);
  heat_out_pos := pidp_temp.out;

  (* 21) Manual station: in auto pass through the positional PID output (rate-limited
         20 %/s), in manual set directly; balance gives a bumpless return to auto *)
  msta_temp(auto_in := heat_out_pos, auto := msta_auto,
            manual_out := msta_man_out, bias := 0.0, rate := 20.0,
            out_min := 0.0, out_max := 100.0, balance := TRUE,
            dt_s := scan_s);
  heat_out := msta_temp.out;

  (* 22) Step PID (for observation): the same loop driving a motorized actuator without
         position feedback, full travel 60 s, minimum pulse 0.5 s, deadband 1 %, travel estimate pos_est *)
  pid3s_temp(pv := temp_held, sp := rs_sp,
             kp := 1.5, ti_s := 60.0, td_s := 0.0, dt_s := scan_s,
             motor_time_s := 60.0, min_pulse_s := 0.5,
             deadband_pct := 1.0, auto := TRUE,
             man_open := FALSE, man_close := FALSE, pos_fb := -1.0);

  (* 23) Bidirectional agitator motor: forward/reverse buttons, interlock stop has
         priority, reversal interlock 2 s; feedback uses output-echo simulation (one-scan
         lag, never times out) *)
  mrev_agit(cmd_fwd := agit_fwd_btn, cmd_rev := agit_rev_btn,
            stop := ilk_trip, fb_fwd := agit_fb_f, fb_rev := agit_fb_r,
            fb_timeout_s := 5.0, reversal_delay_s := 2.0,
            reset := ilk_reset, dt_s := scan_s);
  agit_fb_f := mrev_agit.q_fwd; (* simulated feedback: output echo (effective next scan) *)
  agit_fb_r := mrev_agit.q_rev;

  (* 24) Motorized make-up valve: make-up demand opens / withdrawal closes (interlock
         forces close), a 30 s simulated travel drives the dual limits; travel-timeout limit 60 s *)
  mov_fill(cmd_open := fill_open AND NOT ilk_trip,
           cmd_close := NOT fill_open OR ilk_trip,
           cmd_stop := FALSE, zso := mov_zso, zsc := mov_zsc,
           travel_s := 60.0, reset := ilk_reset, dt_s := scan_s);
  mov_q_open  := mov_fill.q_open;  (* copy to a local first, then use as an IF condition (gotcha 8) *)
  mov_q_close := mov_fill.q_close;
  IF mov_q_open THEN
    mov_sim_pos := mov_sim_pos + 100.0 * scan_s / 30.0; (* 30 s full-travel simulation *)
  ELSIF mov_q_close THEN
    mov_sim_pos := mov_sim_pos - 100.0 * scan_s / 30.0;
  END_IF;
  IF mov_sim_pos > 100.0 THEN
    mov_sim_pos := 100.0;
  ELSIF mov_sim_pos < 0.0 THEN
    mov_sim_pos := 0.0;
  END_IF;
  mov_zso := mov_sim_pos >= 100.0;
  mov_zsc := mov_sim_pos <= 0.0;

  (* 25) Batch dosing: target 100 kg, fine stage 15 kg, in-flight 2 kg; simulated flow
         coarse 8 / fine 1.5 kg/s (valve state echoed from the previous scan, with a one-scan lag) *)
  dose_lime(start := dose_start, abort := dose_abort,
            flow := dose_flow, target := 100.0,
            fine_qty := 15.0, inflight := 2.0,
            dt_s := scan_s, reset := batch_reset);
  dose_q_coar := dose_lime.q_coarse; (* copy to a local first, then use as an IF condition (gotcha 8) *)
  dose_q_fine := dose_lime.q_fine;
  dose_flow := 0.0;
  IF dose_q_coar THEN
    dose_flow := 8.0;
  ELSIF dose_q_fine THEN
    dose_flow := 1.5;
  END_IF;

  (* 26) Gas pressure/temperature compensation: correct the inlet-gas flow from operating
         150 kPa(g)/45 degC to design conditions (square-root DP, mode 0) -> standard-condition inlet-gas flow *)
  comp_gas(q_raw := gas_flow, p_gauge_kpa := 150.0, t_c := 45.0,
           p_ref_kpa_abs := 201.325, t_ref_c := 20.0,
           atm_kpa := 101.325, mode := 0.0);
  gas_flow_std := comp_gas.q_comp;
END_PROGRAM
