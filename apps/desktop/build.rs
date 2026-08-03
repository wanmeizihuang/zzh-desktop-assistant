use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    slint_build::compile("ui/app-window.slint").expect("failed to compile Slint UI");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources();
    }
}

fn compile_windows_resources() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let resource_dir = manifest_dir.join("assets").join("windows");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output dir")).join("app.res");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture");
    let sdk_arch = match target_arch.as_str() {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        other => panic!("unsupported Windows resource architecture: {other}"),
    };
    let resource_compiler = find_resource_compiler(sdk_arch)
        .expect("Windows SDK resource compiler rc.exe was not found");
    let status = Command::new(resource_compiler)
        .current_dir(&resource_dir)
        .arg("/nologo")
        .arg("/c65001")
        .arg("/fo")
        .arg(&output)
        .arg("app.rc")
        .status()
        .expect("failed to launch Windows resource compiler");
    assert!(status.success(), "Windows resource compilation failed");

    println!(
        "cargo::rustc-link-arg-bin=desktop-assistant={}",
        output.display()
    );
    println!("cargo::rerun-if-changed=assets/windows/app.rc");
    println!("cargo::rerun-if-changed=assets/windows/zzh-assistant.ico");
}

fn find_resource_compiler(architecture: &str) -> Option<PathBuf> {
    if let Some(path) = env::var_os("RC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let program_files = env::var_os("ProgramFiles(x86)")?;
    let bin_dir = PathBuf::from(program_files)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions = fs::read_dir(bin_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect::<Vec<_>>();
    versions.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
    versions
        .into_iter()
        .map(|entry| entry.path().join(architecture).join("rc.exe"))
        .find(|path| path.is_file())
}
