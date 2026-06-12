# supervisory-demo — IA2 above an existing DCS

The DCS-supervisory architecture end to end on one machine:

```
Tier0 / supOS  ⇄ MQTT ⇄  IA2 runtime (this project)  ⇄ OPC UA ⇄  DCS (fake)
```

The ST program is a slice of a real jet-mill spec: the classifier motor
current back-controls the feeder with hysteresis (>40 A stop, <30 A run),
and a platform-written flow setpoint passes through to a DCS valve command.

Run (three terminals, repo root):

```bash
mosquitto                                              # broker on :1883
cargo run -p iomap-opcua --example fake_dcs            # fake DCS on :4840
cargo run -p ia2-runtime -- --project-dir examples/supervisory-demo
```

Watch / poke:

```bash
mosquitto_sub -t 'ia2/supervisory_demo/snapshot'                       # live values
mosquitto_pub -t 'ia2/supervisory_demo/write' \
              -m '{"name":"sp_flow","value":12.5}'                     # platform setpoint
# fake_dcs prints "[fake-dcs] FV0203_CMD written -> 12.500" when IA2
# pushes it through, and FEEDER_RUN toggles as the hysteresis trips.
```
