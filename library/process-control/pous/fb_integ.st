(*
  FB_INTEG — general-purpose integrator (with hold/reset/preset and output clamp).

  ~ Standard integrator block, a staple of every major platform's standard library.

  Purpose: deviation integration (monitoring accumulated quantities such as degC*s),
  simple material/energy totals, the integral element in plant simulation, the
  integral path of a custom control algorithm.
  Difference from FB_TOTALIZER: TOTALIZER is a flow-totalizing block fixed at /3600;
  this block is general integration out := out + k * u * dt_s, with hold/preset/clamp.
  Behavior (command priority reset > preset > hold > integrate):
    reset  level-active: out := 0, limit flags cleared;
    preset level-active: out := preset_value (then clamped as usual);
    hold   level-active: integration paused, output held (clamp and limit flags still refresh);
    otherwise: out := out + k * u * dt_s, clamped to [out_min, out_max].
    q_hi / q_lo indicate the output at the high/low limit (anti-windup: at a limit, a
    reverse input exits immediately).
    dt_s <= 0 -> hold this scan (reset/preset are level commands, not subject to this).

  Inputs:
    u            REAL  integrand input
    k            REAL  integral gain (1/s; dimension defined by the caller)
    dt_s         REAL  sample period s
    hold         BOOL  pause integration (level-active)
    reset        BOOL  reset to 0 (level-active, highest priority)
    preset       BOOL  preset (level-active, takes priority over hold)
    preset_value REAL  preset value
    out_min      REAL  output low limit (default -1.0E38 ~ unlimited)
    out_max      REAL  output high limit (default +1.0E38 ~ unlimited)
  Outputs:
    out  REAL  integral output (clamped)
    q_hi BOOL  at high limit
    q_lo BOOL  at low limit
*)
FUNCTION_BLOCK FB_INTEG
  VAR_INPUT
    u            : REAL;
    k            : REAL := 1.0;
    dt_s         : REAL := 0.1;
    hold         : BOOL := FALSE;
    reset        : BOOL := FALSE;
    preset       : BOOL := FALSE;
    preset_value : REAL := 0.0;
    out_min      : REAL := -1.0E38;
    out_max      : REAL := 1.0E38;
  END_VAR
  VAR_OUTPUT
    out  : REAL;
    q_hi : BOOL;
    q_lo : BOOL;
  END_VAR

  IF reset THEN
    out := 0.0;
    q_hi := FALSE;
    q_lo := FALSE;
    RETURN;
  END_IF;

  IF preset THEN
    out := preset_value; (* after preset, fall through to the clamp below as usual *)
  ELSIF NOT hold THEN
    IF dt_s <= 0.0 THEN
      RETURN; (* invalid sample period: hold *)
    END_IF;
    out := out + k * u * dt_s;
  END_IF;
  (* While hold is active: no integration, just refresh the clamp and limit flags as usual *)

  q_hi := FALSE;
  q_lo := FALSE;
  IF out >= out_max THEN
    out := out_max;
    q_hi := TRUE;
  ELSIF out <= out_min THEN
    out := out_min;
    q_lo := TRUE;
  END_IF;
END_FUNCTION_BLOCK
