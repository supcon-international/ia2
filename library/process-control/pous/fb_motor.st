(*
  FB_MOTOR — 电机启停控制，带运行反馈监视与故障锁存。

  用途：皮带机、风机、泵等单速电机的标准启停封装。
  例：远程模式下 HMI/上位启停命令 → 接触器输出，运行反馈（接触器辅点/
  电流继电器）超时不一致则故障停机。

  行为：
    1) 故障监视：out_run 与 run_fb 不一致持续 fb_timeout_s（手工 dt_s 累加
       计时）→ fault 锁存；外部故障 fault_in（热继/变频器故障等）→ fault。
       reset 且 fault_in 已消除时清除锁存。
    2) 命令锁存：cmd_start（电平/点动均可）在 remote 且无故障时置位 out_run；
       cmd_stop 或 fault 复位（停车优先）。
    3) 状态字 status：0 停止，1 启动中（已输出未见反馈），2 运行，3 故障。

  输入：
    cmd_start    BOOL  启动命令
    cmd_stop     BOOL  停止命令（优先）
    remote       BOOL  远程允许（仅闸启动；不强制停车）
    run_fb       BOOL  运行反馈
    fault_in     BOOL  外部故障输入
    reset        BOOL  故障复位
    fb_timeout_s REAL  反馈不一致容忍时间 s
    dt_s         REAL  采样周期 s
  输出：
    out_run BOOL  运行输出（接触器/变频器启动）
    fault   BOOL  故障（锁存）
    status  INT   0 停止 / 1 启动中 / 2 运行 / 3 故障
*)
FUNCTION_BLOCK FB_MOTOR
  VAR_INPUT
    cmd_start    : BOOL;
    cmd_stop     : BOOL;
    remote       : BOOL := TRUE;
    run_fb       : BOOL;
    fault_in     : BOOL;
    reset        : BOOL;
    fb_timeout_s : REAL := 5.0;
    dt_s         : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out_run : BOOL;
    fault   : BOOL;
    status  : INT;
  END_VAR
  VAR
    mism_t : REAL; (* 反馈不一致累计时间 s *)
  END_VAR

  (* 1) 故障监视 *)
  IF out_run <> run_fb THEN
    mism_t := mism_t + dt_s;
  ELSE
    mism_t := 0.0;
  END_IF;
  IF fb_timeout_s > 0.0 AND mism_t >= fb_timeout_s THEN
    fault := TRUE;
  END_IF;
  IF fault_in THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT fault_in THEN
    fault  := FALSE;
    mism_t := 0.0;
  END_IF;

  (* 2) 命令锁存（停车/故障优先） *)
  IF cmd_start AND remote AND NOT fault THEN
    out_run := TRUE;
  END_IF;
  IF cmd_stop OR fault THEN
    out_run := FALSE;
  END_IF;

  (* 3) 状态字 *)
  IF fault THEN
    status := 3;
  ELSIF out_run AND run_fb THEN
    status := 2;
  ELSIF out_run THEN
    status := 1;
  ELSE
    status := 0;
  END_IF;
END_FUNCTION_BLOCK
