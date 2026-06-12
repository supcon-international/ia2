(*
  FB_MANSTATION — 手操/偏置站（manual loader / bias station）。

  ≈ 标准手操站 + 偏置站（manual station / auto-manual station with
  bias），调节输出与执行机构之间的标准操作层，各主流 DCS/PLC
  标准库均有同类块。

  用途：放在调节器输出与执行机构之间：自动时透传（可加偏置、可限
  速），手动时操作员直接给定输出；balance 保证手动→自动切换无扰。
  也可单独作纯手操器（auto := FALSE 常手动）或纯偏置/限速站。

  行为：
    自动（auto = TRUE）：out 朝 (auto_in + bias + corr) 逼近——
      rate > 0 时按 rate 单位/秒限速，rate = 0 直接跳变；钳位到
      [out_min, out_max]。corr 是 balance 切换时回算的内部偏置修正。
    手动（auto = FALSE）：out := manual_out（钳位，不限速），
      tracking := TRUE。
    无扰切换（balance = TRUE）：手动→自动切换沿回算
      corr := out - auto_in - bias
    ——切换瞬间 out 连续；之后 auto_in/bias 的变化照常生效，corr
    保持到下一次切换再重算。balance = FALSE 时 corr 恒 0（切换差值
    靠 rate 限速渡过；rate = 0 则跳变）。
    首扫描：out 直接对中到当前模式的目标值（不从 0 爬升）。
    dt_s <= 0 → 本周期保持不动作。
    注意：bias 是操作员/上位写的名义偏置；corr 是块内自整定修正，
    两者叠加。要"清掉"历史修正：置 balance := FALSE 一拍即可。

  输入：
    auto_in    REAL  自动输入（上游调节器输出）
    auto       BOOL  TRUE 自动 / FALSE 手动
    manual_out REAL  手动给定值
    bias       REAL  偏置（自动时叠加）
    rate       REAL  自动输出限速 单位/s（0 = 不限速）
    out_min    REAL  输出下限
    out_max    REAL  输出上限
    balance    BOOL  TRUE = 手动→自动切换回算偏置修正（无扰）
    dt_s       REAL  采样周期 s
  输出：
    out      REAL  站输出（各模式均钳位）
    tracking BOOL  手动中（上游调节器可接此位做输出跟踪）
*)
FUNCTION_BLOCK FB_MANSTATION
  VAR_INPUT
    auto_in    : REAL;
    auto       : BOOL := TRUE;
    manual_out : REAL := 0.0;
    bias       : REAL := 0.0;
    rate       : REAL := 0.0;
    out_min    : REAL := 0.0;
    out_max    : REAL := 100.0;
    balance    : BOOL := TRUE;
    dt_s       : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out      : REAL;
    tracking : BOOL;
  END_VAR
  VAR
    corr      : REAL; (* balance 回算的内部偏置修正 *)
    target    : REAL; (* 自动目标 = auto_in + bias + corr *)
    delta     : REAL;
    step      : REAL;
    prev_auto : BOOL; (* 切换沿记忆 *)
    init_done : BOOL; (* 首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  IF NOT balance THEN
    corr := 0.0;
  END_IF;

  IF NOT init_done THEN
    (* 首扫描：直接对中当前模式目标，不从 0 爬升 *)
    IF auto THEN
      out := auto_in + bias;
    ELSE
      out := manual_out;
    END_IF;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := NOT auto;
    prev_auto := auto;
    init_done := TRUE;
    RETURN;
  END_IF;

  (* 手动→自动切换沿：balance 回算修正，使 out 连续 *)
  IF auto AND NOT prev_auto AND balance THEN
    corr := out - auto_in - bias;
  END_IF;
  prev_auto := auto;

  IF NOT auto THEN
    (* 手动：直接给定（钳位，不限速） *)
    out := manual_out;
    IF out > out_max THEN
      out := out_max;
    ELSIF out < out_min THEN
      out := out_min;
    END_IF;
    tracking := TRUE;
    RETURN;
  END_IF;
  tracking := FALSE;

  (* 自动：朝 auto_in + bias + corr 限速逼近（rate = 0 跳变） *)
  target := auto_in + bias + corr;
  IF rate > 0.0 THEN
    delta := target - out;
    step := rate * dt_s;
    IF delta > step THEN
      out := out + step;
    ELSIF delta < 0.0 - step THEN
      out := out - step;
    ELSE
      out := target;
    END_IF;
  ELSE
    out := target;
  END_IF;

  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;
END_FUNCTION_BLOCK
