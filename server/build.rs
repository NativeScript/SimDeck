use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        let stub = root.join("server/native_stubs.c");
        println!("cargo:rerun-if-changed={}", stub.display());
        cc::Build::new()
            .file(&stub)
            .flag("-Wall")
            .flag("-Wextra")
            .compile("xcw_native_bridge");
        return;
    }

    let cli = root.join("cli");
    let native = cli.join("native");

    let files = [
        cli.join("DFPrivateSimulatorDisplayBridge.m"),
        cli.join("XCWH264Encoder.m"),
        cli.join("XCWProcessRunner.m"),
        cli.join("XCWPrivateSimulatorBooter.m"),
        cli.join("XCWPrivateSimulatorSession.m"),
        cli.join("XCWAccessibilityBridge.m"),
        cli.join("XCWChromeRenderer.m"),
        cli.join("XCWSimctl.m"),
        native.join("XCWNativeSession.m"),
        native.join("XCWNativeBridge.m"),
    ];

    let mut build = cc::Build::new();
    let x264_flags = pkg_config_flags("x264");
    build
        .files(files.iter())
        .include(&cli)
        .include(&native)
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .flag("-Wall")
        .flag("-Wextra");
    apply_pkg_config_flags(&mut build, &x264_flags);

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("DFPrivateSimulatorDisplayBridge.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWH264Encoder.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWPrivateSimulatorBooter.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWPrivateSimulatorSession.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWAccessibilityBridge.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWChromeRenderer.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("XCWSimctl.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native.join("XCWNativeBridge.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        native.join("XCWNativeSession.h").display()
    );

    build.compile("xcw_native_bridge");

    for framework in [
        "Foundation",
        "Accelerate",
        "AppKit",
        "CoreImage",
        "CoreGraphics",
        "CoreMedia",
        "CoreVideo",
        "ImageIO",
        "QuartzCore",
        "VideoToolbox",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn pkg_config_flags(package: &str) -> Vec<String> {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    let output = Command::new("pkg-config")
        .args(["--cflags", "--libs", package])
        .output()
        .unwrap_or_else(|error| panic!("unable to run pkg-config for {package}: {error}"));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("pkg-config could not find required dependency `{package}`: {stderr}");
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn apply_pkg_config_flags(build: &mut cc::Build, flags: &[String]) {
    for flag in flags {
        if let Some(path) = flag.strip_prefix("-I") {
            build.include(path);
        } else if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        } else {
            build.flag(flag);
        }
    }
}
