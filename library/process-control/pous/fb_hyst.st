(*
  FB_HYST — 双向迟滞（滞环）开关。

  用途：模拟量带回差的通断控制，避免临界点抖动。
  例：分级机电流控制给料机——电流高于 on_sp 停给料、低于 off_sp 恢复
  （低位接通方向）；或温度高于 on_sp 开风机、低于 off_sp 停（高位接通方向）。

  行为（按设定点自动判向）：
    on_sp > off_sp（高位接通）：pv >= on_sp 置位；pv <= off_sp 复位；之间保持。
    on_sp < off_sp（低位接通）：pv <= on_sp 置位；pv >= off_sp 复位；之间保持。
    on_sp = off_sp（退化）   ：out := pv >= on_sp（普通比较器，无回差）。

  输入：
    pv     REAL  过程值
    on_sp  REAL  接通设定点
    off_sp REAL  断开设定点
  输出：
    out    BOOL  开关输出
*)
FUNCTION_BLOCK FB_HYST
  VAR_INPUT
    pv     : REAL;
    on_sp  : REAL;
    off_sp : REAL;
  END_VAR
  VAR_OUTPUT
    out : BOOL;
  END_VAR

  IF on_sp > off_sp THEN
    (* 高位接通 *)
    IF pv >= on_sp THEN
      out := TRUE;
    ELSIF pv <= off_sp THEN
      out := FALSE;
    END_IF;
  ELSIF on_sp < off_sp THEN
    (* 低位接通 *)
    IF pv <= on_sp THEN
      out := TRUE;
    ELSIF pv >= off_sp THEN
      out := FALSE;
    END_IF;
  ELSE
    (* 设定点相等：退化为普通比较器 *)
    out := pv >= on_sp;
  END_IF;
END_FUNCTION_BLOCK
