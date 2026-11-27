# Mobile qualification decisions

Computed decisions; provenance and study completion still require evidence review.

Medians are ns; CV and delta are fractions. A/A lists p(B1>B2), p(B2>B1).

| Candidate / case | Category | B1 median phys / sim | Ratio | CV phys / sim | Delta phys / sim | p(V>B1) phys / sim | A/A physical | A/A simulated | Outcome / reason |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ios-simulator-v1 / aggregate/1k | compute/callback | 1.243316150e+06 / 1.780693200e+06 | 1.432212716e+00 | 9.145650717e-02 / 1.898248214e-01 | 1.026218512e+00 / 1.135606150e+00 | 9.897068461e-05 / 1.038662620e-03 | 4.522644774e-01, 5.477355226e-01 | 8.158450400e-01, 1.841549600e-01 | rejected: cv_absolute; cv_relative; detection_simulated |
| ios-simulator-v1 / cursor_batch_1/1k | allocation/conversion | 3.083167700e+06 / 4.730376575e+06 | 1.534258605e+00 | 9.912876213e-02 / 2.566333461e-01 | 1.070618329e+00 / 1.223654897e+00 | 1.272069077e-04 / 2.396597817e-04 | 4.338979929e-01, 5.661020071e-01 | 3.095022271e-01, 6.904977729e-01 | rejected: closeness; cv_absolute; cv_relative |
| ios-simulator-v1 / cursor_batch_256/1k | allocation/conversion | 9.796817500e+05 / 1.676904650e+06 | 1.711683054e+00 | 1.040088403e-01 / 2.527923241e-01 | 1.010661396e+00 / 8.411601324e-01 | 1.672799925e-04 / 7.579804174e-04 | 3.229128235e-01, 6.770871765e-01 | 4.884448688e-01, 5.115551312e-01 | rejected: closeness; cv_absolute; cv_relative |
| ios-simulator-v1 / fact_log_batch_256/1k | allocation/conversion | 3.003742150e+06 / 3.667892163e+06 | 1.221107532e+00 | 9.226559617e-02 / 1.710681545e-01 | 1.069721035e+00 / 7.432007381e-01 | 1.033260815e-04 / 1.151943659e-03 | 4.351247757e-01, 5.648752243e-01 | 1.676156960e-01, 8.323843040e-01 | rejected: cv_absolute; detection_simulated |
| ios-simulator-v1 / file_transact/1k | storage/startup | 7.988135750e+06 / 6.367802000e+06 | 7.971574594e-01 | 8.489691456e-02 / 2.683581348e-01 | 8.854615734e-01 / 5.838357176e-01 | 1.599282397e-07 / 5.443798176e-03 | 7.100794590e-01, 2.899205410e-01 | 7.421540393e-02, 9.257845961e-01 | rejected: cv_absolute; cv_relative; detection_simulated |
| ios-simulator-v1 / log_writer_append_finish/1k | storage/startup | 1.158536450e+07 / 9.176687250e+06 | 7.920930973e-01 | 1.258154070e-01 / 1.994910934e-01 | 8.225356440e-01 / 7.578125483e-01 | 5.050033645e-08 / 1.238697830e-03 | 7.437453992e-01, 2.562546008e-01 | 6.138366138e-01, 3.861633862e-01 | rejected: cv_absolute; detection_simulated |
| ios-simulator-v1 / options_error/negative_cache | allocation/conversion | 4.024015000e+04 / 3.671657500e+04 | 9.124363354e-01 | 1.083034549e-01 / 5.272656300e-01 | 1.046721372e+00 / 4.065796578e-01 | 1.231321870e-04 / 1.106764374e-01 | 6.117313216e-01, 3.882686784e-01 | 1.072918663e-01, 8.927081337e-01 | rejected: cv_absolute; cv_relative; delta_agreement; detection_simulated |
| ios-simulator-v1 / options_readonly_open/1k | storage/startup | 4.433677250e+05 / 2.197739125e+05 | 4.956921763e-01 | 9.121340472e-02 / 2.368296071e-01 | 1.057153715e+00 / 6.132442471e-01 | 1.308611728e-04 / 1.542698615e-02 | 4.704110625e-01, 5.295889375e-01 | 3.222359289e-02, 9.677764071e-01 | rejected: closeness; cv_absolute; cv_relative; detection_simulated |
| ios-simulator-v1 / predicate/1k | compute/callback | 1.678862500e+06 / 2.561968763e+06 | 1.526014645e+00 | 9.641438376e-02 / 1.777677281e-01 | 9.946708709e-01 / 8.616196828e-01 | 1.001378519e-04 / 6.442454475e-05 | 3.661895090e-01, 6.338104910e-01 | 8.755039643e-01, 1.244960357e-01 | rejected: closeness; cv_absolute |
| ios-simulator-v1 / prepare/point | compute/callback | 2.281750000e+03 / 5.266175000e+03 | 2.307954421e+00 | 9.872965949e-02 / 3.156477386e-01 | 1.074427523e+00 / 6.028948905e-01 | 4.008421772e-04 / 2.518849763e-02 | 4.255940360e-01, 5.744059640e-01 | 7.634527434e-01, 2.365472566e-01 | rejected: closeness; cv_absolute; cv_relative; detection_simulated |
| ios-simulator-v1 / prepared_execute/1k | allocation/conversion | 2.246443125e+05 / 4.441692625e+05 | 1.977211253e+00 | 8.739595324e-02 / 3.664267727e-01 | 1.011302367e+00 / 1.145741512e+00 | 3.337395349e-04 / 1.761072096e-03 | 8.308466703e-01, 1.691533297e-01 | 2.849501530e-01, 7.150498470e-01 | rejected: closeness; cv_absolute; cv_relative; detection_simulated |
| ios-simulator-v1 / register_aggregate/first | compute/callback | 2.181262500e+03 / 1.311558750e+04 | 6.012842333e+00 | 2.826454370e-01 / 2.439924156e-01 | 6.766723400e-01 / 1.458093280e-02 | 6.995334023e-04 / 2.173177541e-01 | 1.734075351e-01, 8.265924649e-01 | 4.302692194e-02, 9.569730781e-01 | rejected: closeness; cv_absolute; delta_agreement; detection_simulated |
| ios-simulator-v1 / register_predicate/first | compute/callback | 1.737987500e+03 / 1.037863750e+04 | 5.971641050e+00 | 2.924433770e-01 / 2.018646313e-01 | 8.900164702e-01 / 2.090604378e-01 | 4.002284361e-04 / 8.393228458e-02 | 3.320684475e-01, 6.679315525e-01 | 1.554707117e-01, 8.445292883e-01 | rejected: closeness; cv_absolute; delta_agreement; detection_simulated |
| ios-simulator-v1 / set_roundtrip/2 | allocation/conversion | 1.281720000e+04 / 2.036303750e+04 | 1.588727452e+00 | 4.815057225e-01 / 7.395893278e-01 | 1.141083856e+00 / 5.816255311e-01 | 4.462637049e-03 / 1.485128855e-01 | 1.727447054e-01, 8.272552946e-01 | 5.268733121e-01, 4.731266879e-01 | rejected: closeness; cv_absolute; detection_physical; detection_simulated |

Category counts only; they never override a case.

| Candidate | Category | Qualified | Rejected | Inconclusive |
| --- | --- | --- | --- | --- |
| ios-simulator-v1 | allocation/conversion | 0 | 6 | 0 |
| ios-simulator-v1 | compute/callback | 0 | 5 | 0 |
| ios-simulator-v1 | storage/startup | 0 | 3 | 0 |
