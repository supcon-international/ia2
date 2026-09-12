# Retained-data validation and numerical reproducibility

This is a capture-integrity/recomputation gate, **not a machine acceptance
test**, a measurement-uncertainty estimate, or a cryptographic attestation
of how the samples were acquired. It uses only Python 3.9+ stdlib; no IA2
runtime, hardware, broker, external service, or model is required.

```bash
python3 docs/bench/data/recompute.py
python3 -m unittest discover -s docs/bench/data -p 'test_*.py' -v
```

`evidence.py` specifies each retained CSV's schema, row count and sampling
contract. `recompute.py` validates all of them before printing a headline,
then performs the historical calculations. Invalid/missing evidence and
numeric mismatches return exit 1. The tests inject failures into memory or
temporary copies; none of the retained CSV/journal files is rewritten.

## Capture contracts

Headers must contain exactly the registered columns, without duplicates.
Short/long/blank CSV records, missing required cells, non-finite numbers,
invalid booleans/enums, fractional integer fields, and counters outside
their nonnegative 64-bit domain are rejected. Count/ordering checks are
independent of the published averages: changing an average's tolerance
cannot hide a dropped row or a duplicate counter snapshot.

| CSV (under `data/`) | Rows | Ordering and additional checks |
|---|---:|---|
| scan-cadence-ethercat-only-20260728.csv | 448 | `t` and `ts_us` strictly increase; 447 derivative-rate samples |
| scan-cadence-mixed-bus-20260709.csv | 373 | Same; 372 derivative-rate samples |
| scan-cadence-mixed-bus-postfix-20260908.csv | 600 | `ts_us` strictly increases; `scan_count` never decreases; gap ≤400000 μs |
| soak-4h-20260908.csv | 14400 | `t_unix` and `ts_us` strictly increase; counter never decreases; gap ≤2000000 μs |
| long-uptime-counters-20260908.csv | 101 | Wall time and uptime strictly increase; counter never decreases |
| dual-gear-20260708.csv | 445 | Time never decreases; exactly five annotated duplicate samples retained; phase vocabulary and presence checked |
| valve-calibration-20260701.csv | 42 | Unique up/down × 0..10 V in 0.5 V increments |
| valve-deadband-20260701.csv | 19 | `idx` is exactly 0..18 |
| valve-step-20260701.csv | 1411 | `t_s` strictly increases |
| valve-endpoint-low-20260701.csv | 22 | Valid low/high zone; finite values |
| valve-endpoint-high-20260701.csv | 13 | Valid up/down direction; finite values |

For the two legacy cadence captures, the first `scan_rate` and three RPM
derivatives must be blank: no prior sample exists. Later blanks in these
columns fail. `ratio` and `sync_err_*` may be blank because the older
collector cannot always define them (e.g. stationary motion); these fields
are not used to compute the cadence headline. No other blank is ignored.

The mixed-bus and soak gap caps are **capture continuity criteria** chosen
as twice the nominal 0.2 s and 1 s polling interval, respectively. They
are not measured maximum PLC jitter or a runtime deadline. Current maxima
are 0.305864 s and 1.117798 s. Every positive adjacent interval contributes
to rate statistics, including unusually short ones; there is no `dt > ...`
filter. Samples with reversed/duplicate timestamps cannot be discarded to
make a result pass. Rates are still arithmetic means of interval rates;
the method has not silently changed to time-weighted averaging.

### Existing limitations must stay visible

`dual-gear-20260708.csv` contains five exact adjacent duplicates, at CSV
physical lines **31, 275, 357, 438, 444** (header is line 1), with timestamps
**5.360, 53.624, 69.915, 86.002, 87.010**. The validator accepts only these
documented positions/times, with both entire rows identical. Any additional
duplicate, shifted exception, or nonidentical same-time sample fails.
The 445 rows therefore contain 440 distinct retained samples. The existing
ratios/error calculations retain the old row weighting and window indices;
this patch neither cleans the raw capture nor claims independent samples.
A future statistical reanalysis must be separately versioned.

The endpoint dwell captures do not carry timestamps or independent sample
IDs. Identical values there may be legitimate repeated readings: schema
and row-count checks cannot prove absence of replay, temporal gaps, or
reordered points. Do not invent timing or reject a repeated physical value
as corruption merely to claim universal deduplication.

## Tolerances: publication rounding only

`check()` accepts integer counts/flags **exactly**. Decimal targets accept
half a unit of their last reported place, plus 1e-12 for floating-point
arithmetic. Strings preserve meaningful trailing zeros (`"500.00"`,
`"2.0000"`). This is a deterministic agreement check against the values
printed here/in the bench pages, not a new experimental acceptance band.

| Target precision | Half-unit allowance | Examples |
|---|---:|---|
| Integer count/flag | 0 | unhealthy, watchdog, trip, backoff counts; 7 held transitions; −7708 counts |
| One decimal | 0.05 | cadence 500.0 / 493.6 / 498.8 scans/s; total recovery 48.3 s |
| Two decimals | 0.005 | soak 500.00/s, span 4.05 h; lifetime 499.73/s, 7.85 days; 2.08 / 0.96 s |
| Three decimals | 0.0005 | endpoint/step/hysteresis volts; quantization median 0.192 V |
| Four decimals | 0.00005 | gear ratios including 2.0000; regression slope/intercept |
| Five decimals | 0.000005 | R² = 0.99991 |

Corrections from the earlier loose checks:

- The deadband capture has **7** unchanged-feedback transitions out of
  18 adjacent transitions; it is not “6 ±3”. The nonzero jump median is
  **0.192 V**, which supports the engineering summary “approximately
  0.2 V” without an unexplained ±0.08 V allowance.
- The measured step peak is **9.958 V**, not a 9.95 V target with ±0.02 V.
- The last lifetime-counter row is **7.84532407 days**, rounding to
  **7.85 days** at two decimal places, not the earlier truncated 7.84.
- Log intervals remain runtime-clock observations: cable-change detection
  to recovery is 2.077100 s. The power-cycle 0.956548 s is the interval
  from the last backoff log to recovery minus its scheduled delay, not an
  independently instrumented physical power-return time or guaranteed
  exact walk execution time. Missing event markers/timestamps fail.

The script does not parse arbitrary Markdown claims. Keep published text
and numeric targets synchronized in review. Claims derived only from
human observation, HTTP prose, or configuration do not become verified
because the numeric recomputation exits 0.

## Clean-package gate

After these changes are committed, export that exact commit's
`docs/bench/` into a fresh temporary directory and run both commands there.
The package must include `evidence.py` and `test_recompute.py` as well as
the raw CSVs and the `.txt` journal excerpts. A pass in a worktree with
untracked/ignored support files is not sufficient publication evidence.
