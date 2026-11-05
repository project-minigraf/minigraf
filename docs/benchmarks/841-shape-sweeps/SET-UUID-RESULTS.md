# Completed set and UUID results

Generated from the completed original public matrix. Latency is milliseconds:
median of the three session medians (minimum–maximum session median), with
five verified timed samples in each session. RSS is the maximum process
high-water mark across successful batches, in MiB. See the [report](README.md)
for timing, oracle, warm-up, and memory-accounting boundaries.

## Sets

77 of 81 cases completed all 15 timings: 1,155 verified timings. Four cases
were rejected during fixture construction; their 60 planned slots are deferred.
All cases use a 256-page storage cache. Projection and membership fixtures have
`2*N` facts; collection fixtures have `N*(K+1)` facts. Collection with `K=0`
has no member facts and returns one empty set per entity. Membership projects six
Boolean results for every entity. K is requested cardinality, and repeated
means that entities share the same set, not duplicate elements within a set.

| Entities | K | Distribution | Operation | Storage | Latency (ms) | Peak RSS (MiB) |
| ---: | ---: | --- | --- | --- | ---: | ---: |
| 10,000 | 0 | repeated | project | committed | 138.264 (133.499–139.436) | 5.6 |
| 10,000 | 0 | repeated | membership | committed | 144.059 (140.829–147.536) | 5.7 |
| 10,000 | 0 | repeated | collect | committed | 14.404 (14.362–14.745) | 5.1 |
| 10,000 | 0 | unique | project | committed | 137.516 (137.341–139.288) | 5.5 |
| 10,000 | 0 | unique | membership | committed | 142.220 (141.629–145.432) | 5.8 |
| 10,000 | 0 | unique | collect | committed | 14.596 (14.510–14.751) | 5.0 |
| 10,000 | 0 | skew | project | committed | 131.298 (129.299–134.571) | 5.5 |
| 10,000 | 0 | skew | membership | committed | 145.492 (141.297–145.530) | 5.8 |
| 10,000 | 0 | skew | collect | committed | 14.525 (14.438–14.951) | 5.0 |
| 10,000 | 1 | repeated | project | committed | 136.017 (134.849–136.839) | 5.9 |
| 10,000 | 1 | repeated | membership | committed | 143.867 (143.224–146.672) | 6.1 |
| 10,000 | 1 | unique | project | committed | 127.499 (125.513–127.694) | 5.9 |
| 10,000 | 1 | unique | membership | committed | 136.415 (135.601–137.829) | 6.1 |
| 10,000 | 1 | skew | project | committed | 131.665 (130.195–133.630) | 6.0 |
| 10,000 | 1 | skew | membership | committed | 147.555 (145.806–148.683) | 6.2 |
| 10,000 | 4 | repeated | project | committed | 108.742 (107.722–110.515) | 6.1 |
| 10,000 | 4 | repeated | membership | committed | 128.064 (124.398–136.219) | 6.4 |
| 10,000 | 4 | repeated | collect | committed | 178.151 (174.330–179.078) | 6.1 |
| 10,000 | 4 | unique | project | committed | 103.980 (100.511–105.661) | 6.2 |
| 10,000 | 4 | unique | membership | committed | 120.643 (118.891–120.998) | 6.2 |
| 10,000 | 4 | unique | collect | committed | 173.178 (171.544–177.085) | 6.1 |
| 10,000 | 4 | skew | project | committed | 107.488 (107.034–109.923) | 6.0 |
| 10,000 | 4 | skew | membership | committed | 124.026 (123.510–127.514) | 6.1 |
| 10,000 | 4 | skew | collect | committed | 176.751 (172.965–178.143) | 6.0 |
| 10,000 | 8 | repeated | project | committed | 96.211 (95.643–96.461) | 6.2 |
| 10,000 | 8 | repeated | membership | committed | 118.391 (118.008–118.935) | 6.4 |
| 10,000 | 8 | unique | project | committed | 86.620 (85.996–88.285) | 6.2 |
| 10,000 | 8 | unique | membership | committed | 106.629 (106.335–107.415) | 6.4 |
| 10,000 | 8 | skew | project | committed | 96.722 (93.954–98.218) | 6.2 |
| 10,000 | 8 | skew | membership | committed | 118.083 (114.759–125.086) | 6.3 |
| 10,000 | 16 | repeated | project | committed | 93.807 (92.722–94.117) | 7.8 |
| 10,000 | 16 | repeated | membership | committed | 122.599 (122.248–125.068) | 7.3 |
| 10,000 | 16 | unique | project | committed | 92.637 (90.943–94.158) | 7.8 |
| 10,000 | 16 | unique | membership | committed | 120.817 (118.468–124.203) | 7.3 |
| 10,000 | 16 | skew | project | committed | 93.723 (92.923–96.848) | 7.8 |
| 10,000 | 16 | skew | membership | committed | 123.723 (123.002–123.871) | 7.1 |
| 10,000 | 32 | repeated | project | committed | 123.570 (121.528–126.686) | 9.1 |
| 10,000 | 32 | repeated | membership | committed | 167.536 (164.690–168.617) | 7.9 |
| 10,000 | 32 | repeated | collect | committed | 306.426 (305.734–309.770) | 9.2 |
| 10,000 | 32 | unique | project | committed | 127.898 (127.327–128.380) | 9.2 |
| 10,000 | 32 | unique | membership | committed | 170.732 (168.933–175.692) | 7.9 |
| 10,000 | 32 | unique | collect | committed | 307.979 (303.850–310.459) | 9.1 |
| 10,000 | 32 | skew | project | committed | 125.233 (122.016–127.279) | 9.1 |
| 10,000 | 32 | skew | membership | committed | 167.131 (166.007–168.641) | 8.1 |
| 10,000 | 32 | skew | collect | committed | 317.371 (308.252–322.381) | 9.3 |
| 10,000 | 64 | repeated | project | committed | 204.056 (197.940–212.760) | 13.1 |
| 10,000 | 64 | repeated | membership | committed | 276.510 (273.912–280.788) | 9.8 |
| 10,000 | 64 | unique | project | committed | 203.706 (201.492–209.489) | 13.1 |
| 10,000 | 64 | unique | membership | committed | 284.011 (280.284–284.205) | 9.9 |
| 10,000 | 64 | skew | project | committed | 202.323 (198.981–202.359) | 13.4 |
| 10,000 | 64 | skew | membership | committed | 281.956 (280.818–282.085) | 9.8 |
| 10,000 | 256 | repeated | project | committed | 778.232 (766.247–836.257) | 38.0 |
| 10,000 | 256 | repeated | membership | committed | 1,112.561 (1,109.732–1,117.153) | 22.4 |
| 10,000 | 256 | repeated | collect | committed | 1,412.728 (1,396.381–1,415.670) | 38.0 |
| 10,000 | 256 | unique | project | committed | 813.740 (812.503–818.063) | 38.0 |
| 10,000 | 256 | unique | membership | committed | 1,136.173 (1,112.866–1,198.736) | 22.5 |
| 10,000 | 256 | unique | collect | committed | 1,428.384 (1,403.639–1,447.432) | 38.1 |
| 10,000 | 256 | skew | project | committed | 788.500 (782.154–831.639) | 38.0 |
| 10,000 | 256 | skew | membership | committed | 1,124.625 (1,099.271–1,161.619) | 22.5 |
| 10,000 | 256 | skew | collect | committed | 1,511.490 (1,411.531–1,511.927) | 38.1 |
| 10,000 | 1,024 | repeated | project | committed | 3,767.407 (3,561.869–3,775.988) | 138.9 |
| 10,000 | 1,024 | repeated | membership | committed | 5,401.476 (5,379.960–5,408.994) | 72.6 |
| 10,000 | 1,024 | unique | project | committed | setup rejected | — |
| 10,000 | 1,024 | unique | membership | committed | setup rejected | — |
| 10,000 | 1,024 | skew | project | committed | setup rejected | — |
| 10,000 | 1,024 | skew | membership | committed | setup rejected | — |
| 10,000 | 32 | repeated | membership | pending | 98.434 (98.357–99.426) | 76.7 |
| 10,000 | 32 | unique | membership | pending | 96.648 (96.412–98.261) | 76.8 |
| 10,000 | 32 | skew | membership | pending | 98.916 (96.022–101.488) | 76.8 |
| 100,000 | 4 | repeated | project | committed | 1,121.301 (1,100.416–1,162.173) | 6.2 |
| 100,000 | 4 | repeated | membership | committed | 1,258.368 (1,258.216–1,313.430) | 6.5 |
| 100,000 | 4 | unique | project | committed | 977.130 (967.745–990.313) | 6.2 |
| 100,000 | 4 | unique | membership | committed | 1,122.035 (1,120.025–1,124.059) | 6.4 |
| 100,000 | 32 | repeated | project | committed | 1,246.687 (1,244.758–1,266.306) | 9.3 |
| 100,000 | 32 | repeated | membership | committed | 1,690.267 (1,689.007–1,776.505) | 8.0 |
| 100,000 | 32 | unique | project | committed | 1,307.158 (1,289.723–1,309.586) | 9.5 |
| 100,000 | 32 | unique | membership | committed | 1,723.360 (1,704.494–1,752.293) | 8.0 |
| 100,000 | 256 | repeated | project | committed | 7,889.644 (7,887.564–7,903.835) | 38.3 |
| 100,000 | 256 | repeated | membership | committed | 11,094.837 (11,017.344–11,112.537) | 22.5 |
| 100,000 | 256 | unique | project | committed | 8,403.226 (8,380.113–8,409.765) | 38.0 |
| 100,000 | 256 | unique | membership | committed | 11,350.154 (11,306.632–11,382.993) | 22.5 |

