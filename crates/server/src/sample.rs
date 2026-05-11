pub const SAMPLE_NAME: &str = "Demo / plc_counter";

pub const SAMPLE_SOURCE: &str = r#"(*
    Demo program for the controlsoftware MVP.
    Increments `counter` every task scan (100 ms). When counter reaches 50,
    resets to zero and flips `blink` (so blink toggles every ~5 seconds).
*)
PROGRAM Demo
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
        PROGRAM plc_task_instance WITH plc_task : Demo;
    END_RESOURCE
END_CONFIGURATION
"#;
