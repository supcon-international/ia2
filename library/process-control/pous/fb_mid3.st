(*
  FB_MID3 — 三取中表决（median select / 2oo3 模拟量表决）。

  ≈ 标准三取中/冗余测量表决块（median selector，2oo3 voting），
  冗余变送器信号选择的惯用块。

  用途：三冗余变送器取中（剔除单点漂移/跳变）、双冗余取均、
  关键测量的坏值降级处理（配 FB_SCALE 的 fault 作 good_x）。
  行为（按好通道数 n_good 降级）：
    3 好 → 取中值（天然剔除单通道漂移）；
    2 好 → 取两好通道均值；
    1 好 → 直通该通道；
    0 好 → out 保持上一拍值，q_fail = TRUE（电平，非锁存）。
    偏差监视：dev_max > 0 时，任意两个好通道之差 > dev_max → q_dev
    （提示冗余通道间漂移拉大，该校验变送器了；dev_max <= 0 关闭监视）。
    good_x 接坏值判定（变送器断线/超量程，如 FB_SCALE.fault 取反），
    默认 TRUE = 全好。
    注意：首拍即 0 好时 out 为初值 0（无历史可保持）；q_fail 期间的
    保持值是"最后一次有好通道时的表决值"。

  输入：
    a, b, c                REAL  三路冗余测量
    good_a, good_b, good_c BOOL  各路有效（默认 TRUE；FALSE = 剔除）
    dev_max                REAL  好通道间偏差监视限值（<= 0 = 关闭）
  输出：
    out    REAL  表决输出（0 好时保持）
    n_good REAL  当前好通道数 0..3
    q_dev  BOOL  好通道间偏差超限
    q_fail BOOL  全部通道失效（输出处于保持）
*)
FUNCTION_BLOCK FB_MID3
  VAR_INPUT
    a       : REAL;
    b       : REAL;
    c       : REAL;
    good_a  : BOOL := TRUE;
    good_b  : BOOL := TRUE;
    good_c  : BOOL := TRUE;
    dev_max : REAL := 0.0;
  END_VAR
  VAR_OUTPUT
    out    : REAL;
    n_good : REAL;
    q_dev  : BOOL;
    q_fail : BOOL;
  END_VAR

  n_good := 0.0;
  IF good_a THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_b THEN
    n_good := n_good + 1.0;
  END_IF;
  IF good_c THEN
    n_good := n_good + 1.0;
  END_IF;

  q_fail := n_good <= 0.0;

  (* 偏差监视：任意两个好通道差超 dev_max（内联 IF 替代 ABS，库惯例） *)
  q_dev := FALSE;
  IF dev_max > 0.0 THEN
    IF good_a AND good_b THEN
      IF (a - b > dev_max) OR (b - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_a AND good_c THEN
      IF (a - c > dev_max) OR (c - a > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
    IF good_b AND good_c THEN
      IF (b - c > dev_max) OR (c - b > dev_max) THEN
        q_dev := TRUE;
      END_IF;
    END_IF;
  END_IF;

  IF n_good >= 3.0 THEN
    (* 三好：取中值 *)
    IF (a >= b AND a <= c) OR (a <= b AND a >= c) THEN
      out := a;
    ELSIF (b >= a AND b <= c) OR (b <= a AND b >= c) THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  ELSIF n_good >= 2.0 THEN
    (* 两好：取均值 *)
    IF NOT good_a THEN
      out := (b + c) / 2.0;
    ELSIF NOT good_b THEN
      out := (a + c) / 2.0;
    ELSE
      out := (a + b) / 2.0;
    END_IF;
  ELSIF n_good >= 1.0 THEN
    (* 一好：直通 *)
    IF good_a THEN
      out := a;
    ELSIF good_b THEN
      out := b;
    ELSE
      out := c;
    END_IF;
  END_IF;
  (* 0 好：out 保持上一拍值，q_fail 提示 *)
END_FUNCTION_BLOCK
