(*
  FB_MOV — 电动阀（开/停/关三命令 + ZSO/ZSC 双限位 + 行程超时）。

  ≈ 标准电动阀块（motor-operated valve，开/停/关型，带行程监视），
  各主流 DCS/PLC 标准库均有同类块。

  用途：电动闸阀/蝶阀等"开-停-关"型电动阀的标准封装。与 FB_VALVE
  （两位阀，单输出保持）的区别：本块是双方向脉冲驱动 + 可中停，
  电机到位由限位自动停；与 FB_PID_3STEP（调节型步进）的区别：
  本块只做全开/全关/中停的顺控操作，不做闭环调节。

  行为：
    1) 故障监视：zso 与 zsc 同时 TRUE → 限位开关故障，立即 fault；
       行程（任一方向输出有效且未到目标限位）持续超过 travel_s →
       行程超时 fault（阀卡/限位丢失），同时停输出。均锁存，
       reset 且双限位条件已消除时清除（行程计时同步清零）。
    2) 命令：cmd_stop 或故障 → 双输出停（优先；cmd_open 与
       cmd_close 同拍同发视为冲突，按停处理）；cmd_open 锁存开输出、
       cmd_close 锁存关输出（互斥，新命令即新行程，行程计时清零）。
    3) 到位自停：q_open 期间 zso 到 → 停开输出；q_close 期间 zsc
       到 → 停关输出。
    4) 状态字 pos_state：0 全关 / 1 开行程中 / 2 全开 / 3 关行程中 /
       4 故障；中停（无输出且不在限位）保持上一状态值。

  输入：
    cmd_open  BOOL  开阀命令
    cmd_close BOOL  关阀命令
    cmd_stop  BOOL  停止命令（优先，中停）
    zso       BOOL  开到位限位
    zsc       BOOL  关到位限位
    travel_s  REAL  行程超时 s（<= 0 = 不监视）
    reset     BOOL  故障复位
    dt_s      REAL  采样周期 s
  输出：
    q_open    BOOL  开方向输出
    q_close   BOOL  关方向输出
    pos_state REAL  0 全关 / 1 开行程中 / 2 全开 / 3 关行程中 / 4 故障
    fault     BOOL  故障（锁存）
*)
FUNCTION_BLOCK FB_MOV
  VAR_INPUT
    cmd_open  : BOOL;
    cmd_close : BOOL;
    cmd_stop  : BOOL;
    zso       : BOOL;
    zsc       : BOOL;
    travel_s  : REAL := 60.0;
    reset     : BOOL;
    dt_s      : REAL := 0.1;
  END_VAR
  VAR_OUTPUT
    q_open    : BOOL;
    q_close   : BOOL;
    pos_state : REAL;
    fault     : BOOL;
  END_VAR
  VAR
    travel_t : REAL; (* 当前行程累计时间 s *)
  END_VAR

  (* 1) 双限位故障（立即） *)
  IF zso AND zsc THEN
    fault := TRUE; (* 双限位同时有效：限位开关故障 *)
  END_IF;

  (* 复位（双限位条件仍在时无效） *)
  IF reset AND NOT (zso AND zsc) THEN
    fault    := FALSE;
    travel_t := 0.0;
  END_IF;

  (* 2) 命令锁存：停/故障/同拍双命令 → 全停；开关互斥，新命令即新行程 *)
  IF cmd_stop OR fault OR (cmd_open AND cmd_close) THEN
    q_open  := FALSE;
    q_close := FALSE;
  ELSIF cmd_open THEN
    IF NOT q_open THEN
      travel_t := 0.0; (* 新行程：计时清零 *)
    END_IF;
    q_open  := TRUE;
    q_close := FALSE;
  ELSIF cmd_close THEN
    IF NOT q_close THEN
      travel_t := 0.0;
    END_IF;
    q_close := TRUE;
    q_open  := FALSE;
  END_IF;

  (* 3) 到位自停 *)
  IF q_open AND zso THEN
    q_open := FALSE;
  END_IF;
  IF q_close AND zsc THEN
    q_close := FALSE;
  END_IF;

  (* 4) 行程超时（手工 dt_s 累加计时） *)
  IF q_open OR q_close THEN
    travel_t := travel_t + dt_s;
    IF travel_s > 0.0 AND travel_t >= travel_s THEN
      fault   := TRUE; (* 行程超时：阀卡/限位丢失 *)
      q_open  := FALSE;
      q_close := FALSE;
    END_IF;
  ELSE
    travel_t := 0.0;
  END_IF;

  (* 5) 状态字（中停保持上一状态值） *)
  IF fault THEN
    pos_state := 4.0;
  ELSIF zso THEN
    pos_state := 2.0;
  ELSIF zsc THEN
    pos_state := 0.0;
  ELSIF q_open THEN
    pos_state := 1.0;
  ELSIF q_close THEN
    pos_state := 3.0;
  END_IF;
END_FUNCTION_BLOCK
