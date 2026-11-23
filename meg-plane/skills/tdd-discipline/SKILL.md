---
name: tdd-discipline
description: Write the failing test first, then the smallest change that passes it, then refactor.
triggers: [test driven, tdd, write the test first, red green refactor, failing test, add a test for, regression test]
required_tools: [run_command, read_file]
---

## Procedure Guidelines

Use this when implementing behaviour, fixing a bug, or extending a module: the test comes
first and the code follows it. A test written after the code proves the code runs; a test
written first proves the behaviour is wanted.

### 1. Agree on the seam before writing anything
Name the function, the file, and the exact input/output pair the test will assert. A test
against a seam that does not exist yet is a plan, not a test. If the seam is unclear, read
the callers first - the call sites are the specification the code already obeys.

### 2. Red first, and watch it fail for the right reason
Write the smallest test that fails, run it, and read the failure. A test that fails at import
or collection proves nothing about the behaviour; it must fail on the ASSERTION. If the
runner reports anything else, fix the test before touching the code.

### 3. Green with the smallest change that passes
Implement the minimum that turns the failure green. No refactoring, no adjacent cleanup, no
"while I am here". A green suite after a large change proves the change did not break what
the tests cover; a green suite after a small change proves the behaviour works.

### 4. Refactor only while green, then re-run
Only now is cleanup allowed, and only in steps small enough that each one keeps the suite
green. If a refactor turns the suite red, revert that step rather than fixing forward: a
refactor that needs its own debugging was not a refactor.

### 5. No tautological tests
A test that mirrors the implementation line-for-line passes if and only if the code is
unchanged; it cannot fail for a reason that matters. Assert on OBSERVABLE behaviour - return
values, files written, exit codes - never on the internal steps the code takes to get there.
