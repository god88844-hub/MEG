---
name: diagnosing-bugs
description: Reproduce first, minimise, hypothesise, instrument, fix, then lock in a regression test.
triggers: [diagnose the bug, why is this failing, reproduce the bug, root cause, intermittent failure, flaky test, minimis, minimise the repro]
required_tools: [run_command, read_file]
---

## Procedure Guidelines

Use this when something is broken and the cause is unknown. The order is the method: every
step produces the input the next step needs, and skipping one is how a fix lands for the
wrong reason and the bug returns.

### 1. Reproduce before anything else
Build the smallest command or script that shows the failure, and run it. No reading code, no
theories, no edits until the failure is on screen. A bug that cannot be reproduced on demand
cannot be confirmed fixed, and everything after this step depends on that confirmation.

### 2. Minimise the reproduction
Remove everything that is not needed to trigger it: extra files, extra flags, extra steps.
Stop when removing anything else makes the failure disappear. What remains is the boundary
of the bug, and it is usually far smaller than the system it was found in.

### 3. Hypothesise from the evidence, one at a time
State a single falsifiable cause - "X happens because Y" - where Y predicts something you
can check. Rank by what the evidence already rules out, not by what feels likely. Never
pursue two hypotheses at once: the second one is how a fix gets credited to the wrong cause.

### 4. Instrument, don't guess
Add the print, the assertion, or the log line that would prove or kill the hypothesis, then
run the repro. Read what it says. A hypothesis that survives instrumentation is a finding;
one that avoids it is a preference.

### 5. Fix at the cause, verify at the symptom
Change the smallest thing that removes the cause, then re-run the ORIGINAL reproduction -
not a simplified version of it. The original is the contract; a fix verified against anything
less is unverified.

### 6. Lock it in with a regression test
Write the test that fails without the fix and passes with it, and run the suite. A bug fixed
without a regression test is a bug scheduled to return, and the second occurrence will cost
more than the test.
