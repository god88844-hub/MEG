---
name: pack-systems
scope: specific
description: OS/Kernel QEMU boot, GCC differential compiler oracle, ChartEvo SELT near-miss/midpoint guard, read-only AppSec, Web3 foundry/slither, Qiskit quantum & ROS2/URDF robotics (Domains 21-29).
supports:
  - os-kernel
  - compilers
  - embedded
  - robotics
  - quantum
  - web3
  - devops-sre
  - appsec
  - data-ml
differs:
  - pack-appdev
excludes:
  - ui-ux
---
# Pack 5: Systems, Compilers, Embedded, Robotics, Quantum, Web3, SRE, AppSec & Data/AI-ML

1. Verify OS/kernel builds via headless QEMU boot sentinel markers and compilers via differential GCC/Clang output equivalence.
2. Enforce ChartEvo SELT rules on numeric/chart pipelines: flag 3%-5% near-miss readings and ban midpoint guessing on uncertain intervals.
3. Run AppSec audits strictly read-only; run `forge test` + `slither` for Web3 contracts; validate Qiskit circuits with competing-algorithm comparison; validate ROS2/URDF joint limits and inertia.