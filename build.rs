use std::{env, fs, path::PathBuf, process::Command};

const ICON_SIZE: u32 = 64;
const ICON_RGBA: &[u8] = include_bytes!("assets/RobloxMultiAccountLauncher64.rgba");

fn main() {
    println!("cargo:rerun-if-changed=assets/RobloxMultiAccountLauncher64.rgba");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    let icon = out_dir.join("rmal-icon.ico");
    let rc_file = out_dir.join("rmal-icon.rc");
    let res_file = out_dir.join("rmal-icon.res");

    write_windows_icon(&icon);

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

fn write_windows_icon(path: &PathBuf) {
    let expected = (ICON_SIZE * ICON_SIZE * 4) as usize;
    assert_eq!(
        ICON_RGBA.len(),
        expected,
        "runtime icon RGBA asset has an unexpected size"
    );

    let mask_stride = ICON_SIZE.div_ceil(32) * 4;
    let xor_size = ICON_SIZE * ICON_SIZE * 4;
    let mask_size = mask_stride * ICON_SIZE;
    let image_size = 40 + xor_size + mask_size;
    let image_offset = 6 + 16;
    let mut ico = Vec::with_capacity((image_offset + image_size) as usize);

    // ICONDIR: reserved, image type (1 = icon), image count.
    push_u16(&mut ico, 0);
    push_u16(&mut ico, 1);
    push_u16(&mut ico, 1);

    // ICONDIRENTRY. A zero width/height means 256; 64 is stored directly.
    ico.push(ICON_SIZE as u8);
    ico.push(ICON_SIZE as u8);
    ico.push(0);
    ico.push(0);
    push_u16(&mut ico, 1);
    push_u16(&mut ico, 32);
    push_u32(&mut ico, image_size);
    push_u32(&mut ico, image_offset);

    // BITMAPINFOHEADER. Icon DIB height includes both XOR and AND masks.
    push_u32(&mut ico, 40);
    push_i32(&mut ico, ICON_SIZE as i32);
    push_i32(&mut ico, (ICON_SIZE * 2) as i32);
    push_u16(&mut ico, 1);
    push_u16(&mut ico, 32);
    push_u32(&mut ico, 0); // BI_RGB
    push_u32(&mut ico, xor_size);
    push_i32(&mut ico, 0);
    push_i32(&mut ico, 0);
    push_u32(&mut ico, 0);
    push_u32(&mut ico, 0);

    // XOR bitmap: BGRA pixels, bottom-up.
    for y in (0..ICON_SIZE as usize).rev() {
        for x in 0..ICON_SIZE as usize {
            let index = (y * ICON_SIZE as usize + x) * 4;
            let r = ICON_RGBA[index];
            let g = ICON_RGBA[index + 1];
            let b = ICON_RGBA[index + 2];
            let a = ICON_RGBA[index + 3];
            ico.extend_from_slice(&[b, g, r, a]);
        }
    }

    // AND transparency mask, also bottom-up and padded to a 32-bit boundary.
    for y in (0..ICON_SIZE as usize).rev() {
        let row_start = ico.len();
        ico.resize(row_start + mask_stride as usize, 0);
        for x in 0..ICON_SIZE as usize {
            let alpha = ICON_RGBA[(y * ICON_SIZE as usize + x) * 4 + 3];
            if alpha < 128 {
                ico[row_start + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }

    fs::write(path, ico).expect("failed to create the Windows application icon");
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend_from_slice(&value.to_le_bytes());
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
