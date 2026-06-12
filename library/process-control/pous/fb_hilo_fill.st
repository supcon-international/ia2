(*
  FB_HILO_FILL — 高位关、低位开的水箱/料仓补给控制。

  用途：液位低于低位开补给阀，高于高位关阀，之间保持当前状态
  （等效于低位接通方向的迟滞开关，专名封装便于组态阅读）。
  例：循环水箱补水：pv <= l_sp 开补水阀，pv >= h_sp 关阀。

  行为：
    pv <= l_sp → valve_open := TRUE（低位开）
    pv >= h_sp → valve_open := FALSE（高位关）
    l_sp < pv < h_sp → 保持。
  假定 l_sp < h_sp；若设反则高位判断优先（先开后即被关，输出为 FALSE）。

  输入：
    pv   REAL  液位/料位过程值
    h_sp REAL  高位设定（关阀）
    l_sp REAL  低位设定（开阀）
  输出：
    valve_open BOOL  补给阀输出（TRUE 开）
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
    valve_open := FALSE; (* 高位关，优先 *)
  ELSIF pv <= l_sp THEN
    valve_open := TRUE;  (* 低位开 *)
  END_IF;
  (* 区间内保持 *)
END_FUNCTION_BLOCK
