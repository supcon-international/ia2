(*
  FB_AVG_TIME — 翻转窗时间平均（tumbling-window time average）。

  ≈ 标准定时平均块（time average），DCS 报表/平稳量监视的惯用形态。

  用途：报表用整窗平均（如每分钟平均流量）、慢变量的抗噪取值、
  与瞬时值比较做漂移监视。
  行为：
    窗内逐拍累加 sum / count；avg_run := sum / count 为当前窗的滚动均值；
    窗满（累计时间达 window_s）时把 avg_run 拷入 avg、置 ready，随后
    清空累加器开下一窗（相位减法回卷，长期节拍不漂移）。
    首窗未满期间 avg 暂跟 avg_run（避免投运期输出 0），ready = FALSE
    标示"尚非完整窗结果"；首窗完成后 ready 恒 TRUE，avg 仅在窗满拍刷新。
    reset 电平有效：清空累加器与 ready，重新开窗。
    window_s <= 0 → 直通（avg := avg_run := u，ready := TRUE）；
    dt_s <= 0 → 保持不动作。
    注意：本块是翻转窗（每窗出一个值），不是滑动窗——真滑动窗需要
    数组缓存（本方言 FB 作用域数组不能 codegen，见 README 坑 7）；
    要"每拍都平滑"的连续输出请用 FB_LAG 指数平滑近似。

  输入：
    u        REAL  输入
    window_s REAL  平均窗长 s（<= 0 直通）
    dt_s     REAL  采样周期 s
    reset    BOOL  复位（电平有效：清窗重开）
  输出：
    avg     REAL  上一完整窗的平均（首窗未满暂跟 avg_run）
    avg_run REAL  当前窗的滚动均值
    ready   BOOL  首个完整窗已完成（avg 可信）
*)
FUNCTION_BLOCK FB_AVG_TIME
  VAR_INPUT
    u        : REAL;
    window_s : REAL := 60.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    avg     : REAL;
    avg_run : REAL;
    ready   : BOOL;
  END_VAR
  VAR
    sum       : REAL; (* 当前窗累加和 *)
    count     : REAL; (* 当前窗样本数 *)
    elapsed_s : REAL; (* 当前窗累计时间，手工计时 *)
  END_VAR

  IF reset THEN
    sum := 0.0;
    count := 0.0;
    elapsed_s := 0.0;
    avg := 0.0;
    avg_run := 0.0;
    ready := FALSE;
    RETURN;
  END_IF;

  IF window_s <= 0.0 THEN
    avg := u; (* 窗长非法：直通 *)
    avg_run := u;
    ready := TRUE;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  sum := sum + u;
  count := count + 1.0;
  elapsed_s := elapsed_s + dt_s;
  avg_run := sum / count;

  IF elapsed_s >= window_s THEN
    (* 窗满：拷贝结果，清空开下一窗（减法回卷保持长期节拍） *)
    avg := avg_run;
    ready := TRUE;
    sum := 0.0;
    count := 0.0;
    elapsed_s := elapsed_s - window_s;
  ELSIF NOT ready THEN
    avg := avg_run; (* 首窗未满：avg 暂跟滚动均值，避免投运期输出 0 *)
  END_IF;
END_FUNCTION_BLOCK
