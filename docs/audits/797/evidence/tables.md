Means, deltas and percentiles are medians over blocks, in ns. Δ is the candidate's median block mean minus the base's (for `cc_*`, wall ns per operation). Allocations and RSS are n/a where the harness cannot measure them: browser (both) and WASI (RSS).

### primary-1-envelope

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
pt_asof | B:B | 128 | 1.0006 | 1.0046 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 14,192.0 | 14,223.8 | +31.8 | 15,327/24,458 | 15,298/24,492 | 212.1→212.1 | 5,816→5,828
pt_asof | B:H | 128 | 1.0130 | 1.0179 | 1.02 | pass | digest of complete results; timed reads row counts | 14,234.0 | 14,501.5 | +267.5 | 15,382/24,510 | 15,560/24,838 | 212.1→212.1 | 5,824→5,780
pt_asof | B:P | 128 | 0.9951 | 0.9998 | 1.02 | pass | digest of complete results; timed reads row counts | 14,262.9 | 14,222.4 | -40.5 | 15,358/24,515 | 15,282/24,508 | 212.1→212.1 | 5,832→5,780
pt_av | B:B | 128 | 1.0009 | 1.0050 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 16,835.4 | 16,838.4 | +3.0 | 16,317/22,888 | 16,292/23,204 | 254.2→254.2 | 5,670→5,696
pt_av | B:H | 128 | 1.0130 | 1.0172 | 1.02 | pass | digest of complete results; timed reads row counts | 16,904.3 | 17,168.7 | +264.4 | 16,342/23,210 | 16,524/23,632 | 254.2→254.2 | 5,704→5,652
pt_av | B:P | 128 | 0.9994 | 1.0039 | 1.02 | pass | digest of complete results; timed reads row counts | 16,903.6 | 16,866.7 | -36.9 | 16,382/23,122 | 16,307/23,300 | 254.2→254.2 | 5,708→5,644
pt_ea | B:B | 128 | 0.9999 | 1.0039 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 14,183.4 | 14,155.5 | -27.9 | 15,814/24,283 | 15,786/24,232 | 202.3→202.3 | 5,782→5,800
pt_ea | B:H | 128 | 1.0112 | 1.0155 | 1.02 | pass | digest of complete results; timed reads row counts | 14,208.1 | 14,391.0 | +182.8 | 15,830/24,297 | 15,972/24,600 | 202.3→202.3 | 5,810→5,760
pt_ea | B:P | 128 | 0.9928 | 0.9971 | 1.02 | pass | digest of complete results; timed reads row counts | 14,235.8 | 14,135.4 | -100.4 | 15,850/24,314 | 15,728/24,171 | 202.3→202.3 | 5,816→5,760

### primary-1-release

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
pt_asof | B:B | 128 | 1.0003 | 1.0043 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 22,777.0 | 22,791.4 | +14.5 | 24,656/39,083 | 24,639/39,066 | 212.1→212.1 | 4,848→4,840
pt_asof | B:H | 128 | 1.0012 | 1.0049 | 1.02 | pass | digest of complete results; timed reads row counts | 22,829.3 | 22,865.2 | +36.0 | 24,672/39,250 | 24,610/39,620 | 212.1→212.1 | 4,844→4,830
pt_asof | B:P | 128 | 1.0081 | 1.0128 | 1.02 | pass | digest of complete results; timed reads row counts | 22,763.5 | 22,912.3 | +148.8 | 24,614/39,035 | 24,554/39,316 | 212.1→212.1 | 4,844→4,830
pt_av | B:B | 128 | 1.0027 | 1.0070 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 26,365.5 | 26,408.2 | +42.8 | 25,538/33,681 | 25,564/33,894 | 254.2→254.2 | 4,770→4,768
pt_av | B:H | 128 | 1.0017 | 1.0058 | 1.02 | pass | digest of complete results; timed reads row counts | 26,446.3 | 26,479.8 | +33.4 | 25,634/33,760 | 25,798/34,066 | 254.2→254.2 | 4,768→4,756
pt_av | B:P | 128 | 1.0104 | 1.0161 | 1.02 | pass | digest of complete results; timed reads row counts | 26,423.9 | 26,680.4 | +256.5 | 25,620/33,880 | 25,829/34,538 | 254.2→254.2 | 4,764→4,754
pt_ea | B:B | 128 | 0.9994 | 1.0032 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 22,510.2 | 22,516.5 | +6.3 | 24,816/38,710 | 24,816/38,676 | 202.3→202.3 | 4,832→4,828
pt_ea | B:H | 128 | 1.0033 | 1.0067 | 1.02 | pass | digest of complete results; timed reads row counts | 22,497.6 | 22,590.3 | +92.8 | 24,782/38,757 | 24,856/39,074 | 202.3→202.3 | 4,832→4,816
pt_ea | B:P | 128 | 1.0099 | 1.0147 | 1.02 | pass | digest of complete results; timed reads row counts | 22,502.9 | 22,732.2 | +229.3 | 24,797/38,778 | 24,869/39,330 | 202.3→202.3 | 4,824→4,820

