from systems_selt import (
    QuantumCandidateResult,
    audit_selt_numeric_reading,
    build_qemu_boot_argv,
    build_web3_verification_pipeline,
    check_gcc_differential_oracle,
    render_ros2_urdf_template,
    select_best_quantum_algorithm,
    verify_appsec_readonly_audit,
    verify_qemu_boot_log,
)


def test_qemu_boot_and_gcc_differential_oracles():
    argv = build_qemu_boot_argv("kernel.elf")
    assert argv[0] == "qemu-system-x86_64"
    assert "-nographic" in argv

    assert verify_qemu_boot_log("init ok\nKERNEL_BOOT_OK\n")["booted"] is True
    assert verify_qemu_boot_log("KERNEL PANIC: page fault")["booted"] is False

    assert check_gcc_differential_oracle("42\n", "42\n", 0, 0)["equivalent"] is True
    assert check_gcc_differential_oracle("42\n", "41\n", 0, 0)["equivalent"] is False


def test_chartevo_selt_near_miss_and_uncertain_interval_midpoint_ban():
    # Accurate (<3%) passes
    assert audit_selt_numeric_reading(101.0, 100.0)["ok"] is True

    # 3%-5% near-miss is flagged for re-measurement
    nm = audit_selt_numeric_reading(104.0, 100.0)
    assert nm["near_miss"] is True
    assert nm["status"] == "near_miss_audit"

    # Exact midpoint of uncertain interval [10, 20] (= 15.0) is banned
    mid = audit_selt_numeric_reading(15.0, 15.2, interval=(10.0, 20.0))
    assert mid["midpoint_violation"] is True
    assert mid["status"] == "midpoint_banned"


def test_appsec_web3_qiskit_and_ros2_templates():
    assert verify_appsec_readonly_audit(["read_file", "glob_grep"])["readonly_ok"] is True
    assert verify_appsec_readonly_audit(["read_file", "edit_file"])["readonly_ok"] is False

    w3 = build_web3_verification_pipeline("contracts")
    assert w3["foundry_test"][:2] == ["forge", "test"]
    assert w3["slither_audit"][0] == "slither"

    best = select_best_quantum_algorithm(
        [
            QuantumCandidateResult("QAOA-p1", circuit_depth=18, two_qubit_gates=12, fidelity=0.94),
            QuantumCandidateResult("VQE-UCCSD", circuit_depth=12, two_qubit_gates=8, fidelity=0.94),
            QuantumCandidateResult("Grover-opt", circuit_depth=10, two_qubit_gates=6, fidelity=0.98),
        ]
    )
    assert best.algorithm == "Grover-opt"

    urdf = render_ros2_urdf_template("meg_bot", "shoulder_joint")
    assert '<robot name="meg_bot">' in urdf
    assert 'name="shoulder_joint" type="revolute"' in urdf