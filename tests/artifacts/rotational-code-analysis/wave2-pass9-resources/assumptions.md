# Assumptions — Wave 2 Pass 9

1. Freeze identity retained: wave2 pass1 `62ee4b4595ad2433bd16b0ac14747dada612b4d6`; HEAD may advance with prior harden commits; dirty tree expected (beads / Pi leftover / books).
2. Harden authorize still covers product fixes on PR #27, but this pass only ships a product edit on a **new** high/critical dual-evidence **correctness-under-load** bug with a small fix.
3. Availability / DoS / cost residuals without wrong-answer dual evidence are named GAP or CONSISTENT-with-bound -- not auto-fixed.
4. Zerostack unavailable (fszero-codemode); no live fleet load test -- evidence is source + prior pins + ureq defaults.
5. Pass9 time-concurrency note that embed uses "timeout via client defaults" is **re-audited** and found incorrect for ureq 2.12 (read/overall timeout None).