### primary-2-envelope

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
pt_asof | B:B | 128 | 0.9979 | 1.0021 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 14,192.0 | 14,191.2 | -0.8 | 15,328/24,490 | 15,334/24,418 | 212.1→212.1 | 5,786→5,822
pt_asof | B:H | 128 | 1.0221 | 1.0260 | 1.02 | FAIL | digest of complete results; timed reads row counts | 14,178.0 | 14,478.4 | +300.3 | 15,310/24,464 | 15,578/24,822 | 212.1→212.1 | 5,820→5,780
pt_asof | B:P | 128 | 0.9935 | 0.9979 | 1.02 | pass | digest of complete results; timed reads row counts | 14,219.5 | 14,103.2 | -116.3 | 15,359/24,532 | 15,198/24,249 | 212.1→212.1 | 5,832→5,792
pt_av | B:B | 128 | 0.9985 | 1.0022 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 16,867.2 | 16,852.6 | -14.6 | 16,285/22,886 | 16,298/22,930 | 254.2→254.2 | 5,660→5,698
pt_av | B:H | 128 | 1.0211 | 1.0245 | 1.02 | FAIL | digest of complete results; timed reads row counts | 16,839.9 | 17,174.7 | +334.8 | 16,309/23,274 | 16,593/23,349 | 254.2→254.2 | 5,690→5,652
pt_av | B:P | 128 | 1.0016 | 1.0055 | 1.02 | pass | digest of complete results; timed reads row counts | 16,876.4 | 16,866.3 | -10.1 | 16,328/23,033 | 16,296/22,850 | 254.2→254.2 | 5,700→5,652
pt_ea | B:B | 128 | 0.9979 | 1.0014 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 14,187.3 | 14,174.9 | -12.4 | 15,798/24,266 | 15,796/24,247 | 202.3→202.3 | 5,768→5,806
pt_ea | B:H | 128 | 1.0167 | 1.0199 | 1.02 | pass | digest of complete results; timed reads row counts | 14,164.4 | 14,373.9 | +209.5 | 15,814/24,222 | 15,939/24,568 | 202.3→202.3 | 5,804→5,762
pt_ea | B:P | 128 | 0.9940 | 0.9978 | 1.02 | pass | digest of complete results; timed reads row counts | 14,181.0 | 14,096.2 | -84.8 | 15,839/24,242 | 15,684/24,074 | 202.3→202.3 | 5,810→5,772

### primary-2-release

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
pt_asof | B:B | 128 | 0.9966 | 1.0007 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 22,862.3 | 22,822.8 | -39.5 | 24,647/39,362 | 24,616/39,280 | 212.1→212.1 | 4,838→4,846
pt_asof | B:H | 128 | 1.0008 | 1.0044 | 1.02 | pass | digest of complete results; timed reads row counts | 22,735.6 | 22,791.6 | +56.0 | 24,620/39,029 | 24,616/39,330 | 212.1→212.1 | 4,844→4,840
pt_asof | B:P | 128 | 1.0024 | 1.0062 | 1.02 | pass | digest of complete results; timed reads row counts | 22,748.3 | 22,798.2 | +49.8 | 24,606/39,024 | 24,434/39,130 | 212.1→212.1 | 4,844→4,836
pt_av | B:B | 128 | 0.9975 | 1.0017 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 26,427.2 | 26,385.5 | -41.7 | 25,598/33,943 | 25,546/33,630 | 254.2→254.2 | 4,768→4,776
pt_av | B:H | 128 | 1.0034 | 1.0071 | 1.02 | pass | digest of complete results; timed reads row counts | 26,410.8 | 26,448.5 | +37.7 | 25,534/33,848 | 25,700/33,911 | 254.2→254.2 | 4,768→4,764
pt_av | B:P | 128 | 1.0062 | 1.0099 | 1.02 | pass | digest of complete results; timed reads row counts | 26,432.8 | 26,568.2 | +135.4 | 25,618/33,700 | 25,694/34,134 | 254.2→254.2 | 4,768→4,762
pt_ea | B:B | 128 | 0.9969 | 1.0003 | ±2% control | resolves ±2% | digest of complete results; timed reads row counts | 22,517.8 | 22,475.4 | -42.3 | 24,803/38,734 | 24,764/38,660 | 202.3→202.3 | 4,828→4,836
pt_ea | B:H | 128 | 1.0060 | 1.0092 | 1.02 | pass | digest of complete results; timed reads row counts | 22,478.0 | 22,615.0 | +136.9 | 24,778/38,654 | 24,867/39,048 | 202.3→202.3 | 4,832→4,824
pt_ea | B:P | 128 | 1.0053 | 1.0088 | 1.02 | pass | digest of complete results; timed reads row counts | 22,506.4 | 22,595.7 | +89.3 | 24,791/38,644 | 24,760/38,942 | 202.3→202.3 | 4,828→4,820

