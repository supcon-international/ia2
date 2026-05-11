(*
    Default starter program. Counter increments every 100ms task scan;
    blink flips every 50 cycles.

    Map `counter` or `blink` to a device channel in the IO Mapping view
    to drive Modbus / EtherCAT outputs.
*)
PROGRAM main
    VAR
        counter: INT;
        blink: BOOL;
    END_VAR

    counter := counter + 1;
    IF counter >= 50 THEN
        counter := 0;
        blink := NOT blink;
    END_IF;
END_PROGRAM

CONFIGURATION config
    RESOURCE plc_res ON PLC
        TASK plc_task(INTERVAL := T#100ms, PRIORITY := 1);
        PROGRAM plc_task_instance WITH plc_task : main;
    END_RESOURCE
END_CONFIGURATION
