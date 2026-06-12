(*
  FB_DOSE — 定量下料/定量给料（粗/精两段 + 落差补偿）。

  ≈ 标准定量加料块（dosing / batch weighing，coarse-fine 两速加料
  + in-flight 落差提前关），配料秤/定量给料控制的惯用块。

  用途：料仓向秤斗/反应釜定量加料：粗加料快速接近目标，精加料
  收口，按落差（关阀后仍在途的物料）提前关精阀，沉降后给出
  完成与过冲量。流量输入可来自称重微分或流量计。

  行为：
    start 上升沿（空闲且未完成、abort 未压下时）开新批：qty 清零，
    进入加料段。加料/沉降期间累积 qty := qty + flow * dt_s
    （flow <= 0 不计，落料完成的判据之一）。
      粗段：qty < target - fine_qty → q_coarse = TRUE（粗精两阀同开）；
      精段：qty < target - inflight → q_fine = TRUE；到点关精阀
      （落差 inflight 即在途量预留），进入沉降段。
      沉降段：等 flow <= 0 或固定 2 s（先到为准）→ done := TRUE，
      overshoot := qty - target（在途料落完后的实际过冲）。
    abort 电平有效：立即停双阀、退出加料/沉降，qty 保留（盘点用），
    done 不置位。
    done 后 start 无效，须 reset（电平有效）清 qty/done/overshoot
    开下一批。
    假定 fine_qty >= inflight >= 0（粗阀先关、精阀后关）；
    done 之后的滴漏不再计入 qty。
    dt_s <= 0 → 本周期保持不动作（命令沿照常记忆）。

  输入：
    start    BOOL  启动（上升沿开新批）
    abort    BOOL  中止（电平有效：停阀保量）
    flow     REAL  瞬时流量 单位/s（称重微分或流量计）
    target   REAL  目标量（与 flow 积分同单位）
    fine_qty REAL  精加料段量（剩这么多时关粗阀）
    inflight REAL  落差/在途量（剩这么多时关精阀）
    dt_s     REAL  采样周期 s
    reset    BOOL  批复位（电平有效：清 qty/done/overshoot）
  输出：
    q_coarse  BOOL  粗加料阀
    q_fine    BOOL  精加料阀
    qty       REAL  本批累计量
    done      BOOL  本批完成（沉降后置位，保持到 reset）
    overshoot REAL  完成后实际 - 目标（落差整定依据）
*)
FUNCTION_BLOCK FB_DOSE
  VAR_INPUT
    start    : BOOL;
    abort    : BOOL;
    flow     : REAL;
    target   : REAL := 100.0;
    fine_qty : REAL := 10.0;
    inflight : REAL := 0.0;
    dt_s     : REAL := 0.1;
    reset    : BOOL;
  END_VAR
  VAR_OUTPUT
    q_coarse  : BOOL;
    q_fine    : BOOL;
    qty       : REAL;
    done      : BOOL;
    overshoot : REAL;
  END_VAR
  VAR
    dosing     : BOOL; (* 加料段（粗/精） *)
    settling   : BOOL; (* 沉降等待段 *)
    settle_t   : REAL; (* 沉降计时 s *)
    prev_start : BOOL; (* 启动上升沿记忆 *)
  END_VAR

  IF reset THEN
    dosing     := FALSE;
    settling   := FALSE;
    settle_t   := 0.0;
    qty        := 0.0;
    done       := FALSE;
    overshoot  := 0.0;
    q_coarse   := FALSE;
    q_fine     := FALSE;
    prev_start := start;
    RETURN;
  END_IF;

  IF abort THEN
    (* 中止：停阀退段，qty 保留，done 不置位 *)
    dosing   := FALSE;
    settling := FALSE;
    q_coarse := FALSE;
    q_fine   := FALSE;
  END_IF;

  (* start 上升沿且空闲且未完成 → 开新批 *)
  IF start AND NOT prev_start AND NOT abort
     AND NOT dosing AND NOT settling AND NOT done THEN
    dosing    := TRUE;
    qty       := 0.0;
    overshoot := 0.0;
    settle_t  := 0.0;
  END_IF;
  prev_start := start;

  IF dt_s <= 0.0 THEN
    RETURN; (* 采样周期非法：保持 *)
  END_IF;

  (* 加料/沉降期间累积流量（负流量不计） *)
  IF (dosing OR settling) AND flow > 0.0 THEN
    qty := qty + flow * dt_s;
  END_IF;

  IF dosing THEN
    q_coarse := qty < target - fine_qty;
    q_fine   := qty < target - inflight;
    IF NOT q_fine THEN
      (* 到落差预留点：关精阀进沉降段 *)
      q_coarse := FALSE;
      dosing   := FALSE;
      settling := TRUE;
      settle_t := 0.0;
    END_IF;
  END_IF;

  IF settling THEN
    settle_t := settle_t + dt_s;
    IF flow <= 0.0 OR settle_t >= 2.0 THEN
      settling  := FALSE;
      done      := TRUE;
      overshoot := qty - target;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