### browser

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
br_join_2 | B:H | 32 | 1.0057 | 1.0170 | 1.05 | pass | digest of complete results; timed reads row counts | 125,519.5 | 125,131.6 | -387.9 | 122,565/158,450 | 122,495/158,680 | n/a | n/a
br_join_2 | B:P | 32 | 1.0071 | 1.0163 | 1.05 | pass | digest of complete results; timed reads row counts | 126,118.0 | 125,911.2 | -206.8 | 122,830/160,203 | 123,144/158,972 | n/a | n/a
br_pt_av | B:H | 32 | 1.0073 | 1.0165 | 1.05 | pass | digest of complete results; timed reads row counts | 57,908.8 | 57,822.2 | -86.6 | 59,818/82,424 | 60,089/81,006 | n/a | n/a
br_pt_av | B:P | 32 | 0.9950 | 1.0009 | 1.05 | pass | digest of complete results; timed reads row counts | 57,902.4 | 57,724.4 | -178.0 | 60,079/81,252 | 59,949/83,507 | n/a | n/a
br_pt_ea | B:H | 32 | 1.0014 | 1.0137 | 1.05 | pass | digest of complete results; timed reads row counts | 63,835.2 | 63,740.6 | -94.5 | 68,956/114,662 | 68,866/112,288 | n/a | n/a
br_pt_ea | B:P | 32 | 0.9927 | 1.0005 | 1.05 | pass | digest of complete results; timed reads row counts | 63,846.8 | 63,659.4 | -187.4 | 69,262/114,332 | 68,882/111,406 | n/a | n/a
br_scan | B:H | 32 | 1.0042 | 1.0097 | 1.05 | pass | digest of complete results; timed reads row counts | 10,749,238.4 | 10,762,806.3 | +13,567.9 | 10,662,459/11,854,526 | 10,669,662/11,352,730 | n/a | n/a
br_scan | B:P | 32 | 1.0012 | 1.0052 | 1.05 | pass | digest of complete results; timed reads row counts | 10,726,368.8 | 10,743,174.4 | +16,805.6 | 10,657,691/11,367,877 | 10,632,954/11,823,376 | n/a | n/a

### cold

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
cold_point (pages resident after eviction, per observation: median 1.1, worst process 147.8, of 2628) | B:H | 48 | 0.9897 | 1.0265 | 1.05 | bound ≤ 1.05; category outstanding: residency ≠ 0 | digest of a warm run; cold reads row counts only | 321,312.2 | 320,678.8 | -633.4 | 317,705/364,696 | 317,348/358,365 | 300.0→300.0 | 4,970→4,952
cold_point (pages resident after eviction, per observation: median 1.1, worst process 16.4, of 2628) | B:P | 48 | 1.0085 | 1.0466 | 1.05 | bound ≤ 1.05; category outstanding: residency ≠ 0 | digest of a warm run; cold reads row counts only | 320,954.1 | 321,832.8 | +878.7 | 318,841/359,682 | 319,636/359,988 | 300.0→300.0 | 4,988→4,940
cold_scan (pages resident after eviction, per observation: median 7.9, worst process 8.1, of 2628) | B:H | 48 | 1.0076 | 1.0148 | 1.05 | bound ≤ 1.05; category outstanding: residency ≠ 0 | digest of a warm run; cold reads row counts only | 12,984,385.4 | 13,087,053.6 | +102,668.1 | 12,909,324/14,579,688 | 12,917,524/14,619,511 | 112496.1→112496.1 | 6,426→6,416
cold_scan (pages resident after eviction, per observation: median 7.9, worst process 8.1, of 2628) | B:P | 48 | 0.9874 | 0.9951 | 1.05 | bound ≤ 1.05; category outstanding: residency ≠ 0 | digest of a warm run; cold reads row counts only | 12,992,931.9 | 12,829,694.2 | -163,237.7 | 12,865,788/14,909,494 | 12,829,400/14,265,158 | 112496.1→112496.1 | 6,424→6,398

