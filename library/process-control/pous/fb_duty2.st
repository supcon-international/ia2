(*
  FB_DUTY2 — 双泵值班/备用（duty/standby）轮换控制。

  ≈ 双泵一用一备（duty/standby）轮换：按启动次序或运行小时均衡选值班泵，故障即时顶替。

  用途：两台泵一用一备：每次启动轮换或按累计运行小时均衡选值班泵；
  值班泵故障备用泵立即顶上；双故障全停。
  行为（扫描确定性：无内部计时，仅沿记忆）：
    demand 上升沿选定本次值班泵，优先级：
      1) 恰有一台已在运转（run_fb 单边 TRUE）→ 直接采纳其为值班
         （无扰接管，避免停转再起备用泵）；
      2) alt_mode = 1 → 累计小时小者值班（hours_a/b 接两个 FB_RUNTIME）；
      3) alt_mode = 0（默认）→ 在当前值班位上翻转（含故障顶替后的状态，
         天然磨损均衡）。
    demand 保持期间：值班泵故障且另一台健康 → 值班立即切换（顶替）；
    被命令泵故障即不出指令；双故障 → cmd 双 FALSE（值班选择保持）。
    demand 撤销 → cmd 双 FALSE。
    接线建议：fault_x 接 FB_MOTOR 的 fault 或电气综保；run_fb_x 接运行
    反馈（抖动触点前置 FB_DEBOUNCE）；反馈超时判故障交给 FB_MOTOR，
    本块不重复计时。

  输入：
    demand   BOOL  系统需泵（电平）
    alt_mode INT   0 = 每次启动轮换 / 1 = 运行小时均衡
    run_fb_a BOOL  A 泵运行反馈
    run_fb_b BOOL  B 泵运行反馈
    fault_a  BOOL  A 泵故障
    fault_b  BOOL  B 泵故障
    hours_a  REAL  A 泵累计小时（alt_mode=1 用，接 FB_RUNTIME.hours）
    hours_b  REAL  B 泵累计小时（alt_mode=1 用）
    dt_s     REAL  采样周期 s（本块当前不计时，保留以统一库接口）
  输出：
    cmd_a     BOOL  A 泵启动指令
    cmd_b     BOOL  B 泵启动指令
    duty_is_a BOOL  当前值班泵为 A
*)
FUNCTION_BLOCK FB_DUTY2
  VAR_INPUT
    demand   : BOOL;
    alt_mode : INT := 0;
    run_fb_a : BOOL := FALSE;
    run_fb_b : BOOL := FALSE;
    fault_a  : BOOL := FALSE;
    fault_b  : BOOL := FALSE;
    hours_a  : REAL := 0.0;
    hours_b  : REAL := 0.0;
    dt_s     : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    cmd_a     : BOOL;
    cmd_b     : BOOL;
    duty_is_a : BOOL;
  END_VAR
  VAR
    prev_demand : BOOL; (* 上升沿记忆 *)
    rising      : BOOL;
  END_VAR

  rising := demand AND NOT prev_demand;
  prev_demand := demand;

  IF rising THEN
    IF run_fb_a AND NOT run_fb_b THEN
      duty_is_a := TRUE;  (* 已在转的泵直接作值班，无扰接管 *)
    ELSIF run_fb_b AND NOT run_fb_a THEN
      duty_is_a := FALSE;
    ELSIF alt_mode = 1 THEN
      duty_is_a := hours_a <= hours_b; (* 小时均衡：少者值班 *)
    ELSE
      duty_is_a := NOT duty_is_a; (* 每次启动轮换（初值 FALSE → 首启选 A） *)
    END_IF;
  END_IF;

  IF demand THEN
    (* 故障顶替：值班故障且另一台健康 → 值班立即切换 *)
    IF duty_is_a AND fault_a AND NOT fault_b THEN
      duty_is_a := FALSE;
    ELSIF NOT duty_is_a AND fault_b AND NOT fault_a THEN
      duty_is_a := TRUE;
    END_IF;
    cmd_a := duty_is_a AND NOT fault_a;
    cmd_b := NOT duty_is_a AND NOT fault_b;
  ELSE
    cmd_a := FALSE;
    cmd_b := FALSE;
  END_IF;
END_FUNCTION_BLOCK
