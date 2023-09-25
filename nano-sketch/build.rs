use std::process::Command;

fn main() {
    // Build test_lib crate
    let status = Command::new("cargo")
        .args(&["build", "--target", "thumbv6m-none-eabi", "--release", "-p", "test_lib"])
        .current_dir("../")
        .status()
        .expect("Failed to start cargo build");
    assert!(status.success(), "Failed to build test_lib");

    // Compile Arduino sketch
    let status = Command::new("arduino-cli")
        .args(&[
            "compile",
            "--fqbn",
            "arduino:mbed_nano:nanorp2040connect",
            ".",
            "--build-property",
            "compiler.libraries.ldflags=-ltest_lib -L../target/thumbv6m-none-eabi/release",
        ])
        .status()
        .expect("Failed to start arduino-cli compile");
    assert!(status.success(), "Failed to compile Arduino sketch");

    // List Arduino boards and find the correct port
    let output = Command::new("arduino-cli")
        .args(&["board", "list"])
        .output()
        .expect("Failed to list boards");
    let stdout = String::from_utf8(output.stdout).expect("Output was not valid UTF-8");
    let board_port: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("Arduino Nano RP2040 Connect"))
        .collect();
    let board_port = board_port[0]
        .split_whitespace()
        .next()
        .expect("Failed to get board port");

    // Upload Arduino sketch
    let status = Command::new("arduino-cli")
        .args(&[
            "upload",
            "--fqbn",
            "arduino:mbed_nano:nanorp2040connect",
            "--port",
            board_port,
            ".",
        ])
        .status()
        .expect("Failed to start arduino-cli upload");
    assert!(status.success(), "Failed to upload Arduino sketch");
}
