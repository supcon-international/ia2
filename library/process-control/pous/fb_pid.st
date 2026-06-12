(*
  FB_PID — 增量式（速度型）PID 调节器，带抗积分饱和、无扰切换、前馈、
  输出跟踪与设定值斜坡。

  ≈ 通用 PID 必备形态：SP 斜坡 / 输出跟踪 / 前馈 / 手自动无扰（各主流 DCS/PLC 标准库均有同类块）。

  典型用法：流量 PID 驱动变频泵，正作用，输出限幅即变频上下限（如 10~45Hz
  ——流量作 pv，变频器频率给定作 out：out_min:=10.0, out_max:=45.0,
    direct := TRUE 正作用——pv 升高输出升高）。

  算法（增量式 + 前馈）：
    sp_int := sp 经 sp_rate 限速斜坡（sp_rate <= 0 直接取 sp；首扫描对中）
    e      := sp_int - pv（反作用，默认）；direct=TRUE 时 e := pv - sp_int
    Δpid   := kp*(e - e_prev)                          —— 比例增量
            + kp*dt_s/ti_s * e（ti_s>0 时）            —— 积分增量
            + kp*td_s/dt_s * (e - 2*e_prev + e_prev2)（td_s>0 时）—— 微分增量
    acc    := acc + Δpid；out := acc + ff，钳位到 [out_min, out_max]
    收尾    acc := out - ff（回算）
  抗积分饱和：输出钳位 + 回算 + 条件积分（输出已贴限且积分增量继续推向
  限幅时丢弃积分项）。
  模式优先级：track（输出跟踪，联锁强制/外部接管用）> NOT auto（手动）>
  自动。track/手动时 out 跟踪给定值（钳位），误差历史照常更新，回到自动
  无跳变。ff 直接叠加在钳位前输出上（前馈即时生效；回算保证 ff 增减、
  钳位、模式切换全程无扰）。
  向后兼容：新增输入（ff/track/track_value/sp_rate）均带默认值，取默认值
  时行为与旧版逐拍一致，既有调用无需改动；新增输出 dev 不影响旧接线。

  输入：
    pv          REAL  过程值
    sp          REAL  设定值
    kp          REAL  比例增益
    ti_s        REAL  积分时间 s（0 = 积分关）
    td_s        REAL  微分时间 s（0 = 微分关，微分项不滤波）
    dt_s        REAL  采样周期 s（<=0 时本周期保持不动作）
    direct      BOOL  TRUE = 正作用（pv↑→out↑），FALSE = 反作用
    auto        BOOL  TRUE 自动 / FALSE 手动
    manual_out  REAL  手动给定值
    out_min     REAL  输出下限（假定 out_min < out_max）
    out_max     REAL  输出上限
    ff          REAL  前馈量（钳位前叠加到输出；接 FB_LEADLAG 可加动态补偿）
    track       BOOL  输出跟踪使能（优先级最高，盖过 auto/手动）
    track_value REAL  跟踪值
    sp_rate     REAL  设定值斜坡限速 单位/s（0 = 不限速）
  输出：
    out         REAL  调节输出（各模式均钳位）
    dev         REAL  偏差 pv - sp_int（经 SP 斜坡后的实际工作偏差）
*)
FUNCTION_BLOCK FB_PID
  VAR_INPUT
    pv          : REAL;
    sp          : REAL;
    kp          : REAL := 1.0;
    ti_s        : REAL := 0.0;
    td_s        : REAL := 0.0;
    dt_s        : REAL := 0.1;
    direct      : BOOL := FALSE;
    auto        : BOOL := TRUE;
    manual_out  : REAL := 0.0;
    out_min     : REAL := 0.0;
    out_max     : REAL := 100.0;
    ff          : REAL := 0.0;
    track       : BOOL := FALSE;
    track_value : REAL := 0.0;
    sp_rate     : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out : REAL;
    dev : REAL;
  END_VAR
  VAR
    sp_int    : REAL; (* 斜坡后的内部工作设定值 *)
    acc       : REAL; (* PID 累加器（不含 ff） *)
    e         : REAL;
    e_prev    : REAL;
    e_prev2   : REAL;
    p_term    : REAL;
    i_term    : REAL;
    d_term    : REAL;
    sp_step   : REAL;
    sp_delta  : REAL;
    init_done : BOOL; (* sp_int 首扫描对中标志 *)
  END_VAR

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持输出与历史不变 *)
  END_IF;

  (* 设定值斜坡：sp_rate > 0 时 sp_int 限速逼近 sp；首扫描直接对中 *)
  IF NOT init_done THEN
    sp_int := sp;
    init_done := TRUE;
  ELSIF sp_rate > 0.0 THEN
    sp_step := sp_rate * dt_s;
    sp_delta := sp - sp_int;
    IF sp_delta > sp_step THEN
      sp_int := sp_int + sp_step;
    ELSIF sp_delta < 0.0 - sp_step THEN
      sp_int := sp_int - sp_step;
    ELSE
      sp_int := sp;
    END_IF;
  ELSE
    sp_int := sp;
  END_IF;

  dev := pv - sp_int;

  (* 控制偏差；正作用取反号 *)
  IF direct THEN
    e := pv - sp_int;
  ELSE
    e := sp_int - pv;
  END_IF;

  IF track THEN
    (* 输出跟踪：无条件跟随 track_value（联锁强制/超驰接管），优先级最高 *)
    out := track_value;
  ELSIF NOT auto THEN
    (* 手动：跟踪 manual_out，切回自动时增量式自然无扰 *)
    out := manual_out;
  ELSE
    p_term := kp * (e - e_prev);

    i_term := 0.0;
    IF ti_s > 0.0 THEN
      i_term := kp * dt_s / ti_s * e;
      (* 条件积分：输出已饱和且积分仍推向限幅方向时，本周期丢弃积分项 *)
      IF (out >= out_max AND i_term > 0.0) OR (out <= out_min AND i_term < 0.0) THEN
        i_term := 0.0;
      END_IF;
    END_IF;

    d_term := 0.0;
    IF td_s > 0.0 THEN
      d_term := kp * td_s / dt_s * (e - 2.0 * e_prev + e_prev2);
    END_IF;

    acc := acc + p_term + i_term + d_term;
    out := acc + ff; (* 前馈钳位前叠加 *)
  END_IF;

  (* 输出钳位（增量式下同时起抗积分饱和作用） *)
  IF out > out_max THEN
    out := out_max;
  ELSIF out < out_min THEN
    out := out_min;
  END_IF;

  (* 回算：acc 恒 = 钳位后输出 - 前馈 → 钳位/ff 变化/手动/跟踪全程无扰 *)
  acc := out - ff;

  e_prev2 := e_prev;
  e_prev  := e;
END_FUNCTION_BLOCK
