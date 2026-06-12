(*
  FB_ALARM_HL — 四级高低报警（H/L/HH/LL），带死区、延时与简化 ISA-18.2 确认锁存。

  用途：液位/温度/压力等模拟量的 高/低/高高/低低 报警。
  例：碳化塔液位 LT 报警，HH 联锁停进料。

  行为（每级相同，独立计时）：
    1) 原始越限判定（带死区回差）：
       高类：pv > sp 置位；pv < sp - deadband 复位。
       低类：pv < sp 置位；pv > sp + deadband 复位。
    2) 延时：越限条件连续保持 delay_s 后才触发（内部用 dt_s 手工累加计时，
       不使用 TON 实例，便于离线测试与移植）。
    3) 锁存（简化 ISA-18.2）：报警输出 = 触发条件 OR (已锁存 AND NOT ack)。
       即条件存在时报警常在；条件消失后仍保持，直到 ack 确认才清除。

  输入：
    pv        REAL  过程值
    h_sp      REAL  高报设定
    l_sp      REAL  低报设定
    hh_sp     REAL  高高报设定
    ll_sp     REAL  低低报设定
    deadband  REAL  死区（回差，工程单位）
    delay_s   REAL  报警延时 s（0 = 立即）
    dt_s      REAL  采样周期 s
    ack       BOOL  确认（电平有效）
  输出：
    h_alm / l_alm / hh_alm / ll_alm  BOOL  锁存后的报警输出
    any_alm                          BOOL  任一报警
    h_cond / l_cond / hh_cond / ll_cond BOOL 原始越限位（含死区，未延时未锁存）
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
    h_t     : REAL; (* 各级延时累加器，手工计时替代 TON *)
    l_t     : REAL;
    hh_t    : REAL;
    ll_t    : REAL;
    h_trip  : BOOL;
    l_trip  : BOOL;
    hh_trip : BOOL;
    ll_trip : BOOL;
  END_VAR

  (* ---- 高报 H ---- *)
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

  (* ---- 低报 L ---- *)
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

  (* ---- 高高报 HH ---- *)
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

  (* ---- 低低报 LL ---- *)
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
