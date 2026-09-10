use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/RobloxMultiAccountLauncher.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    let icon = manifest_dir
        .join("assets")
        .join("RobloxMultiAccountLauncher.ico");
    let rc_file = out_dir.join("rmal-icon.rc");
    let res_file = out_dir.join("rmal-icon.res");

    let icon_rc = icon
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    fs::write(&rc_file, format!("1 ICON \"{icon_rc}\"\r\n"))
        .expect("failed to create the Windows icon resource script");

    let rc = find_resource_compiler().expect(
        "Windows resource compiler rc.exe was not found. Install the Windows SDK/MSVC C++ build tools.",
    );
    let status = Command::new(rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res_file.display()))
        .arg(&rc_file)
        .status()
        .expect("failed to start rc.exe");

    if !status.success() {
        panic!("rc.exe failed while embedding the application icon");
    }

    println!(
        "cargo:rustc-link-arg-bin=roblox-multi-account-launcher={}",
        res_file.display()
    );
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(rc) = env::var_os("RC") {
        return Some(PathBuf::from(rc));
    }

    if let Ok(output) = Command::new("where.exe").arg("rc.exe").output() {
        if output.status.success() {
            if let Some(line) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find(|line| !line.trim().is_empty())
            {
                return Some(PathBuf::from(line.trim()));
            }
        }
    }

    for variable in ["WindowsSdkVerBinPath", "WindowsSdkBinPath"] {
        if let Some(base) = env::var_os(variable) {
            let candidate = PathBuf::from(base).join("x64").join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(program_files) = env::var_os(variable) else {
            continue;
        };
        let bin = PathBuf::from(program_files)
            .join("Windows Kits")
            .join("10")
            .join("bin");

        let direct = bin.join("x64").join("rc.exe");
        if direct.is_file() {
            return Some(direct);
        }

        let mut versions = match fs::read_dir(&bin) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        versions.sort();
        versions.reverse();

        for version in versions {
            let candidate = version.join("x64").join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}
