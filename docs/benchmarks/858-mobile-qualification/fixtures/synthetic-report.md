# Mobile qualification decisions

**SYNTHETIC — NOT REAL DATA; tests only.**

Medians are ns; CV and delta are fractions. A/A lists p(B1>B2), p(B2>B1).

| Candidate / case | Category | B1 median phys / sim | Ratio | CV phys / sim | Delta phys / sim | p(V>B1) phys / sim | A/A physical | A/A simulated | Outcome / reason |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| synthetic-simulated / aggregate/1k | compute/callback | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / cursor_batch_1/1k | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / cursor_batch_256/1k | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / fact_log_batch_256/1k | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / file_transact/1k | storage/startup | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / log_writer_append_finish/1k | storage/startup | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / options_error/negative_cache | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / options_readonly_open/1k | storage/startup | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / predicate/1k | compute/callback | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / prepare/point | compute/callback | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / prepared_execute/1k | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / register_aggregate/first | compute/callback | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / register_predicate/first | compute/callback | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |
| synthetic-simulated / set_roundtrip/2 | allocation/conversion | 1.000000000e+02 / 1.000000000e+02 | 1.000000000e+00 | 1.414213562e-02 / 1.414213562e-02 | 1.000000000e+00 / 1.000000000e+00 | 2.795557243e-12 / 2.795557243e-12 | 5.000000000e-01, 5.000000000e-01 | 5.000000000e-01, 5.000000000e-01 | qualified: all gates passed |

Category counts only; they never override a case.

| Candidate | Category | Qualified | Rejected | Inconclusive |
| --- | --- | --- | --- | --- |
| synthetic-simulated | allocation/conversion | 6 | 0 | 0 |
| synthetic-simulated | compute/callback | 5 | 0 | 0 |
| synthetic-simulated | storage/startup | 3 | 0 | 0 |
