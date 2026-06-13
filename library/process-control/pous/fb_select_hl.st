(*
  FB_SELECT_HL — high/low selector (override selection).

  ~ High/low selector (override selector), the customary block for override control.

  Purpose: override control -- two controller outputs taking the higher/lower for a
  shared actuator (e.g. compressor anti-surge override, a pressure-override-temperature
  loop); can also take the higher/lower of dual transmitters.

  Behavior: select_high = TRUE takes the greater, FALSE takes the lesser;
  a_selected indicates the output currently comes from a (on a tie, a is deemed
  selected so the indication does not chatter).

  Inputs:
    a           REAL  channel a (typically the primary controller output)
    b           REAL  channel b (typically the override controller output)
    select_high BOOL  TRUE = high select / FALSE = low select
  Outputs:
    out        REAL  selected value
    a_selected BOOL  TRUE = the output currently comes from a
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
