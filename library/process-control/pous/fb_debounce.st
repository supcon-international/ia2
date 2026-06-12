(*
  FB_DEBOUNCE — 开关量去抖（双向独立确认延时）。

  ≈ 开关量去抖（on/off 双延时），DI 处理标配。

  用途：浮球液位开关、限位开关、继电器/接触器辅助触点等抖动信号确认；
  亦可给 FB_DUTY2 的 run_fb/fault 输入前置去抖。
  行为：
    u 连续为 TRUE 满 t_on_s → q := TRUE；u 连续为 FALSE 满 t_off_s →
    q := FALSE；其间任何反向跳变将对应方向计时清零（重新计）。
    t = 0 表示该方向立即生效。实际确认时间按 dt_s 拍向上取整。
    （手工 dt_s 累加计时，等效 TON 接通确认 + 断开确认，离线可测。）

  输入：
    u       BOOL  原始开关量
    t_on_s  REAL  置位确认时间 s（0 = 立即）
    t_off_s REAL  复位确认时间 s（0 = 立即）
    dt_s    REAL  采样周期 s
  输出：
    q       BOOL  去抖后的开关量
*)
FUNCTION_BLOCK FB_DEBOUNCE
  VAR_INPUT
    u       : BOOL;
    t_on_s  : REAL := 0.1;
    t_off_s : REAL := 0.1;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    on_t  : REAL; (* 置位方向累计 *)
    off_t : REAL; (* 复位方向累计 *)
  END_VAR

  IF u THEN
    off_t := 0.0;
    IF NOT q THEN
      on_t := on_t + dt_s;
      IF on_t >= t_on_s THEN
        q := TRUE;
      END_IF;
    END_IF;
  ELSE
    on_t := 0.0;
    IF q THEN
      off_t := off_t + dt_s;
      IF off_t >= t_off_s THEN
        q := FALSE;
      END_IF;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