### concurrency

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
cc_1 | B:H | 48 | 0.9959 | 1.0028 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 17,456.2 | 17,416.7 | -39.5 | 19,714/28,076 | 19,576/27,936 | 211.3→211.3 | 7,040→7,040
cc_1 | B:P | 48 | 1.0047 | 1.0106 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 17,414.2 | 17,535.8 | +121.6 | 19,670/28,040 | 19,846/28,064 | 211.3→211.3 | 7,010→6,974
cc_44 | B:H | 48 | 1.0342 | 1.0465 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 7,683.4 | 7,984.4 | +300.9 | 347,765/669,378 | 359,740/682,662 | 210.6→210.6 | 19,068→19,296
cc_44 | B:P | 48 | 0.9941 | 1.0068 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 7,741.9 | 7,641.2 | -100.7 | 347,871/664,409 | 342,718/673,365 | 210.6→210.6 | 19,262→19,248
cc_8 | B:H | 48 | 1.0097 | 1.0210 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 10,960.9 | 11,067.0 | +106.1 | 93,986/147,632 | 93,946/148,802 | 210.7→210.7 | 11,844→11,784
cc_8 | B:P | 48 | 0.9988 | 1.0104 | 1.05 | bound ≤ 1.05; not fully accepted: results not digested | row counts only | 10,925.5 | 10,899.3 | -26.2 | 95,163/145,100 | 92,550/149,636 | 210.7→210.7 | 11,850→11,894

