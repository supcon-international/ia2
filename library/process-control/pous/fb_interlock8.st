(*
  FB_INTERLOCK8 — 8 路联锁汇总 + 首出（first-out）记录。

  ≈ 联锁汇总 + 首出捕捉（first-out annunciation，参考 ISA-18.1 首出窗概念）。

  用途：机组联锁跳车原因追溯：多条件汇总成一个跳闸位，并锁存最先
  动作的那一路，事后定位跳车根因（首出之后的连锁性动作不覆盖记录）。
  行为：
    有效条件 ax := cx AND enablex（enablex 默认 TRUE；置 FALSE = 旁路该路）。
    tripped := 任一有效条件 OR (已锁存 AND NOT reset)
    ——条件在则常报；条件全清后保持，直到 reset（等效"reset 且全清"
    才能复位；条件未清时按 reset 无效）。
    首出捕获：tripped 由 FALSE→TRUE 的那一拍记录当时动作的最小路号
    （同拍多路同动取小号）。联锁保持期间后续动作不改写首出；
    复位成功（tripped 回 FALSE）时 first_out 清 0。
    cx 语义：TRUE = 该条件已动作（跳闸方向）。常闭型信号请在外部取反。

  输入：
    c1..c8           BOOL  联锁条件（TRUE = 动作/越限）
    enable1..enable8 BOOL  各路投用（默认 TRUE；FALSE = 旁路）
    reset            BOOL  复位（电平有效；仅条件全清后生效）
  输出：
    tripped   BOOL  联锁动作（锁存）
    first_out INT   首出路号 1..8，0 = 无
*)
FUNCTION_BLOCK FB_INTERLOCK8
  VAR_INPUT
    c1      : BOOL := FALSE;
    c2      : BOOL := FALSE;
    c3      : BOOL := FALSE;
    c4      : BOOL := FALSE;
    c5      : BOOL := FALSE;
    c6      : BOOL := FALSE;
    c7      : BOOL := FALSE;
    c8      : BOOL := FALSE;
    enable1 : BOOL := TRUE;
    enable2 : BOOL := TRUE;
    enable3 : BOOL := TRUE;
    enable4 : BOOL := TRUE;
    enable5 : BOOL := TRUE;
    enable6 : BOOL := TRUE;
    enable7 : BOOL := TRUE;
    enable8 : BOOL := TRUE;
    reset   : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    tripped   : BOOL;
    first_out : INT;
  END_VAR
  VAR
    a1    : BOOL; (* 投用后的有效条件 *)
    a2    : BOOL;
    a3    : BOOL;
    a4    : BOOL;
    a5    : BOOL;
    a6    : BOOL;
    a7    : BOOL;
    a8    : BOOL;
    any_c : BOOL;
    was   : BOOL; (* 上一拍 tripped，捕获首出沿 *)
  END_VAR

  a1 := c1 AND enable1;
  a2 := c2 AND enable2;
  a3 := c3 AND enable3;
  a4 := c4 AND enable4;
  a5 := c5 AND enable5;
  a6 := c6 AND enable6;
  a7 := c7 AND enable7;
  a8 := c8 AND enable8;
  any_c := a1 OR a2 OR a3 OR a4 OR a5 OR a6 OR a7 OR a8;

  was := tripped;
  tripped := any_c OR (tripped AND NOT reset);

  IF tripped AND NOT was THEN
    (* 新跳闸事件：记录首出（同拍多路取最小号） *)
    IF a1 THEN
      first_out := 1;
    ELSIF a2 THEN
      first_out := 2;
    ELSIF a3 THEN
      first_out := 3;
    ELSIF a4 THEN
      first_out := 4;
    ELSIF a5 THEN
      first_out := 5;
    ELSIF a6 THEN
      first_out := 6;
    ELSIF a7 THEN
      first_out := 7;
    ELSIF a8 THEN
      first_out := 8;
    END_IF;
  ELSIF NOT tripped THEN
    first_out := 0; (* 复位成功清首出 *)
  END_IF;
END_FUNCTION_BLOCK
