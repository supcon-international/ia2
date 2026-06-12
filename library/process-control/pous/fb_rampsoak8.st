(*
  FB_RAMPSOAK8 — 8 段程序给定发生器（ramp-soak setpoint programmer）。

  ≈ 标准程序给定/曲线给定块（ramp-soak / setpoint programmer，
  升温曲线、热处理曲线、批次温度程序），各主流 DCS/PLC 与程序
  调节仪均有同类块。

  用途：按"斜坡到目标 + 保温"的分段曲线生成设定值，接 FB_PID /
  FB_PID_POS 的 sp 输入：热处理/养护/反应釜升温程序、开停车
  升降负荷曲线。段参数为标量展开 sp1..sp8 / ramp1_s..ramp8_s /
  soak1_s..soak8_s（本方言 FB 作用域数组不能 codegen，见 README
  坑 7；FB_CHAR 同例）。

  行为：
    每段 N：sp 从上一段电平（段 1 从 sp_init）经 rampN_s 线性斜坡到
    spN（rampN_s <= 0 跳变），再保温 soakN_s（soaking = TRUE；
    soakN_s <= 0 即过）。段参数在进段拍锁存——运行中改参数下一段
    生效。
    run 电平有效：TRUE 启动并推进程序；FALSE → 回到初始（seg := 0，
    sp := sp_init，done 清零），再置 TRUE 从段 1 重新开始。
    hold 电平有效：冻结 sp 与段内计时（hold 期间 advance 无效）。
    advance 上升沿：立即跳下一段（从当前 sp 续斜坡，曲线连续）；
    在末段 advance → 完成（或 cyclic 回绕）。
    末段（n_seg 钳 2..8）保温结束：
      cyclic = FALSE → done := TRUE，sp 钉在末段目标，seg := 0，
      保持到 reset 或 run 撤销；
      cyclic = TRUE  → 回绕段 1（从当前 sp 续斜坡，长期循环）。
    reset 电平有效（优先）：回初始同 run 撤销。
    dt_s <= 0 → 段内不推进（启动/advance 沿照常处理）。

  输入：
    sp1..sp8       REAL  各段目标值（标量展开）
    ramp1_s..ramp8_s REAL  各段斜坡时间 s（<= 0 = 跳变）
    soak1_s..soak8_s REAL  各段保温时间 s（<= 0 = 不保温）
    n_seg          REAL  使用段数 2..8（自动钳位）
    run            BOOL  运行（电平；撤销回初始）
    hold           BOOL  暂停（电平；冻结 sp 与计时）
    advance        BOOL  跳段（上升沿）
    reset          BOOL  复位（电平，优先）
    cyclic         BOOL  TRUE = 末段完成后回绕段 1 循环
    sp_init        REAL  起始值（段 1 斜坡的出发电平）
    dt_s           REAL  采样周期 s
  输出：
    sp      REAL  程序设定值输出
    seg     REAL  当前段 1..8（0 = 未运行/已完成）
    soaking BOOL  保温段进行中
    done    BOOL  程序完成（非循环；保持到 reset / run 撤销）
*)
FUNCTION_BLOCK FB_RAMPSOAK8
  VAR_INPUT
    sp1     : REAL := 0.0;
    sp2     : REAL := 0.0;
    sp3     : REAL := 0.0;
    sp4     : REAL := 0.0;
    sp5     : REAL := 0.0;
    sp6     : REAL := 0.0;
    sp7     : REAL := 0.0;
    sp8     : REAL := 0.0;
    ramp1_s : REAL := 0.0;
    ramp2_s : REAL := 0.0;
    ramp3_s : REAL := 0.0;
    ramp4_s : REAL := 0.0;
    ramp5_s : REAL := 0.0;
    ramp6_s : REAL := 0.0;
    ramp7_s : REAL := 0.0;
    ramp8_s : REAL := 0.0;
    soak1_s : REAL := 0.0;
    soak2_s : REAL := 0.0;
    soak3_s : REAL := 0.0;
    soak4_s : REAL := 0.0;
    soak5_s : REAL := 0.0;
    soak6_s : REAL := 0.0;
    soak7_s : REAL := 0.0;
    soak8_s : REAL := 0.0;
    n_seg   : REAL := 2.0;
    run     : BOOL := FALSE;
    hold    : BOOL := FALSE;
    advance : BOOL := FALSE;
    reset   : BOOL := FALSE;
    cyclic  : BOOL := FALSE;
    sp_init : REAL := 0.0;
    dt_s    : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    sp      : REAL;
    seg     : REAL;
    soaking : BOOL;
    done    : BOOL;
  END_VAR
  VAR
    nn         : REAL; (* 钳位后的段数 2..8 *)
    elapsed_t  : REAL; (* 段内相位计时 s（斜坡/保温各自从 0 起） *)
    level_from : REAL; (* 本段斜坡出发电平 *)
    seg_sp     : REAL; (* 进段锁存的段参数 *)
    seg_ramp_s : REAL;
    seg_soak_s : REAL;
    enter_seg  : REAL; (* 本拍要进入的段号（0 = 不进段） *)
    go_next    : BOOL; (* 本拍段完成/跳段 *)
    adv_edge   : BOOL;
    prev_adv   : BOOL; (* advance 上升沿记忆 *)
  END_VAR

  IF reset THEN
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    elapsed_t := 0.0;
    prev_adv := advance;
    RETURN;
  END_IF;

  adv_edge := advance AND NOT prev_adv;
  prev_adv := advance;

  IF NOT run THEN
    (* 停止：回初始，重新 run 从段 1 开始 *)
    seg := 0.0;
    soaking := FALSE;
    done := FALSE;
    sp := sp_init;
    RETURN;
  END_IF;

  IF done THEN
    RETURN; (* 完成保持：sp 钉在末段目标，等 reset / run 撤销 *)
  END_IF;

  nn := n_seg;
  IF nn < 2.0 THEN
    nn := 2.0;
  ELSIF nn > 8.0 THEN
    nn := 8.0;
  END_IF;

  enter_seg := 0.0;
  go_next := FALSE;

  IF seg < 0.5 THEN
    (* 启动：从 sp_init 进段 1 *)
    sp := sp_init;
    enter_seg := 1.0;
  ELSIF hold THEN
    RETURN; (* 冻结：sp 与计时保持（advance 在 hold 期间无效） *)
  ELSIF adv_edge THEN
    go_next := TRUE; (* 跳段：从当前 sp 续走 *)
  ELSE
    IF dt_s <= 0.0 THEN
      RETURN; (* 采样周期非法：段内不推进 *)
    END_IF;
    elapsed_t := elapsed_t + dt_s;
    IF NOT soaking THEN
      (* 斜坡相位 *)
      IF seg_ramp_s <= 0.0 THEN
        sp := seg_sp; (* 跳变段 *)
        soaking := TRUE;
        elapsed_t := 0.0;
      ELSIF elapsed_t >= seg_ramp_s THEN
        sp := seg_sp;
        soaking := TRUE;
        elapsed_t := elapsed_t - seg_ramp_s; (* 跨相位余量结转 *)
      ELSE
        sp := level_from + (seg_sp - level_from) * elapsed_t / seg_ramp_s;
      END_IF;
    END_IF;
    IF soaking AND elapsed_t >= seg_soak_s THEN
      go_next := TRUE; (* 保温结束 *)
    END_IF;
  END_IF;

  (* 段推进决策：末段 → 完成/回绕；否则进下一段 *)
  IF go_next THEN
    IF seg >= nn - 0.5 THEN
      IF cyclic THEN
        enter_seg := 1.0; (* 回绕：段 1 从当前 sp 续斜坡 *)
      ELSE
        done := TRUE;
        sp := seg_sp; (* 完成：钉在末段目标（提前跳段也直接到位） *)
        seg := 0.0;
        soaking := FALSE;
      END_IF;
    ELSE
      enter_seg := seg + 1.0;
    END_IF;
  END_IF;

  (* 进段：锁存段参数，斜坡从当前 sp 出发 *)
  IF enter_seg > 0.5 THEN
    seg := enter_seg;
    level_from := sp;
    elapsed_t := 0.0;
    soaking := FALSE;
    IF seg <= 1.5 THEN
      seg_sp := sp1;
      seg_ramp_s := ramp1_s;
      seg_soak_s := soak1_s;
    ELSIF seg <= 2.5 THEN
      seg_sp := sp2;
      seg_ramp_s := ramp2_s;
      seg_soak_s := soak2_s;
    ELSIF seg <= 3.5 THEN
      seg_sp := sp3;
      seg_ramp_s := ramp3_s;
      seg_soak_s := soak3_s;
    ELSIF seg <= 4.5 THEN
      seg_sp := sp4;
      seg_ramp_s := ramp4_s;
      seg_soak_s := soak4_s;
    ELSIF seg <= 5.5 THEN
      seg_sp := sp5;
      seg_ramp_s := ramp5_s;
      seg_soak_s := soak5_s;
    ELSIF seg <= 6.5 THEN
      seg_sp := sp6;
      seg_ramp_s := ramp6_s;
      seg_soak_s := soak6_s;
    ELSIF seg <= 7.5 THEN
      seg_sp := sp7;
      seg_ramp_s := ramp7_s;
      seg_soak_s := soak7_s;
    ELSE
      seg_sp := sp8;
      seg_ramp_s := ramp8_s;
      seg_soak_s := soak8_s;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