### secondary

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
con_checkpoint | B:H | 48 | 0.9941 | 1.0052 | 1.05 | pass | none (no result) | 631,270.1 | 620,908.6 | -10,361.5 | 613,404/964,533 | 609,314/960,965 | 1672.7→1672.6 | 66,180→66,244
con_checkpoint | B:P | 48 | 1.0015 | 1.0133 | 1.05 | pass | none (no result) | 629,057.7 | 629,841.2 | +783.6 | 611,758/961,767 | 612,601/963,720 | 1672.6→1673.8 | 66,820→60,566
con_direct | B:H | 48 | 1.0056 | 1.0183 | 1.05 | pass | none (no result) | 97.8 | 97.1 | -0.7 | 96/103 | 94/103 | 1.0→1.0 | 66,936→67,134
con_direct | B:P | 48 | 1.1663 | 1.1744 | 1.05 | FAIL | none (no result) | 97.5 | 114.0 | +16.5 | 96/103 | 114/118 | 1.0→2.0 | 67,118→61,480
con_large_tx | B:H | 48 | 0.9963 | 1.0015 | 1.05 | pass | digest of complete results; timed reads row counts | 20,870.3 | 20,845.7 | -24.7 | 20,592/25,496 | 20,560/25,821 | 284.0→284.0 | 66,180→66,244
con_large_tx | B:P | 48 | 1.0095 | 1.0161 | 1.05 | pass | digest of complete results; timed reads row counts | 20,813.3 | 21,034.2 | +220.8 | 20,542/25,888 | 20,800/25,734 | 284.0→285.0 | 66,820→60,566
con_open | B:H | 48 | 0.9978 | 1.0015 | 1.05 | pass | none (no result) | 26,038.7 | 25,903.0 | -135.7 | 25,666/30,732 | 25,498/31,048 | 64.0→64.0 | 14,830→14,428
con_open | B:P | 48 | 1.0035 | 1.0075 | 1.05 | pass | none (no result) | 25,781.2 | 25,915.0 | +133.8 | 25,446/31,372 | 25,584/30,579 | 64.0→65.0 | 14,794→14,172
cw_long | B:H | 48 | 0.9960 | 0.9994 | 1.05 | pass | digest of complete results; timed reads row counts | 11,879,403.2 | 11,824,931.1 | -54,472.1 | 11,804,306/13,217,290 | 11,792,790/13,284,318 | 112171.0→112171.0 | 14,830→14,428
cw_long | B:P | 48 | 0.9945 | 0.9987 | 1.05 | pass | digest of complete results; timed reads row counts | 11,875,157.4 | 11,838,938.2 | -36,219.3 | 11,810,476/13,175,814 | 11,776,852/12,960,986 | 112171.0→112171.0 | 14,794→14,172
fl_paged | B:H | 48 | 0.9880 | 0.9909 | 1.05 | pass | digest of complete results; timed reads row counts | 22,703,569.7 | 22,469,659.6 | -233,910.1 | 22,579,422/24,009,678 | 22,370,454/23,888,775 | 483666.0→483666.0 | 14,830→14,428
fl_paged | B:P | 48 | 0.9841 | 0.9885 | 1.05 | pass | digest of complete results; timed reads row counts | 22,716,849.8 | 22,357,293.1 | -359,556.8 | 22,584,410/24,400,652 | 22,207,456/23,874,188 | 483666.0→483666.0 | 14,794→14,172
nc_point | B:H | 48 | 1.0026 | 1.0074 | 1.05 | pass | digest of complete results; timed reads row counts | 24,790.5 | 24,826.9 | +36.4 | 28,153/36,584 | 28,326/37,166 | 212.0→212.0 | 14,830→14,428
nc_point | B:P | 48 | 1.0081 | 1.0137 | 1.05 | pass | digest of complete results; timed reads row counts | 24,789.8 | 25,037.5 | +247.7 | 28,166/37,189 | 28,408/37,156 | 212.0→212.0 | 14,794→14,172
nc_scan | B:H | 48 | 0.9951 | 0.9990 | 1.05 | pass | digest of complete results; timed reads row counts | 12,689,756.4 | 12,672,949.9 | -16,806.4 | 12,623,336/14,076,314 | 12,598,568/13,885,363 | 112701.0→112701.0 | 14,830→14,428
nc_scan | B:P | 48 | 0.9952 | 0.9989 | 1.05 | pass | digest of complete results; timed reads row counts | 12,713,403.9 | 12,654,989.2 | -58,414.7 | 12,618,898/14,029,561 | 12,581,154/14,024,658 | 112701.0→112701.0 | 14,794→14,172
sc_dense_uuid | B:H | 48 | 0.9958 | 1.0003 | 1.05 | pass | digest of complete results; timed reads row counts | 11,487,511.9 | 11,450,242.1 | -37,269.8 | 11,426,982/12,923,934 | 11,396,480/12,913,895 | 112016.0→112016.0 | 14,830→14,428
sc_dense_uuid | B:P | 48 | 0.9999 | 1.0039 | 1.05 | pass | digest of complete results; timed reads row counts | 11,486,807.8 | 11,494,298.9 | +7,491.0 | 11,419,483/13,024,490 | 11,434,132/12,806,693 | 112016.0→112016.0 | 14,794→14,172
sc_long_history | B:H | 48 | 0.9951 | 0.9996 | 1.05 | pass | digest of complete results; timed reads row counts | 249,671.3 | 248,602.0 | -1,069.4 | 247,394/271,320 | 246,849/267,436 | 3260.0→3260.0 | 14,830→14,428
sc_long_history | B:P | 48 | 1.0025 | 1.0077 | 1.05 | pass | digest of complete results; timed reads row counts | 249,798.2 | 251,703.5 | +1,905.3 | 247,920/273,116 | 249,092/271,534 | 3260.0→3260.0 | 14,794→14,172
sc_short | B:H | 48 | 0.9953 | 0.9997 | 1.05 | pass | digest of complete results; timed reads row counts | 11,986,978.4 | 11,944,553.8 | -42,424.6 | 11,920,102/13,434,857 | 11,890,838/13,481,144 | 112021.0→112021.0 | 14,830→14,428
sc_short | B:P | 48 | 0.9961 | 0.9998 | 1.05 | pass | digest of complete results; timed reads row counts | 11,994,266.2 | 11,936,334.1 | -57,932.1 | 11,903,360/13,278,074 | 11,879,789/13,281,634 | 112021.0→112021.0 | 14,794→14,172
sc_sparse_uuid | B:H | 48 | 0.9921 | 0.9957 | 1.05 | pass | digest of complete results; timed reads row counts | 260,970.0 | 258,895.0 | -2,075.0 | 259,164/282,146 | 257,662/275,966 | 5024.0→5024.0 | 14,830→14,428
sc_sparse_uuid | B:P | 48 | 0.9942 | 0.9983 | 1.05 | pass | digest of complete results; timed reads row counts | 261,261.2 | 260,373.8 | -887.5 | 258,949/284,821 | 258,043/281,408 | 5024.0→5024.0 | 14,794→14,172
sh_as_of | B:H | 48 | 1.0046 | 1.0100 | 1.05 | pass | digest of complete results; timed reads row counts | 20,131.7 | 20,228.7 | +97.0 | 19,918/25,226 | 19,984/25,763 | 285.0→285.0 | 12,882→12,404
sh_as_of | B:P | 48 | 0.9982 | 1.0057 | 1.05 | pass | digest of complete results; timed reads row counts | 20,188.8 | 20,304.5 | +115.7 | 19,935/25,756 | 19,986/24,997 | 285.0→285.0 | 12,844→12,334
sh_as_of_ts | B:H | 48 | 0.9983 | 1.0025 | 1.05 | pass | digest of complete results; timed reads row counts | 775,707.3 | 772,527.1 | -3,180.2 | 769,604/816,682 | 769,312/822,024 | 11877.0→11877.0 | 12,882→12,404
sh_as_of_ts | B:P | 48 | 0.9973 | 1.0017 | 1.05 | pass | digest of complete results; timed reads row counts | 775,711.1 | 775,346.1 | -365.1 | 772,649/817,278 | 772,284/820,586 | 11877.0→11877.0 | 12,844→12,334
sh_as_of_ts_scan | B:H | 48 | 0.9988 | 1.0029 | 1.05 | pass | digest of complete results; timed reads row counts | 7,626,128.0 | 7,628,113.3 | +1,985.3 | 7,580,550/7,901,030 | 7,588,714/7,811,623 | 76294.0→76294.0 | 12,882→12,404
sh_as_of_ts_scan | B:P | 48 | 0.9984 | 1.0019 | 1.05 | pass | digest of complete results; timed reads row counts | 7,634,308.5 | 7,626,841.5 | -7,467.0 | 7,585,552/7,920,116 | 7,588,468/8,025,112 | 76294.0→76294.0 | 12,844→12,334
sh_attribute_scan | B:H | 48 | 1.0022 | 1.0062 | 1.05 | pass | digest of complete results; timed reads row counts | 10,672,537.6 | 10,690,609.3 | +18,071.7 | 10,612,747/11,566,579 | 10,634,511/11,865,178 | 112021.0→112021.0 | 6,684→6,664
sh_attribute_scan | B:P | 48 | 0.9979 | 1.0020 | 1.05 | pass | digest of complete results; timed reads row counts | 10,689,254.4 | 10,663,976.4 | -25,278.0 | 10,617,502/12,065,214 | 10,612,682/11,734,378 | 112021.0→112021.0 | 6,692→6,664
sh_attribute_value | B:H | 48 | 1.0192 | 1.0262 | 1.05 | pass | digest of complete results; timed reads row counts | 15,999.9 | 16,272.9 | +273.0 | 15,786/20,515 | 16,088/21,332 | 270.0→270.0 | 5,416→5,374
sh_attribute_value | B:P | 48 | 1.0048 | 1.0142 | 1.05 | pass | digest of complete results; timed reads row counts | 15,998.3 | 16,180.9 | +182.6 | 15,778/21,012 | 15,985/20,572 | 270.0→270.0 | 5,410→5,374
sh_count | B:H | 48 | 0.9968 | 1.0026 | 1.05 | pass | digest of complete results; timed reads row counts | 41,068,304.3 | 41,006,882.3 | -61,422.0 | 40,912,169/42,866,036 | 40,831,575/42,457,291 | 477489.7→477489.7 | 12,882→12,404
sh_count | B:P | 48 | 1.0005 | 1.0040 | 1.05 | pass | digest of complete results; timed reads row counts | 41,186,280.1 | 41,172,853.2 | -13,426.9 | 41,017,124/42,818,934 | 40,999,824/43,001,730 | 477489.7→477489.7 | 12,844→12,334
sh_count_grouped | B:H | 48 | 0.9911 | 0.9961 | 1.05 | pass | digest of complete results; timed reads row counts | 6,041,161.9 | 5,989,567.4 | -51,594.5 | 5,996,008/6,417,956 | 5,952,320/6,326,556 | 75953.0→75953.0 | 12,882→12,404
sh_count_grouped | B:P | 48 | 0.9914 | 0.9953 | 1.05 | pass | digest of complete results; timed reads row counts | 6,045,797.0 | 5,998,476.1 | -47,321.0 | 6,011,242/6,455,302 | 5,965,906/6,467,170 | 75953.0→75953.0 | 12,844→12,334
sh_hot_as_of | B:H | 48 | 0.9989 | 1.0032 | 1.05 | pass | digest of complete results; timed reads row counts | 240,524.5 | 241,330.8 | +806.3 | 238,767/263,498 | 239,112/261,413 | 2778.0→2778.0 | 14,830→14,428
sh_hot_as_of | B:P | 48 | 1.0052 | 1.0110 | 1.05 | pass | digest of complete results; timed reads row counts | 241,393.8 | 243,603.2 | +2,209.4 | 239,578/262,098 | 241,117/265,086 | 2778.0→2778.0 | 14,794→14,172
sh_hot_current | B:H | 48 | 0.9982 | 1.0031 | 1.05 | pass | digest of complete results; timed reads row counts | 249,974.2 | 250,581.0 | +606.8 | 247,659/274,712 | 248,358/272,500 | 3267.0→3267.0 | 14,830→14,428
sh_hot_current | B:P | 48 | 1.0040 | 1.0097 | 1.05 | pass | digest of complete results; timed reads row counts | 250,834.3 | 252,495.2 | +1,660.9 | 248,819/271,672 | 250,240/272,260 | 3267.0→3267.0 | 14,794→14,172
sh_join_2 | B:H | 48 | 1.0125 | 1.0177 | 1.05 | pass | digest of complete results; timed reads row counts | 36,335.0 | 36,839.2 | +504.2 | 35,866/42,397 | 36,452/43,068 | 534.0→534.0 | 6,688→6,672
sh_join_2 | B:P | 48 | 0.9984 | 1.0056 | 1.05 | pass | digest of complete results; timed reads row counts | 36,607.4 | 36,649.6 | +42.1 | 36,062/42,815 | 36,083/42,361 | 534.0→534.0 | 6,702→6,668
sh_join_3_scan | B:H | 48 | 0.9925 | 0.9957 | 1.05 | pass | digest of complete results; timed reads row counts | 43,328,360.1 | 42,914,958.2 | -413,401.9 | 43,182,828/44,561,955 | 42,742,794/44,375,078 | 628334.0→628334.0 | 12,882→12,404
sh_join_3_scan | B:P | 48 | 0.9953 | 1.0000 | 1.05 | pass | digest of complete results; timed reads row counts | 43,266,295.1 | 43,176,694.6 | -89,600.5 | 43,180,432/44,721,884 | 42,940,630/44,870,019 | 628334.0→628334.0 | 12,844→12,334
sh_not_join_hash | B:H | 48 | 0.9994 | 1.0043 | 1.05 | pass | digest of complete results; timed reads row counts | 5,802,646.2 | 5,792,829.4 | -9,816.8 | 5,778,831/6,157,891 | 5,774,830/6,171,070 | 69282.8→69282.8 | 12,882→12,404
sh_not_join_hash | B:P | 48 | 0.9958 | 0.9994 | 1.05 | pass | digest of complete results; timed reads row counts | 5,802,256.5 | 5,779,014.3 | -23,242.2 | 5,771,006/6,184,750 | 5,753,748/6,181,490 | 69282.8→69282.8 | 12,844→12,334
sh_not_probe | B:H | 48 | 0.9990 | 1.0035 | 1.05 | pass | digest of complete results; timed reads row counts | 2,627,998.7 | 2,626,831.5 | -1,167.2 | 2,566,280/2,804,514 | 2,568,068/2,801,176 | 33879.6→33879.6 | 12,882→12,404
sh_not_probe | B:P | 48 | 0.9974 | 1.0011 | 1.05 | pass | digest of complete results; timed reads row counts | 2,627,805.9 | 2,623,650.1 | -4,155.8 | 2,565,158/2,809,394 | 2,561,114/2,786,158 | 33879.6→33879.6 | 12,844→12,334
sh_or_2 | B:H | 48 | 1.0001 | 1.0043 | 1.05 | pass | digest of complete results; timed reads row counts | 9,449,519.9 | 9,448,738.3 | -781.6 | 9,391,438/9,927,541 | 9,389,164/9,776,644 | 104895.7→104895.7 | 12,882→12,404
sh_or_2 | B:P | 48 | 0.9958 | 0.9998 | 1.05 | pass | digest of complete results; timed reads row counts | 9,416,899.1 | 9,388,147.4 | -28,751.8 | 9,381,848/9,748,922 | 9,355,176/9,705,876 | 104895.7→104895.7 | 12,844→12,334
sh_point_entity | B:H | 48 | 1.0131 | 1.0207 | 1.05 | pass | digest of complete results; timed reads row counts | 15,917.7 | 16,144.0 | +226.3 | 15,748/20,570 | 15,950/21,064 | 260.0→260.0 | 5,464→5,436
sh_point_entity | B:P | 48 | 0.9959 | 1.0048 | 1.05 | pass | digest of complete results; timed reads row counts | 15,971.6 | 15,951.2 | -20.4 | 15,760/20,903 | 15,786/20,278 | 260.0→260.0 | 5,464→5,438
sh_recursive_ancestors | B:H | 48 | 0.9981 | 1.0027 | 1.05 | pass | digest of complete results; timed reads row counts | 1,209,488.6 | 1,207,755.3 | -1,733.3 | 1,201,178/1,273,990 | 1,199,923/1,267,516 | 17734.0→17734.0 | 12,882→12,404
sh_recursive_ancestors | B:P | 48 | 0.9974 | 1.0014 | 1.05 | pass | digest of complete results; timed reads row counts | 1,211,471.6 | 1,205,869.9 | -5,601.7 | 1,203,254/1,274,176 | 1,200,294/1,274,808 | 17734.0→17734.0 | 12,844→12,334
sh_window | B:H | 48 | 0.9950 | 0.9996 | 1.05 | pass | digest of complete results; timed reads row counts | 36,824,513.0 | 36,766,100.4 | -58,412.7 | 36,725,990/38,143,838 | 36,557,236/37,981,422 | 561647.0→561647.0 | 14,830→14,428
sh_window | B:P | 48 | 0.9928 | 0.9969 | 1.05 | pass | digest of complete results; timed reads row counts | 36,881,034.8 | 36,745,260.9 | -135,773.9 | 36,750,472/38,616,509 | 36,543,122/38,075,480 | 561647.0→561647.0 | 14,794→14,172

