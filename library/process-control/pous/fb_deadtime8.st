(*
  FB_DEADTIME8 — 纯滞后（dead time / transport delay），8 标量抽头 + 线性插值。

  ≈ 标准纯滞后块（dead time / PTt / transport delay），对象仿真与
  前馈动态补偿（Smith 预估类结构）的惯用块。

  用途：对象仿真（与 FB_LAG 串联即组成一阶惯性+纯滞后 FOPDT 对象）、
  前馈通路的传输延迟匹配、皮带/管道输送类测量的时间对齐。
  行为：
    抽头链 z1..z8（z1 最新、z8 最老），抽样周期 = t_delay_s / 8：
    相位 phase_s 以 dt_s 手工累加，每经过一个抽样周期整链移位一次
    （z8←z7←…←z1←u）；一拍最多移 8 次（8 次后整链已被 u 充满，
    WHILE 有界、扫描确定性）。
    输出在最老两个抽头间按相位分数线性插值：
      out := z8 + (z7 - z8) * phase_s / period_s
    ——阶跃输入以一个抽样周期的斜坡渡过，输出连续无台阶。
    分辨率 = t_delay_s / 8：8 抽头是无数组方言下的折衷，延迟时间的
    阶梯/插值误差约为延迟的 1/8——回路整定够用；要更细分辨率请串联
    两级本块（各承担一半延迟）。
    t_delay_s <= 0 → 直通（out := u），抽头随 u 充满——延迟在线投用
    的那一刻从当前值平滑起步（无扰）。
    reset / 首扫描：全部抽头充 u，out := u，相位清零。
    dt_s <= 0 → 保持不动作。

  输入：
    u         REAL  输入
    t_delay_s REAL  纯滞后时间 s（<= 0 直通；在线可改，按新周期推进）
    dt_s      REAL  采样周期 s
    reset     BOOL  复位（电平有效：抽头充 u，输出跟随 u）
  输出：
    out REAL  延迟后的输出（线性插值，连续）
*)
FUNCTION_BLOCK FB_DEADTIME8
  VAR_INPUT
    u         : REAL;
    t_delay_s : REAL := 0.0;
    dt_s      : REAL := 0.1;
    reset     : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  VAR
    z1        : REAL; (* 抽头链：z1 最新 … z8 最老（无数组方言的标量形式） *)
    z2        : REAL;
    z3        : REAL;
    z4        : REAL;
    z5        : REAL;
    z6        : REAL;
    z7        : REAL;
    z8        : REAL;
    phase_s   : REAL; (* 当前抽样周期内相位，手工计时 *)
    period_s  : REAL; (* 抽样周期 = t_delay_s / 8 *)
    shifts    : REAL; (* 本拍移位计数（封顶 8：整链已饱和） *)
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF reset OR NOT init_done THEN
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    init_done := TRUE;
    RETURN;
  END_IF;

  IF t_delay_s <= 0.0 THEN
    (* 直通；抽头随 u 充满，延迟在线投用时从当前值无扰起步 *)
    z1 := u;
    z2 := u;
    z3 := u;
    z4 := u;
    z5 := u;
    z6 := u;
    z7 := u;
    z8 := u;
    phase_s := 0.0;
    out := u;
    RETURN;
  END_IF;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  period_s := t_delay_s / 8.0;
  phase_s := phase_s + dt_s;

  (* 每经过一个抽样周期整链移位一次；封顶 8 次/拍（dt_s 跨多个周期时
     补移，8 次后整链已被 u 充满，再移无意义）——WHILE 有界 *)
  shifts := 0.0;
  WHILE (phase_s >= period_s) AND (shifts < 8.0) DO
    z8 := z7;
    z7 := z6;
    z6 := z5;
    z5 := z4;
    z4 := z3;
    z3 := z2;
    z2 := z1;
    z1 := u;
    phase_s := phase_s - period_s;
    shifts := shifts + 1.0;
  END_WHILE;
  IF phase_s >= period_s THEN
    phase_s := 0.0; (* 退化情形 dt_s >= t_delay_s：整链已是 u，相位清零防积累 *)
  END_IF;

  (* 最老两抽头间按相位分数插值：移位瞬间 frac=0 → out=z8(旧 z7)，连续 *)
  out := z8 + (z7 - z8) * phase_s / period_s;
END_FUNCTION_BLOCK
