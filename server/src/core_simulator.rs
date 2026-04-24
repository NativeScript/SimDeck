use anyhow::{bail, Context};
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

pub fn start() -> anyhow::Result<()> {
    run_xcrun(["simctl", "list", "devices"])?;
    print_status("start");
    Ok(())
}

pub fn shutdown() -> anyhow::Result<()> {
    let _ = Command::new("xcrun")
        .args(["simctl", "shutdown", "all"])
        .output();
    stop_service()?;
    print_status("shutdown");
    Ok(())
}

pub fn restart() -> anyhow::Result<()> {
    let _ = Command::new("xcrun")
        .args(["simctl", "shutdown", "all"])
        .output();
    stop_service()?;
    thread::sleep(Duration::from_millis(500));
    run_xcrun(["simctl", "list", "devices"])?;
    print_status("restart");
    Ok(())
}

fn stop_service() -> anyhow::Result<()> {
    match Command::new("killall")
        .args(["-TERM", "com.apple.CoreSimulator.CoreSimulatorService"])
        .output()
        .context("run killall")?
    {
        output if output.status.success() => Ok(()),
        output => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("No matching processes") {
                Ok(())
            } else {
                bail_with_output("killall CoreSimulatorService", output).map(|_| ())
            }
        }
    }
}

fn run_xcrun<const N: usize>(args: [&str; N]) -> anyhow::Result<Output> {
    let output = Command::new("xcrun")
        .args(args)
        .output()
        .context("run xcrun")?;
    if output.status.success() {
        Ok(output)
    } else {
        bail_with_output(&format!("xcrun {}", args.join(" ")), output)
    }
}

fn bail_with_output(command: &str, output: Output) -> anyhow::Result<Output> {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    bail!(
        "{command} failed: {}",
        if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        }
    );
}

fn print_status(action: &str) {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "service": "com.apple.CoreSimulator.CoreSimulatorService",
            "action": action,
        }))
        .unwrap()
    );
}
