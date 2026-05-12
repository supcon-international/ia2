(*
    Default starter program. Counter increments each scan; blink flips
    every 50 cycles. The scan period is set in the project's Tasks pane
    (defaults to 100 ms).

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
