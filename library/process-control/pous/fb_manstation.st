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
