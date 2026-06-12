(*
  FB_FLOW_COMP — 气体流量温压补偿（理想气体近似）。

  ≈ 标准温压补偿块（pressure/temperature compensation for gas
  flow），气体流量折标况/折设计条件的惯用块。

  用途：按工况压力/温度把气体流量修正到参比（设计）条件：
  设计条件标定的 DP 流量计在工况偏离设计点时的读数修正、
  体积式流量折标况量。补偿比：
    ratio := (p_abs / p_ref) * (t_ref_k / t_k)   —— 密度比（理想气体）
    p_abs := p_gauge_kpa + atm_kpa（表压 → 绝压）
    t_k   := t_c + 273.15
  行为（按 mode 选流量计类型）：
    mode = 0（开方式 DP 流量计，差压开方后的流量信号）：
      q_comp := q_raw * SQRT(ratio)
      ——DP 流量 ∝ SQRT(密度)，密度偏离设计值按开方修正。
    mode = 1（线性体积式，工况体积流量折参比条件）：
      q_comp := q_raw * ratio
    防护：t_k <= 0、t_ref_k <= 0、p_ref_kpa_abs <= 0 或 p_abs <= 0
    （测量/组态非法）→ q_comp := q_raw 直通。
    纯组合逻辑，无内部状态。
    范围说明：理想气体近似（不含压缩因子/湿度修正）；贸易交接级
    的全规格能量计量标准算法不在本块范围。
    参比条件默认 p_ref = 201.325 kPa(a)（即 100 kPa 表压 + 标准
    大气压）、t_ref = 20 ℃——按流量计设计数据单整定。

  输入：
    q_raw         REAL  原始流量（工况读数）
    p_gauge_kpa   REAL  工况表压 kPa(g)
    t_c           REAL  工况温度 ℃
    p_ref_kpa_abs REAL  参比绝压 kPa(a)（设计条件）
    t_ref_c       REAL  参比温度 ℃（设计条件）
    atm_kpa       REAL  当地大气压 kPa（表压→绝压换算）
    mode          REAL  0 = 开方式 DP（乘 SQRT(ratio)）/
                        1 = 线性体积式（乘 ratio）
  输出：
    q_comp REAL  补偿后的流量
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
    p_abs_kpa : REAL; (* 工况绝压 *)
    t_k       : REAL; (* 工况热力学温度 K *)
    t_ref_k   : REAL; (* 参比热力学温度 K *)
    ratio     : REAL; (* 密度比（理想气体） *)
  END_VAR

  p_abs_kpa := p_gauge_kpa + atm_kpa;
  t_k := t_c + 273.15;
  t_ref_k := t_ref_c + 273.15;

  IF (t_k <= 0.0) OR (t_ref_k <= 0.0)
     OR (p_ref_kpa_abs <= 0.0) OR (p_abs_kpa <= 0.0) THEN
    q_comp := q_raw; (* 测量/组态非法：直通 *)
    RETURN;
  END_IF;

  ratio := (p_abs_kpa / p_ref_kpa_abs) * (t_ref_k / t_k);

  IF mode >= 0.5 THEN
    q_comp := q_raw * ratio;       (* 1 = 线性体积式 *)
  ELSE
    q_comp := q_raw * SQRT(ratio); (* 0 = 开方式 DP *)
  END_IF;
END_FUNCTION_BLOCK