## UUID projection

All 36 cases completed all 15 timings: 540 verified timings. All fixtures are
committed and have 18 input facts per output row. Cache is the configured
storage page-cache capacity, not the UUID translation-cache capacity. `refs`
projects each referenced UUID twice; `project` also projects the source entity. K identifies
the repeated-reference working-set size; it is not used by other distributions.

| Output rows | Distribution | K | Operation | Cache pages | Latency (ms) | Peak RSS (MiB) |
| ---: | --- | ---: | --- | ---: | ---: | ---: |
| 10,000 | dense | — | refs | 256 | 140.277 (138.622–141.248) | 5.9 |
| 10,000 | descending | — | refs | 256 | 142.381 (140.625–143.947) | 5.9 |
| 10,000 | sparse | — | refs | 256 | 146.867 (144.045–147.846) | 5.9 |
| 10,000 | permuted | — | refs | 256 | 143.816 (143.175–145.505) | 5.9 |
| 10,000 | hot | — | refs | 256 | 142.979 (142.822–144.709) | 5.8 |
| 10,000 | repeat | 1 | refs | 256 | 136.396 (135.206–139.838) | 5.5 |
| 10,000 | repeat | 16 | refs | 256 | 141.056 (139.397–153.842) | 5.6 |
| 10,000 | repeat | 64 | refs | 256 | 140.761 (138.380–142.046) | 5.6 |
| 10,000 | repeat | 1024 | refs | 256 | 140.076 (137.505–140.184) | 5.6 |
| 10,000 | dense | — | refs | 0 | 232.492 (231.790–233.308) | 4.7 |
| 10,000 | sparse | — | refs | 0 | 272.708 (270.754–276.131) | 4.7 |
| 10,000 | repeat | 1024 | refs | 0 | 233.725 (233.398–235.855) | 4.7 |
| 10,000 | dense | — | refs | 4,096 | 141.338 (139.948–144.926) | 5.9 |
| 10,000 | sparse | — | refs | 4,096 | 143.303 (140.584–143.330) | 9.9 |
| 10,000 | repeat | 1024 | refs | 4,096 | 137.833 (137.032–141.882) | 5.6 |
| 10,000 | dense | — | project | 256 | 140.937 (140.083–143.604) | 5.9 |
| 10,000 | sparse | — | project | 256 | 147.126 (144.131–148.014) | 5.8 |
| 10,000 | repeat | 16 | project | 256 | 143.537 (142.863–145.095) | 5.5 |
| 100,000 | dense | — | refs | 256 | 1,319.750 (1,274.001–1,328.459) | 5.9 |
| 100,000 | descending | — | refs | 256 | 1,327.198 (1,285.076–1,328.104) | 6.0 |
| 100,000 | sparse | — | refs | 256 | 1,403.875 (1,370.577–1,488.121) | 5.9 |
| 100,000 | permuted | — | refs | 256 | 1,423.934 (1,375.051–1,459.436) | 5.8 |
| 100,000 | hot | — | refs | 256 | 1,354.647 (1,314.559–1,377.837) | 5.8 |
| 100,000 | repeat | 1 | refs | 256 | 1,314.792 (1,295.166–1,323.254) | 5.9 |
| 100,000 | repeat | 16 | refs | 256 | 1,282.495 (1,276.887–1,308.526) | 5.9 |
| 100,000 | repeat | 64 | refs | 256 | 1,322.175 (1,304.588–1,327.239) | 5.9 |
| 100,000 | repeat | 1024 | refs | 256 | 1,348.075 (1,293.944–1,348.797) | 5.9 |
| 100,000 | dense | — | refs | 0 | 2,233.528 (2,217.201–2,235.676) | 4.8 |
| 100,000 | sparse | — | refs | 0 | 2,691.401 (2,660.940–2,693.667) | 4.8 |
| 100,000 | repeat | 1024 | refs | 0 | 2,234.094 (2,231.775–2,262.906) | 4.8 |
| 100,000 | dense | — | refs | 4,096 | 1,271.313 (1,227.755–1,281.130) | 18.1 |
| 100,000 | sparse | — | refs | 4,096 | 1,319.552 (1,313.308–1,377.072) | 21.6 |
| 100,000 | repeat | 1024 | refs | 4,096 | 1,268.710 (1,262.105–1,279.253) | 14.9 |
| 100,000 | dense | — | project | 256 | 1,329.770 (1,302.874–1,343.883) | 5.9 |
| 100,000 | sparse | — | project | 256 | 1,397.245 (1,359.585–1,420.328) | 5.9 |
| 100,000 | repeat | 16 | project | 256 | 1,345.729 (1,307.399–1,355.332) | 5.9 |
