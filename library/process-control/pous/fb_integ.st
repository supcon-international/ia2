(*
  FB_INTEG — 通用积分器（带保持/复位/置数与输出钳位）。

  ≈ 标准积分块（integrator），各主流平台标准库必备。

  用途：偏差积分（℃·s 之类的累计量监视）、简单物料/能量累计、
  对象仿真中的积分环节、自定义控制算法的积分通路。
  与 FB_TOTALIZER 的区别：TOTALIZER 是固定 /3600 的流量累积专用块；
  本块为通用积分 out := out + k * u * dt_s，带 hold/preset/钳位。
  行为（命令优先级 reset > preset > hold > 积分）：
    reset  电平有效：out := 0，越限位清零；
    preset 电平有效：out := preset_value（随后照常钳位）；
    hold   电平有效：暂停积分，输出保持（钳位与越限位照常刷新）；
    其余：out := out + k * u * dt_s，钳位到 [out_min, out_max]。
    q_hi / q_lo 指示输出贴上/下限（抗饱和：贴限后反向输入立即退出）。
    dt_s <= 0 → 本拍保持不动作（reset/preset 为电平命令，不受此限）。

  输入：
    u            REAL  被积输入
    k            REAL  积分增益（1/s 量纲由调用方定义）
    dt_s         REAL  采样周期 s
    hold         BOOL  暂停积分（电平有效）
    reset        BOOL  复位到 0（电平有效，最高优先）
    preset       BOOL  置数（电平有效，优先于 hold）
    preset_value REAL  置数值
    out_min      REAL  输出下限（默认 -1.0E38 ≈ 不限）
    out_max      REAL  输出上限（默认 +1.0E38 ≈ 不限）
  输出：
    out  REAL  积分输出（钳位）
    q_hi BOOL  贴上限
    q_lo BOOL  贴下限
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
    out := preset_value; (* 置数后照常走下方钳位 *)
  ELSIF NOT hold THEN
    IF dt_s <= 0.0 THEN
      RETURN; (* 采样周期非法：保持 *)
    END_IF;
    out := out + k * u * dt_s;
  END_IF;
  (* hold 电平期间：不积分，仅照常刷新钳位与越限位 *)

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
