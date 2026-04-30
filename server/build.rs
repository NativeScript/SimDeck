use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
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
    build
        .files(files.iter())
        .include(&cli)
        .include(&native)
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .flag("-Wall")
        .flag("-Wextra");

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