### wasi

Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C
---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:
wasi_committed_con_direct | B:H | 32 | 0.9862 | 1.0007 | 1.05 | pass | none (no result) | 192.4 | 188.1 | -4.3 | 191/211 | 188/204 | 1.0→1.0 | n/a
wasi_committed_con_direct | B:P | 32 | 1.1185 | 1.1301 | 1.05 | FAIL | none (no result) | 193.1 | 215.3 | +22.2 | 192/212 | 212/235 | 1.0→2.0 | n/a
wasi_committed_join_2 | B:H | 32 | 0.9966 | 1.0010 | 1.05 | pass | digest of complete results; timed reads row counts | 43,240.9 | 43,219.1 | -21.8 | 42,688/50,068 | 42,668/49,647 | 532.0→532.0 | n/a
wasi_committed_join_2 | B:P | 32 | 0.9936 | 1.0015 | 1.05 | pass | digest of complete results; timed reads row counts | 43,143.9 | 43,071.1 | -72.8 | 42,677/50,055 | 42,508/49,318 | 532.0→532.0 | n/a
wasi_committed_pt_av | B:H | 32 | 1.0027 | 1.0079 | 1.05 | pass | digest of complete results; timed reads row counts | 20,859.2 | 21,039.4 | +180.2 | 20,036/28,792 | 20,124/28,571 | 254.2→254.2 | n/a
wasi_committed_pt_av | B:P | 32 | 0.9844 | 0.9940 | 1.05 | pass | digest of complete results; timed reads row counts | 20,822.5 | 20,660.2 | -162.2 | 20,004/28,829 | 19,749/28,273 | 254.2→254.2 | n/a
wasi_committed_pt_ea | B:H | 32 | 1.0022 | 1.0090 | 1.05 | pass | digest of complete results; timed reads row counts | 17,791.7 | 17,778.2 | -13.5 | 18,062/29,234 | 18,154/28,956 | 209.0→209.0 | n/a
wasi_committed_pt_ea | B:P | 32 | 0.9956 | 1.0048 | 1.05 | pass | digest of complete results; timed reads row counts | 17,832.8 | 17,737.8 | -94.9 | 18,050/28,918 | 18,208/28,636 | 209.0→209.0 | n/a
wasi_committed_scan | B:H | 32 | 0.9938 | 0.9984 | 1.05 | pass | digest of complete results; timed reads row counts | 4,008,200.6 | 3,990,149.2 | -18,051.4 | 3,986,506/4,183,370 | 3,972,311/4,233,772 | 34030.0→34030.0 | n/a
wasi_committed_scan | B:P | 32 | 0.9931 | 0.9985 | 1.05 | pass | digest of complete results; timed reads row counts | 4,010,477.0 | 3,992,316.0 | -18,161.0 | 3,991,959/4,239,511 | 3,967,182/4,234,810 | 34030.0→34030.0 | n/a
wasi_pending_pt_av | B:H | 32 | 1.0000 | 1.0023 | 1.05 | pass | digest of complete results; timed reads row counts | 2,305,855.8 | 2,305,695.7 | -160.1 | 2,299,936/2,394,942 | 2,298,078/2,443,002 | 35103.0→35103.0 | n/a
wasi_pending_pt_av | B:P | 32 | 0.9964 | 1.0016 | 1.05 | pass | digest of complete results; timed reads row counts | 2,307,227.5 | 2,289,418.5 | -17,809.0 | 2,294,288/2,436,091 | 2,286,704/2,413,598 | 35103.0→35103.0 | n/a
wasi_pending_pt_ea | B:H | 32 | 0.9924 | 1.0014 | 1.05 | pass | digest of complete results; timed reads row counts | 8,873.2 | 8,743.0 | -130.2 | 9,607/12,022 | 9,467/11,854 | 77.2→77.2 | n/a
wasi_pending_pt_ea | B:P | 32 | 0.9897 | 1.0032 | 1.05 | pass | digest of complete results; timed reads row counts | 8,863.4 | 8,788.0 | -75.4 | 9,600/11,938 | 9,429/11,816 | 77.2→77.2 | n/a
wasi_pending_scan | B:H | 32 | 1.0052 | 1.0105 | 1.05 | pass | digest of complete results; timed reads row counts | 4,443,464.6 | 4,454,651.2 | +11,186.6 | 4,425,839/4,656,610 | 4,445,678/4,721,459 | 45243.0→45243.0 | n/a
wasi_pending_scan | B:P | 32 | 0.9920 | 0.9982 | 1.05 | pass | digest of complete results; timed reads row counts | 4,445,031.1 | 4,410,444.0 | -34,587.1 | 4,425,610/4,648,340 | 4,393,544/4,616,716 | 45243.0→45243.0 | n/a
