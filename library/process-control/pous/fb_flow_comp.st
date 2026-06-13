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
