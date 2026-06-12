(*
  FB_MOTOR_REV — 双向（正/反转）电机控制，带换向闭锁、运行反馈
  监视与故障锁存。

  ≈ 标准双向电机块（reversing motor / forward-reverse starter，
  含换向互锁延时），各主流 DCS/PLC 标准库均有同类块。

  用途：搅拌器、卷扬、双向皮带、闸门开度电机等可逆运转设备的
  标准封装（范式沿自 FB_MOTOR：命令锁存 + 反馈超时故障锁存）。

  行为：
    1) 故障监视：fb_fwd 与 fb_rev 同时为 TRUE → 反馈/接触器故障，
       立即 fault；输出与反馈不一致（q_fwd<>fb_fwd 或 q_rev<>fb_rev）
       持续 fb_timeout_s（手工 dt_s 累加计时）→ fault。均锁存，
       reset 且双反馈条件已消除时清除。
    2) 方向命令锁存（停车/故障优先）：cmd_fwd / cmd_rev 置位对应
       方向请求（同拍双向命令视为冲突，不动作）；stop 或 fault 清除
       全部请求。
    3) 换向闭锁：换向（反向请求仍在、或反向反馈仍在转）时强制双输出
       全停 reversal_delay_s 后才允许新方向输出——接触器灭弧/机械
       泄劲时间。reversal_delay_s 应小于 fb_timeout_s（闭锁等待期的
       反馈余转靠超时容忍）。
    4) 状态字 state：0 停止 / 1 正转 / 2 反转 / 3 故障 / 4 换向等待。

  输入：
    cmd_fwd          BOOL  正转命令
    cmd_rev          BOOL  反转命令
    stop             BOOL  停止命令（优先）
    fb_fwd           BOOL  正转运行反馈
    fb_rev           BOOL  反转运行反馈
    fb_timeout_s     REAL  反馈不一致容忍时间 s
    reversal_delay_s REAL  换向全停闭锁时间 s
    reset            BOOL  故障复位
    dt_s             REAL  采样周期 s
  输出：
    q_fwd BOOL  正转输出
    q_rev BOOL  反转输出
    fault BOOL  故障（锁存）
    state REAL  0 停止 / 1 正转 / 2 反转 / 3 故障 / 4 换向等待
*)
FUNCTION_BLOCK FB_MOTOR_REV
  VAR_INPUT
    cmd_fwd          : BOOL;
    cmd_rev          : BOOL;
    stop             : BOOL;
    fb_fwd           : BOOL;
    fb_rev           : BOOL;
    fb_timeout_s     : REAL := 5.0;
    reversal_delay_s : REAL := 2.0;
    reset            : BOOL;
    dt_s             : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_fwd : BOOL;
    q_rev : BOOL;
    fault : BOOL;
    state : REAL;
  END_VAR
  VAR
    want_fwd : BOOL; (* 锁存的方向请求 *)
    want_rev : BOOL;
    wait_t   : REAL; (* 换向闭锁剩余时间 s *)
    mism_t   : REAL; (* 反馈不一致累计时间 s *)
    conflict : BOOL; (* 同拍双向命令 *)
  END_VAR

  (* 1) 故障监视 *)
  IF fb_fwd AND fb_rev THEN
    fault := TRUE; (* 双向反馈同时有效：接触器粘连/反馈回路故障 *)
  END_IF;
  IF (q_fwd <> fb_fwd) OR (q_rev <> fb_rev) THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT (fb_fwd AND fb_rev) THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) 方向命令锁存（停车/故障优先；同拍双向命令冲突不动作） *)
  conflict := cmd_fwd AND cmd_rev;
  IF cmd_fwd AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_rev OR fb_rev THEN
      wait_t := reversal_delay_s; (* 反向仍在令/仍在转：先全停闭锁 *)
    END_IF;
    want_fwd := TRUE;
    want_rev := FALSE;
  ELSIF cmd_rev AND NOT conflict AND NOT fault AND NOT stop THEN
    IF want_fwd OR fb_fwd THEN
      wait_t := reversal_delay_s;
    END_IF;
    want_rev := TRUE;
    want_fwd := FALSE;
  END_IF;
  IF stop OR fault THEN
    want_fwd := FALSE;
    want_rev := FALSE;
  END_IF;

  (* 3) 换向闭锁计时（手工 dt_s 递减） *)
  IF wait_t > 0.0 THEN
    wait_t := wait_t - dt_s;
    IF wait_t < 0.0 THEN
      wait_t := 0.0;
    END_IF;
  END_IF;

  (* 4) 输出：闭锁期间双输出强制全停 *)
  q_fwd := want_fwd AND wait_t <= 0.0;
  q_rev := want_rev AND wait_t <= 0.0;

  (* 5) 状态字 *)
  IF fault THEN
    state := 3.0;
  ELSIF wait_t > 0.0 AND (want_fwd OR want_rev) THEN
    state := 4.0;
  ELSIF q_fwd THEN
    state := 1.0;
  ELSIF q_rev THEN
    state := 2.0;
  ELSE
    state := 0.0;
  END_IF;
END_FUNCTION_BLOCK
