(*
  FB_VALVE — 开关阀（两位阀）控制，带行程超时与双限位故障监视。

  用途：气动/电动开关阀的标准封装，ZSO/ZSC 双限位回讯。
  例：进料阀开到位才允许下游启动；行程超时报阀卡。

  行为：
    1) 故障监视：
       - zso 与 zsc 同时为 TRUE → 限位开关故障，立即 fault；
       - 命令位置与限位回讯不一致（开阀未见 zso / 关阀未见 zsc）持续
         travel_timeout_s（手工 dt_s 累加计时）→ 行程超时 fault；
       - 外部故障 fault_in → fault。均锁存，reset 且 fault_in 消除后清除。
    2) 命令锁存：cmd_open 在 remote 且无故障时置位 out_open；
       cmd_close 或 fault 复位（故障时故障安全关阀，关阀优先）。
    3) 状态字 status：0 关到位 / 1 行程中 / 2 开到位 / 3 故障。

  输入：
    cmd_open         BOOL  开阀命令
    cmd_close        BOOL  关阀命令（优先）
    remote           BOOL  远程允许（仅闸开阀）
    zso              BOOL  开到位限位（Zero Switch Open）
    zsc              BOOL  关到位限位（Zero Switch Closed）
    fault_in         BOOL  外部故障输入
    reset            BOOL  故障复位
    travel_timeout_s REAL  行程超时 s
    dt_s             REAL  采样周期 s
  输出：
    out_open BOOL  开阀输出（TRUE 开 / FALSE 关）
    fault    BOOL  故障（锁存）
    status   INT   0 关到位 / 1 行程中 / 2 开到位 / 3 故障
*)
FUNCTION_BLOCK FB_VALVE
  VAR_INPUT
    cmd_open         : BOOL;
    cmd_close        : BOOL;
    remote           : BOOL := TRUE;
    zso              : BOOL;
    zsc              : BOOL;
    fault_in         : BOOL;
    reset            : BOOL;
    travel_timeout_s : REAL := 30.0;
    dt_s             : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    out_open : BOOL;
    fault    : BOOL;
    status   : INT;
  END_VAR
  VAR
    travel_t : REAL; (* 未到位累计时间 s *)
    in_pos   : BOOL;
  END_VAR

  (* 1) 故障监视 *)
  IF zso AND zsc THEN
    fault := TRUE; (* 双限位同时有效：限位开关故障 *)
  END_IF;

  in_pos := (out_open AND zso) OR (NOT out_open AND zsc);
  IF in_pos THEN
    travel_t := 0.0;
  ELSE
    travel_t := travel_t + dt_s;
  END_IF;
  IF travel_timeout_s > 0.0 AND travel_t >= travel_timeout_s THEN
    fault := TRUE; (* 行程超时：阀卡/限位丢失 *)
  END_IF;

  IF fault_in THEN
    fault := TRUE;
  END_IF;
  IF reset AND NOT fault_in THEN
    fault    := FALSE;
    travel_t := 0.0;
  END_IF;

  (* 2) 命令锁存（关阀/故障优先，故障安全关阀） *)
  IF cmd_open AND remote AND NOT fault THEN
    out_open := TRUE;
  END_IF;
  IF cmd_close OR fault THEN
    out_open := FALSE;
  END_IF;

  (* 3) 状态字 *)
  IF fault THEN
    status := 3;
  ELSIF out_open AND zso THEN
    status := 2;
  ELSIF NOT out_open AND zsc THEN
    status := 0;
  ELSE
    status := 1;
  END_IF;
END_FUNCTION_BLOCK
