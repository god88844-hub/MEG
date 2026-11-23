---
name: compiler-debugging
description: Read a GCC or Clang build failure, fix the real error, and separate link faults from compile faults.
triggers: [compilation error, compile error, gcc, clang, undefined reference, linker error, ld returned, "build failed", "does not compile", -Werror, segment, fault]
required_tools: [run_command, read_file]
---

## Procedure Guidelines

A build failure is a MESSAGE, and the first job is to read it rather than to start editing.
Nearly all wasted compilation time is spent fixing the last error in the output when the
compiler has already said, on the first line, what is wrong.

### 1. Capture the output instead of scrolling it

Long compiler output gets truncated, and the actual diagnostic is usually near the TOP - the
errors cascade and the first is the cause, the rest are consequences. When output is truncated
to a slice, the cause is what got cut.

```
gcc /app/decomp.c -o /app/decomp > /tmp/build.log 2>&1; tail -40 /tmp/build.log
```

Reading a saved log beats re-running the build, and it is the difference between seeing the
first error and seeing the fiftieth.

### 2. Read the diagnostic properly

GCC and Clang share a shape:

```
/app/decomp.c:42:7: error: 'foo' undeclared here (not in a function)
   42 |   foo();
      |   ^~~
```

- `error:` stops the build. `warning:` does not, unless `-Werror`.
- `file:line:column` is where to look. Column matters for a missing semicolon.
- The quoted token is the symbol. The `^~~~` underline points at what it expects instead.
- `note:` lines following an error are usually the fix. Read them.

### 3. Fix errors in order, and rebuild once

The compiler stops at a cascade point and everything after it may be noise. Fix the FIRST
error, rebuild, and re-read: a later error frequently disappears because the first one was the
cause.

### 4. Separate compile faults from link faults

They have different shapes and different fixes, and conflating them wastes the most time.

**Compile fault** - the compiler never produced an object file. Named in the message, located by
line and column, and fixed by editing source.

**Link fault** - every file compiled; the objects could not be combined. Reported by `ld` or
`lld`, and there is no line number because no single line is wrong.

```
undefined reference to `foo'
collect2: error: ld returned 1 exit status
```

In order of likelihood:

1. The function is defined in a file that is not on the command line - the usual cause.
2. The declaration and the definition disagree, or the definition is missing entirely.
3. The name is `static` in one file and used from another. That is always a link error and
   never a compile error, which is what makes it confusing.
4. C versus C++ name mangling.

A `main` that is not found by the LINKER means there is no `main` at all, or it is not in a
compiled file - the compiler is quite happy about a library with no entry point.

### 5. Check the flags only after the source is right

A missing `-lm`, `-lpthread` or `-I` produces a link error that no amount of source editing
fixes, and a spurious one that looks like a typo in the code. When the error is a link error
and the symbol is a libc or POSIX function, the flag is the first thing to check.

### 6. Prove it with the binary

```
ls -l /app/decomp && /app/decomp < args
```

A build that reports success and produces no binary has not succeeded. Run it.
