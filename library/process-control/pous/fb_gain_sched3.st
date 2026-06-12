(*
  FB_GAIN_SCHED3 — 三区增益调度（gain scheduling，迟滞分区 +
  可选区界线性过渡）。

  ≈ 标准增益调度器（gain scheduler / parameter scheduler，按工况
  变量分区切换 PID 参数组），非线性对象分段整定的惯用块。

  用途：对象增益随工况大幅变化（负荷段、阀特性段、pH 强非线性段）
  时按调度变量（负荷/开度/PV 本身）分三区出 kp/ti/td 参数组，
  输出直接接 FB_PID / FB_PID_POS 的参数输入——参数在线改变即时
  生效（两块均按当前参数算增量/分项）。

  行为：
    分区（u_sched 对 x12 / x23 两个区界，假定 x12 < x23）：
      区 1：u <= x12；区 2：x12 < u <= x23；区 3：u > x23。
    迟滞：越过区界 ± hyst 才换区（防区界附近抖动来回切参数）；
    首扫描按纯比较播种。大阶跃一拍内可连跳两区（逐级迁移收敛）。
    参数输出：
      interp = FALSE：取当前区参数组（迟滞切换，参数有台阶——
      接 FB_PID 增量式天然无扰，接 FB_PID_POS 时 P 项会按新 kp
      立即重算）；
      interp = TRUE 且 hyst > 0：u 落在区界 ±hyst 内时两区参数
      线性过渡（参数输出连续、仅由 u_sched 决定；zone 仍按迟滞出
      离散指示）。
    注意：interp 混用 ti = 0（积分关）与 ti > 0 的区时，过渡段会
    产生极小的 ti（极强积分）——各区 ti/td 应同为 0 或同为非 0。

  输入：
    u_sched         REAL  调度变量（负荷/阀位/PV 等）
    x12             REAL  区 1/2 边界
    x23             REAL  区 2/3 边界（> x12，且两过渡带不重叠）
    hyst            REAL  区界迟滞半宽（0 = 无迟滞）
    interp          BOOL  TRUE = 区界 ±hyst 内参数线性过渡
    kp1, ti1_s, td1_s REAL  区 1 参数组
    kp2, ti2_s, td2_s REAL  区 2 参数组
    kp3, ti3_s, td3_s REAL  区 3 参数组
  输出：
    kp_out   REAL  调度后比例增益（接 PID 的 kp）
    ti_out_s REAL  调度后积分时间 s
    td_out_s REAL  调度后微分时间 s
    zone     REAL  当前区 1 / 2 / 3（迟滞判定）
*)
FUNCTION_BLOCK FB_GAIN_SCHED3
  VAR_INPUT
    u_sched : REAL;
    x12     : REAL := 33.3;
    x23     : REAL := 66.6;
    hyst    : REAL := 0.0;
    interp  : BOOL := FALSE;
    kp1     : REAL := 1.0;
    ti1_s   : REAL := 0.0;
    td1_s   : REAL := 0.0;
    kp2     : REAL := 1.0;
    ti2_s   : REAL := 0.0;
    td2_s   : REAL := 0.0;
    kp3     : REAL := 1.0;
    ti3_s   : REAL := 0.0;
    td3_s   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    kp_out   : REAL;
    ti_out_s : REAL;
    td_out_s : REAL;
    zone     : REAL;
  END_VAR
  VAR
    f : REAL; (* 过渡带内插值分数 0..1 *)
  END_VAR

  (* 1) 区选择（迟滞）：首扫描 zone = 0 → 按纯比较播种 *)
  IF zone < 0.5 THEN
    IF u_sched <= x12 THEN
      zone := 1.0;
    ELSIF u_sched <= x23 THEN
      zone := 2.0;
    ELSE
      zone := 3.0;
    END_IF;
  END_IF;

  (* 逐级迁移（1→2、3→2 先判，2→1/2→3 后判：大阶跃一拍收敛） *)
  IF zone <= 1.5 THEN
    IF u_sched > x12 + hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 2.5 THEN
    IF u_sched < x23 - hyst THEN
      zone := 2.0;
    END_IF;
  END_IF;
  IF zone >= 1.5 AND zone <= 2.5 THEN
    IF u_sched > x23 + hyst THEN
      zone := 3.0;
    ELSIF u_sched < x12 - hyst THEN
      zone := 1.0;
    END_IF;
  END_IF;

  (* 2) 区参数（离散取值，迟滞切换） *)
  IF zone <= 1.5 THEN
    kp_out   := kp1;
    ti_out_s := ti1_s;
    td_out_s := td1_s;
  ELSIF zone <= 2.5 THEN
    kp_out   := kp2;
    ti_out_s := ti2_s;
    td_out_s := td2_s;
  ELSE
    kp_out   := kp3;
    ti_out_s := ti3_s;
    td_out_s := td3_s;
  END_IF;

  (* 3) interp：区界 ±hyst 内线性过渡（参数连续、仅由 u_sched 决定，
        覆盖上面的离散取值；zone 输出不受影响） *)
  IF interp AND hyst > 0.0 THEN
    IF (u_sched > x12 - hyst) AND (u_sched < x12 + hyst) THEN
      f := (u_sched - (x12 - hyst)) / (2.0 * hyst);
      kp_out   := kp1 + f * (kp2 - kp1);
      ti_out_s := ti1_s + f * (ti2_s - ti1_s);
      td_out_s := td1_s + f * (td2_s - td1_s);
    ELSIF (u_sched > x23 - hyst) AND (u_sched < x23 + hyst) THEN
      f := (u_sched - (x23 - hyst)) / (2.0 * hyst);
      kp_out   := kp2 + f * (kp3 - kp2);
      ti_out_s := ti2_s + f * (ti3_s - ti2_s);
      td_out_s := td2_s + f * (td3_s - td2_s);
    ELSIF u_sched <= x12 - hyst THEN
      kp_out   := kp1;
      ti_out_s := ti1_s;
      td_out_s := td1_s;
    ELSIF u_sched >= x23 + hyst THEN
      kp_out   := kp3;
      ti_out_s := ti3_s;
      td_out_s := td3_s;
    ELSE
      kp_out   := kp2; (* 两过渡带之间：纯区 2 *)
      ti_out_s := ti2_s;
      td_out_s := td2_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
