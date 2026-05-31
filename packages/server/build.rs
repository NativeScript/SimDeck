use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        let stub = root.join("native_stubs.c");
        println!("cargo:rerun-if-changed={}", stub.display());
        cc::Build::new()
            .file(&stub)
            .flag_if_supported("-Wall")
            .flag_if_supported("-Wextra")
            .compile("xcw_native_bridge");
        return;
    }

    let cli = root.join("native");
    let camera = cli.join("camera");
    let native = cli.join("bridge");

    let files = [
        camera.join("SimDeckCameraService.m"),
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
    let x264_flags = pkg_config_flags("x264", true);
    build
        .files(files.iter())
        .include(&camera)
        .include(&cli)
        .include(&native)
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra");
    apply_pkg_config_compile_flags(&mut build, &x264_flags);

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        cli.join("DFPrivateSimulatorDisplayBridge.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        camera.join("SimDeckCameraShared.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        camera.join("SimDeckCameraInfo.plist").display()
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
    emit_pkg_config_link_flags(&x264_flags);

    for framework in [
        "Foundation",
        "Accelerate",
        "AppKit",
        "AVFoundation",
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
    println!(
        "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        camera.join("SimDeckCameraInfo.plist").display()
    );
}

fn pkg_config_flags(package: &str, static_link: bool) -> Vec<String> {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    let mut args = vec!["--cflags", "--libs"];
    if static_link {
        args.push("--static");
    }
    args.push(package);
    let output = Command::new("pkg-config")
        .args(args)
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

fn apply_pkg_config_compile_flags(build: &mut cc::Build, flags: &[String]) {
    for flag in flags {
        if let Some(path) = flag.strip_prefix("-I") {
            build.include(path);
        } else if !flag.starts_with("-L") && !flag.starts_with("-l") {
            build.flag(flag);
        }
    }
}

fn emit_pkg_config_link_flags(flags: &[String]) {
    let mut link_paths = Vec::new();
    for flag in flags {
        if let Some(path) = flag.strip_prefix("-L") {
            link_paths.push(PathBuf::from(path));
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            if lib == "x264" {
                if let Some(archive) = static_archive_for_lib(lib, &link_paths) {
                    println!("cargo:rustc-link-arg=-Wl,-force_load,{}", archive.display());
                } else {
                    println!("cargo:rustc-link-lib=static={lib}");
                }
            } else {
                println!("cargo:rustc-link-lib={lib}");
            }
        }
    }
}

fn static_archive_for_lib(lib: &str, link_paths: &[PathBuf]) -> Option<PathBuf> {
    let archive_name = format!("lib{lib}.a");
    link_paths
        .iter()
        .map(|path| path.join(&archive_name))
        .find(|path| path.is_file())
}
