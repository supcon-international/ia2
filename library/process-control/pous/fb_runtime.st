(*
  FB_RUNTIME — 设备运行小时 + 启动次数统计。

  ≈ 设备运行统计（累计运行小时 + 启动次数），维护保养数据标配。

  用途：泵/风机磨损均衡（hours 接 FB_DUTY2 的 hours_a/b，配 alt_mode=1）、
  维保到期提醒、启停频度统计。
  行为：
    running 为 TRUE 期间 hours := hours + dt_s/3600；
    running 上升沿 starts := starts + 1；
    reset 电平有效，两者清零（优先，复位周期不累计）。
    实例可由调用方放 VAR RETAIN 块下断电保持（同 FB_TOTALIZER 注意事项：
    IA2 retain 按 i32 存取，REAL 小时数的保持精度受此限制；starts 为
    整数计数不受影响）。

  输入：
    running BOOL  设备运行中（接运行反馈，必要时前置 FB_DEBOUNCE）
    reset   BOOL  清零（电平有效，优先；检修换泵后用）
    dt_s    REAL  采样周期 s
  输出：
    hours  REAL  累计运行小时
    starts DINT  累计启动次数
*)
FUNCTION_BLOCK FB_RUNTIME
  VAR_INPUT
    running : BOOL;
    reset   : BOOL := FALSE;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    hours  : REAL;
    starts : DINT;
  END_VAR
  VAR
    prev_run : BOOL; (* 上升沿记忆 *)
  END_VAR

  IF reset THEN
    hours := 0.0;
    starts := 0;
  ELSE
    IF running AND NOT prev_run THEN
      starts := starts + 1;
    END_IF;
    IF running AND dt_s > 0.0 THEN
      hours := hours + dt_s / 3600.0;
    END_IF;
  END_IF;
  prev_run := running;
END_FUNCTION_BLOCK
