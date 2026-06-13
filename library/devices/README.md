# Device catalog

One file per directory entry: a field-validated device template. The
`/api/devices` creation flow and EtherCAT bus discovery (`/discover`) use
it to turn "hand-type the channels" into "recognise → pre-fill → adjust":
when a discovered `vendor_id`/`product_id` matches a template, the device's
slaves / channels / DC requirements are generated straight from it, instead
of transcribing byte offsets off the logs.

## Entry format

Same shape as a project `devices/*.toml`, minus the site-specific fields
(`nic`), plus metadata:

```toml
name = "InoSV660N"             # default device name (editable)
protocol = "ethercat"
description = "…"              # one-line summary
vendor_id = 1048576             # identity key (EtherCAT ESI identity)
product_id = 786701
requires_dc_sync = "sync0"     # hard requirement: without it the slave never reaches OP
recommended_cycle_us = 2000

[[channels]]                    # PDO/register template, the validated layout
…
```

Rules:

- **Only validated templates** — an entry must have run on real hardware
  (connection + OP + correct data) before it goes in the catalog; record
  the source (which bench, which project) in the file's header comment.
- No customer/project information; the device vendor and public model
  number are hardware facts and are allowed.
- Modbus devices likewise (recognised by the engineer picking the model,
  no bus discovery): register table + word order + recommended poll
  interval.
