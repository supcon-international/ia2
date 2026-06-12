(*
  FB_SELECT_HL — 高/低选择器（超驰选择）。

  ≈ 高/低选择器（override selector），超驰控制惯用块。

  用途：超驰（override）控制——两个控制器输出取高/取低共用一个执行机构
  （如压缩机防喘振超驰、压力超驰温度回路）；亦可做双变送器取高/取低。

  行为：select_high = TRUE 取大者，FALSE 取小者；
  a_selected 指示当前输出取自 a（相等时判 a 选中，指示不抖动）。

  输入：
    a           REAL  通道 a（常接主控制器输出）
    b           REAL  通道 b（常接超驰控制器输出）
    select_high BOOL  TRUE = 高选 / FALSE = 低选
  输出：
    out        REAL  选中值
    a_selected BOOL  TRUE = 当前输出取自 a
*)
FUNCTION_BLOCK FB_SELECT_HL
  VAR_INPUT
    a           : REAL;
    b           : REAL;
    select_high : BOOL := TRUE;
  END_VAR
  VAR_OUTPUT
    out        : REAL;
    a_selected : BOOL;
  END_VAR

  IF select_high THEN
    IF a >= b THEN
      out := a;
      a_selected := TRUE;
    ELSE
      out := b;
      a_selected := FALSE;
    END_IF;
  ELSE
    IF a <= b THEN
      out := a;
      a_selected := TRUE;
    ELSE
      out := b;
      a_selected := FALSE;
    END_IF;
  END_IF;
END_FUNCTION_BLOCK
