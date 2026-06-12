(*
  FB_LAG — 一阶惯性滤波（PT1 / 一阶滞后）。

  ≈ 标准一阶惯性滤波（PT1 / LAG），各主流平台标准库必备。

  用途：模拟量去噪、测量平滑、微分/变化率运算的前置滤波。
  算法（后向欧拉离散，无条件稳定）：
    out := out + dt_s / (t_s + dt_s) * (u - out)
  说明：
    t_s <= 0 → 直通（out := u）；dt_s <= 0 → 保持不动作；
    reset 电平有效 → out := u（滤波器对中，无扰投运）；
    首个扫描周期自动对中（out := u），避免从 0 缓慢爬升的投运冲击。

  输入：
    u     REAL  输入
    t_s   REAL  滤波时间常数 s（0 = 直通）
    dt_s  REAL  采样周期 s
    reset BOOL  复位/对中（电平有效：out 跟随 u）
  输出：
    out   REAL  滤波输出
*)
FUNCTION_BLOCK FB_LAG
  VAR_INPUT
    u     : REAL;
    t_s   : REAL := 1.0;
    dt_s  : REAL := 0.1;
    reset : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF reset OR NOT init_done THEN
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_s <= 0.0 THEN
    out := u; (* 直通 *)
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  out := out + dt_s / (t_s + dt_s) * (u - out);
END_FUNCTION_BLOCK
