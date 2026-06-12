(*
  FB_CASCADE — 级联耦合胶水（主回路输出 → 副回路 SP 映射 + 非级联跟踪回算）。

  ≈ 标准级联耦合块（cascade SP coupling / 主回路跟踪回算），
  串级回路组态的惯用胶水：两台 PID 之间只差这一块。

  用途：串级回路组态（温度主回路 → 流量副回路之类），把主调输出
  0..100% 映射到副回路 SP 工程量程，并在非级联时给主调供跟踪值，
  保证 CAS 投/切无扰。
  行为：
    CAS（cas_on = TRUE）：
      slave_sp := sp_min + master_out / 100 * (sp_max - sp_min)
      master_track := FALSE（主调正常调节）。
    非 CAS：
      slave_sp := slave_sp_local（操作员直接给副回路的本地 SP）；
      master_track := TRUE，master_track_value :=
        (slave_sp_now - sp_min) / (sp_max - sp_min) * 100，钳位 0..100
      ——主调输出被强制为"当前副 SP 的等效百分比"，切回 CAS 的那一拍
      slave_sp 不跳变（无扰切换）。
      量程未组态（|sp_max - sp_min| < 1E-6）→ 跟踪值取 0（防除零）。
    跟踪值每拍都计算（与模式无关），CAS 切除瞬间即有效。
    纯组合逻辑，无内部状态。

  接线（与两台 FB_PID 组级联；slave_sp_now 喂上一拍工作 SP）：
    casc(master_out := pid_master.out, cas_on := cas_enable,
         sp_min := 0.0, sp_max := 2000.0,
         slave_sp_local := op_sp, slave_sp_now := slave_sp_used);
    slave_sp_used := casc.slave_sp;
    pid_master(pv := temp_pv, sp := temp_sp, ..., dt_s := scan_s,
               track := casc.master_track,
               track_value := casc.master_track_value);
    pid_slave(pv := flow_pv, sp := slave_sp_used, ..., dt_s := scan_s);

  输入：
    master_out     REAL  主调输出 0..100 %
    cas_on         BOOL  级联投入（FALSE = 副回路用本地 SP，主调跟踪）
    sp_min         REAL  副回路 SP 量程下限（工程单位）
    sp_max         REAL  副回路 SP 量程上限
    slave_sp_local REAL  非级联时的副回路本地 SP
    slave_sp_now   REAL  副回路当前工作 SP（喂上一拍 slave_sp，跟踪回算用）
  输出：
    slave_sp           REAL  副回路设定值（接 pid_slave.sp）
    master_track       BOOL  主调跟踪命令（接 pid_master.track）
    master_track_value REAL  主调跟踪值 0..100 %（接 pid_master.track_value）
*)
FUNCTION_BLOCK FB_CASCADE
  VAR_INPUT
    master_out     : REAL;
    cas_on         : BOOL := FALSE;
    sp_min         : REAL := 0.0;
    sp_max         : REAL := 100.0;
    slave_sp_local : REAL := 0.0;
    slave_sp_now   : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    slave_sp           : REAL;
    master_track       : BOOL;
    master_track_value : REAL;
  END_VAR
  VAR
    span : REAL; (* 副回路 SP 量程跨度 *)
  END_VAR

  span := sp_max - sp_min;

  (* 跟踪值每拍计算：当前副 SP 的等效百分比（量程未组态取 0，防除零） *)
  IF (span < 0.000001) AND (span > -0.000001) THEN
    master_track_value := 0.0;
  ELSE
    master_track_value := (slave_sp_now - sp_min) / span * 100.0;
    IF master_track_value < 0.0 THEN
      master_track_value := 0.0;
    ELSIF master_track_value > 100.0 THEN
      master_track_value := 100.0;
    END_IF;
  END_IF;

  IF cas_on THEN
    slave_sp := sp_min + master_out / 100.0 * span;
    master_track := FALSE;
  ELSE
    slave_sp := slave_sp_local;
    master_track := TRUE;
  END_IF;
END_FUNCTION_BLOCK
