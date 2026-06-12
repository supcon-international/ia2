(*
  FB_PWM — 时间比例输出（0..100% 指令 → 固定周期占空比通断）。

  ≈ 时间比例输出（time-proportioning / PWM）：连续量 → 周期通断，固态继电器/电磁阀温控惯用。

  用途：PID 输出驱动固态继电器/电磁阀的通断式执行机构：电加热器、
  蒸汽通断阀、加药泵。
  行为：
    相位累加器同 FB_PULSER：phase_s 以 dt_s 累加，到 period_s 减法回卷
    （长期节拍不漂移）；q := phase_s < on_s。
    on_s := u/100 * period_s（u 先钳位 0..100）。
    最小通/断时间（保护接触器/SSR/阀件，按有效占空比钳位实现）：
      0 < on_s < min_on_s            → on_s 提升至 min_on_s；
      0 < period_s - on_s < min_off_s → on_s 压缩至 period_s - min_off_s
      （断侧优先级更高；若 min_on_s + min_off_s > period_s 属参数矛盾，
      实际向断侧让步，极端时恒断——勿如此整定）。
    u <= 0 → 整周期断；u >= 100 → 整周期通（满幅不受最小通断钳位影响）。
    NOT enable 或 period_s/dt_s <= 0 → q := FALSE 且相位清零
    （重新使能从周期起点开始，先通后断）。
    u 变化即时生效（周期内比较门限即变，无需等下个周期）。

  输入：
    u         REAL  指令 0..100 %
    period_s  REAL  PWM 周期 s
    min_on_s  REAL  最小导通时间 s（0 = 不限）
    min_off_s REAL  最小关断时间 s（0 = 不限）
    enable    BOOL  使能
    dt_s      REAL  采样周期 s
  输出：
    q         BOOL  时间比例输出
*)
FUNCTION_BLOCK FB_PWM
  VAR_INPUT
    u         : REAL;
    period_s  : REAL := 10.0;
    min_on_s  : REAL := 0.0;
    min_off_s : REAL := 0.0;
    enable    : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q : BOOL;
  END_VAR
  VAR
    phase_s : REAL; (* 周期内相位，手工计时替代 TP/TON *)
    uc      : REAL;
    on_s    : REAL; (* 本拍有效导通时长 *)
    off_s   : REAL;
  END_VAR

  IF NOT enable OR period_s <= 0.0 OR dt_s <= 0.0 THEN
    phase_s := 0.0;
    q := FALSE;
    RETURN;
  END_IF;

  uc := u;
  IF uc < 0.0 THEN
    uc := 0.0;
  ELSIF uc > 100.0 THEN
    uc := 100.0;
  END_IF;

  on_s := uc * period_s / 100.0;

  (* 最小导通：有效占空比上调（u=0 不受影响） *)
  IF on_s > 0.0 AND on_s < min_on_s THEN
    on_s := min_on_s;
  END_IF;

  (* 最小关断：有效占空比下调（u=100 时 off=0 不触发，保持恒通） *)
  off_s := period_s - on_s;
  IF off_s > 0.0 AND off_s < min_off_s THEN
    on_s := period_s - min_off_s;
    IF on_s < 0.0 THEN
      on_s := 0.0;
    END_IF;
  END_IF;

  q := phase_s < on_s;

  phase_s := phase_s + dt_s;
  IF phase_s >= period_s THEN
    phase_s := phase_s - period_s; (* 减法回卷，保持长期节拍 *)
  END_IF;
END_FUNCTION_BLOCK
