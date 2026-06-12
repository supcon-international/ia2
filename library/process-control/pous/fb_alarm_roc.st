(*
  FB_ALARM_ROC — 变化率报警（rate-of-change），带导数滤波、延时与确认锁存。

  ≈ 变化率报警（rate-of-change alarm），导数经一阶滤波去噪。

  用途：温度飞升、压力骤变、液位暴跌（泄漏/管破）等异常变化速度监视。
  行为：
    原始导数 raw_roc := (pv - pv_prev) / dt_s；
    roc 为 raw_roc 经内部 PT1 滤波（时间常数固定 3*dt_s，约 4 拍收敛）
    ——单拍差分对量化/噪声极敏感，不滤波必误报；需要更强去噪时
    给 pv 前置 FB_LAG。
    |roc| > roc_limit 连续保持 delay_s 后触发；锁存至 ack
    （同 FB_ALARM_HL 范式：条件在则常报，消失后保持到确认）。
    roc_limit <= 0 → 监视关闭（触发条件恒 FALSE；已锁存输出仍须 ack 清）。
    首扫描只对中 pv_prev、不判报（避免投运假导数）；dt_s <= 0 → 保持。

  输入：
    pv        REAL  过程值
    roc_limit REAL  变化率限值 |单位|/s（双向）
    delay_s   REAL  报警延时 s（0 = 立即）
    dt_s      REAL  采样周期 s
    ack       BOOL  确认（电平有效）
  输出：
    alm BOOL  变化率报警（锁存）
    roc REAL  滤波后的变化率 单位/s（带符号，可用于趋势显示）
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
    t_f       : REAL; (* 导数滤波时间常数 = 3*dt_s *)
    cond      : BOOL;
    trip      : BOOL;
    alm_t     : REAL; (* 延时累加器 *)
    init_done : BOOL;
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 保持 *)
  END_IF;

  IF NOT init_done THEN
    pv_prev := pv; (* 首扫描对中，不判报 *)
    roc := 0.0;
    init_done := TRUE;
    RETURN;
  END_IF;

  raw_roc := (pv - pv_prev) / dt_s;
  pv_prev := pv;

  (* 内部 PT1 去噪：t_f = 3*dt_s 固定（每拍吸收 1/4 新值） *)
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
