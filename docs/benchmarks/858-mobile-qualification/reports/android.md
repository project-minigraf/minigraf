# Mobile qualification decisions

Computed decisions; provenance and study completion still require evidence review.

Medians are ns; CV and delta are fractions. A/A lists p(B1>B2), p(B2>B1).

| Candidate / case | Category | B1 median phys / sim | Ratio | CV phys / sim | Delta phys / sim | p(V>B1) phys / sim | A/A physical | A/A simulated | Outcome / reason |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| android-emulator-v1 / aggregate/1k | compute/callback | 7.358400115e+07 / 3.235926298e+07 | 4.397594921e-01 | 6.171747741e-02 / 2.467573100e-01 | 1.043312499e+00 / 9.580154131e-01 | 4.493900428e-10 / 2.649463390e-04 | 7.193256522e-01, 2.806743478e-01 | 1.925142008e-01, 8.074857992e-01 | rejected: closeness; cv_absolute; cv_relative |
| android-emulator-v1 / cursor_batch_1/1k | allocation/conversion | 1.048979502e+08 / 4.345714853e+07 | 4.142802450e-01 | 6.269255514e-02 / 2.011294494e-01 | 9.117875858e-01 / 1.022400140e+00 | 5.578039503e-09 / 5.012933220e-05 | 5.714030188e-01, 4.285969812e-01 | 2.342110707e-01, 7.657889293e-01 | rejected: closeness; cv_absolute; cv_relative |
| android-emulator-v1 / cursor_batch_256/1k | allocation/conversion | 2.522654713e+06 / 1.387294275e+06 | 5.499342689e-01 | 1.144578226e-01 / 4.161492374e-02 | 9.777349186e-01 / 9.554573146e-01 | 7.169331484e-07 / 1.390782790e-11 | 8.559397976e-02, 9.144060202e-01 | 4.382739927e-01, 5.617260073e-01 | rejected: closeness |
| android-emulator-v1 / fact_log_batch_256/1k | allocation/conversion | 1.811980138e+06 / 1.006720075e+06 | 5.555911205e-01 | 5.324415129e-02 / 3.858784506e-02 | 1.076566285e+00 / 9.945105023e-01 | 1.676012917e-09 / 1.765090186e-06 | 6.688597872e-01, 3.311402128e-01 | 1.893509518e-01, 8.106490482e-01 | rejected: closeness |
| android-emulator-v1 / file_transact/1k | storage/startup | 9.592651500e+06 / 2.947968750e+06 | 3.073153184e-01 | 1.043701339e-01 / 7.183077422e-02 | 1.054198519e+00 / 1.014769322e+00 | 6.939104014e-05 / 6.715635650e-08 | 9.122364786e-01, 8.776352141e-02 | 2.070381698e-01, 7.929618302e-01 | rejected: closeness |
| android-emulator-v1 / log_writer_append_finish/1k | storage/startup | 2.341970825e+07 / 9.136256250e+06 | 3.901097380e-01 | 4.782668655e-02 / 1.930640890e-02 | 9.513141672e-01 / 1.013160889e+00 | 2.398655042e-09 / 4.412554179e-12 | 9.299029895e-01, 7.009701051e-02 | 4.215526756e-01, 5.784473244e-01 | rejected: closeness |
| android-emulator-v1 / options_error/negative_cache | allocation/conversion | 2.092544875e+05 / 8.702965000e+04 | 4.159033865e-01 | 5.707093290e-02 / 5.030343615e-02 | 9.695724566e-01 / 9.460893500e-01 | 5.991721045e-09 / 1.352938705e-10 | 7.816979137e-01, 2.183020863e-01 | 3.132942892e-01, 6.867057108e-01 | rejected: closeness |
| android-emulator-v1 / options_readonly_open/1k | storage/startup | 2.443755500e+05 / 1.242984625e+05 | 5.086370650e-01 | 5.113342681e-02 / 1.706966496e-02 | 1.072008840e+00 / 8.463746082e-01 | 2.984883459e-08 / 3.088675791e-06 | 8.325643684e-01, 1.674356316e-01 | 5.533155313e-01, 4.466844687e-01 | rejected: closeness |
| android-emulator-v1 / predicate/1k | compute/callback | 7.714814866e+07 / 3.356018907e+07 | 4.350096491e-01 | 4.712508306e-02 / 1.301818402e-01 | 1.064369050e+00 / 9.729367615e-01 | 7.849990231e-09 / 3.911421702e-06 | 6.660675111e-01, 3.339324889e-01 | 2.273118412e-01, 7.726881588e-01 | rejected: closeness; cv_relative |
| android-emulator-v1 / prepare/point | compute/callback | 1.510574750e+05 / 6.412963750e+04 | 4.245379946e-01 | 5.404105388e-02 / 5.233459302e-02 | 1.024896649e+00 / 8.941201328e-01 | 7.810394157e-09 / 5.678592548e-10 | 8.249655048e-01, 1.750344952e-01 | 6.997955388e-01, 3.002044612e-01 | rejected: closeness |
| android-emulator-v1 / prepared_execute/1k | allocation/conversion | 1.192826925e+06 / 6.379798375e+05 | 5.348469456e-01 | 8.703405250e-02 / 1.031397860e-01 | 7.557931131e-01 / 8.543317405e-01 | 8.458401090e-08 / 2.558719934e-05 | 3.846220201e-01, 6.153779799e-01 | 2.848890916e-01, 7.151109084e-01 | rejected: closeness |
| android-emulator-v1 / register_aggregate/first | compute/callback | 2.284846000e+05 / 8.789402500e+04 | 3.846824906e-01 | 1.395538803e-01 / 5.503418997e-02 | 7.017447237e-01 / 6.182344306e-01 | 3.397064446e-05 / 2.579471496e-09 | 5.024556683e-01, 4.975443317e-01 | 1.512893943e-01, 8.487106057e-01 | rejected: closeness |
| android-emulator-v1 / register_predicate/first | compute/callback | 2.306742625e+05 / 8.327780000e+04 | 3.610190365e-01 | 1.820752675e-01 / 1.792753967e-02 | 7.402364622e-01 / 6.366582691e-01 | 1.978201640e-05 / 8.540926474e-11 | 5.760722277e-01, 4.239277723e-01 | 9.702936205e-01, 2.970637946e-02 | rejected: closeness |
| android-emulator-v1 / set_roundtrip/2 | allocation/conversion | 2.778513750e+05 / 1.134070875e+05 | 4.081573737e-01 | 7.001662079e-02 / 1.345670887e-01 | 9.839351614e-01 / 9.189900719e-01 | 9.816138789e-06 / 3.392438664e-07 | 6.338813288e-01, 3.661186712e-01 | 4.079066146e-01, 5.920933854e-01 | rejected: closeness |

Category counts only; they never override a case.

| Candidate | Category | Qualified | Rejected | Inconclusive |
| --- | --- | --- | --- | --- |
| android-emulator-v1 | allocation/conversion | 0 | 6 | 0 |
| android-emulator-v1 | compute/callback | 0 | 5 | 0 |
| android-emulator-v1 | storage/startup | 0 | 3 | 0 |
