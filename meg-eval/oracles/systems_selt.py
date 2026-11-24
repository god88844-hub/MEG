"""P6-06 Pack 5 Oracle: QEMU boot, GCC differential compiler oracle, ChartEvo SELT
(3%-5% near-miss audit + uncertain-interval midpoint ban), read-only AppSec,
Web3 foundry/slither, Qiskit competing-algorithm search & ROS2/URDF templates."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, List, Sequence, Tuple


# ---------------------------------------------------------------------------
# 1. QEMU Boot & GCC Differential Compiler Oracles
# ---------------------------------------------------------------------------

def build_qemu_boot_argv(kernel_elf: str, memory_mb: int = 128) -> List[str]:
    return [
        "qemu-system-x86_64",
        "-kernel",
        kernel_elf,
        "-m",
        str(memory_mb),
        "-nographic",
        "-no-reboot",
        "-serial",
        "stdio",
    ]


def verify_qemu_boot_log(serial_log: str, expected_sentinel: str = "KERNEL_BOOT_OK") -> Dict[str, object]:
    panicked = "kernel panic" in serial_log.lower() or "triple fault" in serial_log.lower()
    booted = expected_sentinel in serial_log and not panicked
    return {"booted": booted, "panicked": panicked}


def check_gcc_differential_oracle(
    reference_stdout: str,
    candidate_stdout: str,
    reference_exit: int,
    candidate_exit: int,
) -> Dict[str, object]:
    """Differential compiler oracle: candidate binary must match reference GCC exit code & stdout byte-for-byte."""
    match = (reference_exit == candidate_exit) and (reference_stdout == candidate_stdout)
    return {
        "equivalent": match,
        "exit_mismatch": reference_exit != candidate_exit,
        "stdout_mismatch": reference_stdout != candidate_stdout,
    }


# ---------------------------------------------------------------------------
# 2. ChartEvo SELT Compiled Local Toolchain (3%-5% Near-Miss + Midpoint Ban)
# ---------------------------------------------------------------------------

def audit_selt_numeric_reading(
    predicted: float,
    ground_truth: float,
    interval: Tuple[float, float] | None = None,
) -> Dict[str, object]:
    """
    ChartEvo SELT audit:
    - Flags 3%-5% relative error as a `near_miss` requiring tool-assisted re-measurement.
    - Bans returning the exact midpoint `(lo + hi) / 2` of an uncertain interval.
    """
    denom = max(abs(ground_truth), 1e-9)
    rel_err = abs(predicted - ground_truth) / denom
    near_miss = 0.03 <= rel_err <= 0.05

    midpoint_violation = False
    if interval is not None:
        lo, hi = interval
        mid = (lo + hi) / 2.0
        if hi > lo and abs(predicted - mid) <= 1e-6:
            midpoint_violation = True

    status = (
        "midpoint_banned"
        if midpoint_violation
        else "pass"
        if rel_err < 0.03
        else "near_miss_audit"
        if near_miss
        else "fail"
    )
    return {
        "status": status,
        "relative_error": rel_err,
        "near_miss": near_miss,
        "midpoint_violation": midpoint_violation,
        "ok": status == "pass",
    }


# ---------------------------------------------------------------------------
# 3. Read-Only AppSec, Web3 (Foundry/Slither), Qiskit & ROS2/URDF Templates
# ---------------------------------------------------------------------------

def verify_appsec_readonly_audit(tools_invoked: Sequence[str]) -> Dict[str, object]:
    mutating = [t for t in tools_invoked if t in {"write_file", "edit_file", "delete_file"}]
    return {"readonly_ok": len(mutating) == 0, "violations": mutating}


def build_web3_verification_pipeline(contract_dir: str) -> Dict[str, List[str]]:
    return {
        "foundry_test": ["forge", "test", "--root", contract_dir, "-vvv"],
        "slither_audit": ["slither", contract_dir, "--fail-high"],
    }


@dataclass(frozen=True)
class QuantumCandidateResult:
    algorithm: str
    circuit_depth: int
    two_qubit_gates: int
    fidelity: float


def select_best_quantum_algorithm(candidates: Sequence[QuantumCandidateResult]) -> QuantumCandidateResult:
    """Qiskit Competing-Algorithm search: maximize fidelity, breaking ties by shallower depth & fewer 2Q gates."""
    if not candidates:
        raise ValueError("Competing-Algorithm search requires at least one candidate")
    return sorted(
        candidates,
        key=lambda c: (-c.fidelity, c.circuit_depth, c.two_qubit_gates, c.algorithm),
    )[0]


def render_ros2_urdf_template(robot_name: str, joint_name: str, lower: float = -1.57, upper: float = 1.57) -> str:
    return (
        f'<robot name="{robot_name}">\n'
        '  <link name="base_link"/>\n'
        '  <link name="arm_link"/>\n'
        f'  <joint name="{joint_name}" type="revolute">\n'
        '    <parent link="base_link"/>\n'
        '    <child link="arm_link"/>\n'
        f'    <limit lower="{lower}" upper="{upper}" effort="10.0" velocity="1.0"/>\n'
        '  </joint>\n'
        '</robot>\n'
    )