(*=====================================================================
  nx6_modbus — NX6-1261-RTU I/O commissioning program

  Goal: exercise the photoelectric switch (DI) and the motorized ball
  valve (DO on/off + AO 0-10V control with AI position feedback).
  Setpoints (valve_on / ao1_setpoint) are injected via the runtime
  force API; observations (sensor0 / di_word / ai1..4) via /status.

  Notes:
   - DI/DO are bits inside a register word (the coupler only supports
     FC 3/6/16), so use WORD bit operations to read/set them.
   - 0-10V ↔ 0-32000 (3200 counts/V, measured); ao1_setpoint 16000 ≈ 5V.
   - The output register map is the 2026-07-01 field-verified one — it
     differs from the vendor manual; see devices/coupler.toml.
   - ao1 = terminal V1/M1 (valve control). ao4 = terminal V0/M0, whose
     DAC is dead on this unit (stuck at 0V) — do not use.
=====================================================================*)
PROGRAM main
  VAR
    (* ===== iomap bindings (NX6-1261-RTU) ===== *)
    di_word : WORD;                       (* ID16N 16 DI  @ HR40001 *)
    ai1 : INT; ai2 : INT; ai3 : INT; ai4 : INT;   (* AD4V-D 4 AI @ HR40065-68; ai1 = valve feedback *)
    do_word : WORD;                       (* OD16N 16 DO  @ HR40193 [measured, not 40033!] *)
    ao1 : INT; ao2 : INT; ao3 : INT; ao4 : INT;   (* DA4VC-D @ HR40194-97 → V1/V2/V3/V0 *)

    (* ===== setpoints (runtime force) ===== *)
    valve_on     : BOOL;     (* drives OD16N channel 0 *)
    ao1_setpoint : INT;      (* raw counts for terminal V1, 0..32000 = 0..10V *)

    (* ===== observations ===== *)
    sensor0   : BOOL;        (* E3ZG photoelectric switch on ID16N channel 0 *)
    heartbeat : UDINT;
  END_VAR

  heartbeat := heartbeat + 1;

  (* 1) read the photoelectric switch = DI channel 0 (bit 0 of di_word) *)
  sensor0 := (di_word AND WORD#16#0001) <> WORD#16#0000;

  (* 2) drive the DO on/off channel 0 (only ch0 used here, whole-word write) *)
  IF valve_on THEN
    do_word := WORD#16#0001;
  ELSE
    do_word := WORD#16#0000;
  END_IF;

  (* 3) drive the valve 0-10V = AO terminal V1 *)
  ao1 := ao1_setpoint;

  (* ao2..ao4 stay 0; watch ai1..ai4 via /status *)
END_PROGRAM
