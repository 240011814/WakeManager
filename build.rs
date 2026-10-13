use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn main() {
    embuild::espidf::sysenv::output();
    emit_clangd_db();
}

/// 为 clangd 生成 compilation database：`<target>/clangd-db/compile_commands.json`。
///
/// `bindings.h` 里的 ESP-IDF 头文件（`esp_chip_info.h` 等）其 include 路径来自
/// esp-idf-sys 的 CMake 构建，编辑器里的 clangd 无从得知 → 报
/// "'esp_chip_info.h' file not found"。这里从 esp-idf-sys 产出的
/// `out/build/compile_commands.json` 收集全部 `-I`/`-isystem`/`-D`（含托管组件
/// mqtt / network_provisioning），为 `bindings.h` 合成一条条目。
///
/// 放 `<target>/clangd-db/` 而非仓库根 `build/`：后者被 `wokwi/mkwokwi.sh --real`
/// 每次 `rm -rf` 重建，放进去会被烧录任务顺手删掉。
fn emit_clangd_db() {
    let out_dir = match env::var("OUT_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => return,
    };
    // OUT_DIR = <target>/<triple>/<profile>/build/wol-rs-<hash>/out
    let build_root = match out_dir.parent().and_then(Path::parent) {
        Some(p) => p.to_path_buf(), // <target>/<triple>/<profile>/build
        None => return,
    };
    // <target>（CARGO_TARGET_DIR 可覆盖，与 cargo 语义一致）
    let target_dir = match env::var("CARGO_TARGET_DIR") {
        Ok(v) => {
            let p = PathBuf::from(v);
            if p.is_absolute() {
                p
            } else {
                PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into())).join(p)
            }
        }
        Err(_) => build_root
            .parent() // <profile>
            .and_then(Path::parent) // <triple>
            .and_then(Path::parent) // <target>
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("target")),
    };

    let Some(src_db) = newest_compile_db(&build_root) else {
        return; // esp-idf-sys 还没构建过（首次构建失败等）：跳过，下次 build 再生成
    };
    let Ok(src) = fs::read_to_string(&src_db) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&src) else {
        return;
    };
    let Some(entries) = json.as_array() else {
        return;
    };

    // 收集所有编译单元的 -I / -isystem / -D（相对路径按各自 directory 解析成绝对路径）
    let mut flags: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push = |f: String, flags: &mut Vec<String>, seen: &mut HashSet<String>| {
        if seen.insert(f.clone()) {
            flags.push(f);
        }
    };
    let mut compiler: Option<String> = None;
    let mut probe: Option<String> = None;
    for e in entries {
        let dir = e.get("directory").and_then(|v| v.as_str()).unwrap_or("");
        let cmd = e.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if compiler.is_none() {
            if let Some(first) = cmd.split_whitespace().next() {
                if first.ends_with("gcc") || first.ends_with("cc") {
                    compiler = Some(first.to_string());
                    probe = Some(probe_command(cmd));
                }
            }
        }
        let toks: Vec<String> = cmd
            .split_whitespace()
            .map(|t| t.replace('\\', ""))
            .collect();
        let mut i = 0;
        while i < toks.len() {
            let t = &toks[i];
            let rest = t.strip_prefix("-isystem");
            let inline = t.strip_prefix("-I").filter(|_| rest.is_none());
            if let Some(v) = rest.or(inline) {
                // -isystem<dir> / -I<dir>
                if !v.is_empty() {
                    push_abs(v, dir, &mut flags, &mut seen);
                } else if let Some(v) = toks.get(i + 1) {
                    push_abs(v, dir, &mut flags, &mut seen);
                    i += 1;
                }
            } else if t == "-D" {
                if let Some(v) = toks.get(i + 1) {
                    push(format!("-D{v}"), &mut flags, &mut seen);
                    i += 1;
                }
            } else if let Some(v) = t.strip_prefix("-D") {
                push(format!("-D{v}"), &mut flags, &mut seen);
            }
            i += 1;
        }
    }
    if flags.is_empty() {
        return;
    }

    // 守卫宏：esp-idf-sys 构建托管组件时才定义，CMake 的 compile DB 里没有，
    // 手工补上，bindings.h 里的 network_provisioning 头文件才能被 clangd 展开。
    for g in [
        "ESP_IDF_COMP_ESPRESSIF__NETWORK_PROVISIONING_ENABLED",
        "ESP_IDF_COMP_ESPRESSIF__MQTT_ENABLED",
    ] {
        push(format!("-D{g}"), &mut flags, &mut seen);
    }

    // gcc 的隐式头文件搜索目录（picolibc / newlib 的 `stdio.h`、`sys/features.h`
    // 等在其中）：clangd 不像 gcc 那样自动注入，得显式补 -isystem。裸跑 gcc 探测
    // 拿不到 picolibc 目录，必须用真实编译命令探测。
    if let (Some(cc), Some(probe)) = (&compiler, &probe) {
        for d in toolchain_system_includes(cc, probe) {
            if seen.insert(format!("isys:{d}")) {
                flags.push(format!("-isystem {d}"));
            }
        }
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    let mut cmd = compiler.unwrap_or_else(|| "clang".to_string());
    cmd.push_str(" -x c -fsyntax-only");
    for f in &flags {
        cmd.push(' ');
        cmd.push_str(f);
    }
    let header = manifest.join("bindings.h");
    cmd.push(' ');
    cmd.push_str(&header.display().to_string());

    let db: serde_json::Value = serde_json::json!([{
        "directory": manifest.display().to_string(),
        "command": cmd,
        "file": header.display().to_string(),
    }]);
    let out_dir = target_dir.join("clangd-db");
    if fs::create_dir_all(&out_dir).is_ok() {
        let _ = fs::write(
            out_dir.join("compile_commands.json"),
            serde_json::to_vec_pretty(&db).unwrap_or_default(),
        );
    }

    fn push_abs(p: &str, dir: &str, flags: &mut Vec<String>, seen: &mut HashSet<String>) {
        let abs = Path::new(p);
        let full = if abs.is_absolute() {
            abs.to_path_buf()
        } else {
            Path::new(dir).join(abs)
        };
        let s = full.display().to_string();
        if seen.insert(s.clone()) {
            flags.push(format!("-I{s}"));
        }
    }
}

/// 取最新一次 esp-idf-sys CMake 构建的 compile_commands.json
/// （esp-idf-sys-<hash> 目录随依赖版本/特征变化，不能写死）。
fn newest_compile_db(build_root: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for e in fs::read_dir(build_root).ok()?.flatten() {
        let p = e.path();
        if !p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("esp-idf-sys-"))
            .unwrap_or(false)
        {
            continue;
        }
        let db = p.join("out/build/compile_commands.json");
        let mtime = fs::metadata(&db).and_then(|m| m.modified()).ok();
        if let (Some(t), true) = (mtime, db.is_file()) {
            if best.as_ref().map(|(b, _)| t > *b).unwrap_or(true) {
                best = Some((t, db));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// 从一条真实编译命令里剥掉 `-c` / `-o <arg>` / 源文件，留下的参数足以让 gcc
/// 报告与真实构建一致的头文件搜索目录（`-E -Wp,-v`）。
fn probe_command(cmd: &str) -> String {
    let toks: Vec<&str> = cmd
        .split_whitespace()
        .map(|t| t.trim_end_matches('\\'))
        .collect();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        match t {
            "-c" => i += 1,
            "-o" => i += 2,
            _ if t.ends_with(".c") && t.contains('/') => i += 1,
            _ => {
                out.push(t);
                i += 1;
            }
        }
    }
    out.join(" ")
}

/// 用真实编译命令探测 gcc 的隐式头文件搜索目录，只保留工具链内部的
/// （picolibc / newlib / gcc 内建头），IDF 组件的 -I 目录早已单独收集。
fn toolchain_system_includes(gcc: &str, probe_cmd: &str) -> Vec<String> {
    let Ok(out) = std::process::Command::new("sh")
        .args(["-c", &format!("{probe_cmd} -E -Wp,-v -x c /dev/null")])
        .output()
    else {
        return Vec::new();
    };
    // gcc 把搜索目录打到 stderr
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    // 工具链根 = gcc 所在 bin/ 的上一级（.../xtensa-esp-elf/）
    let prefix = Path::new(gcc)
        .parent()
        .and_then(Path::parent)
        .and_then(|p| fs::canonicalize(p).ok())
        .map(|p| p.display().to_string());
    let mut dirs = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim_end_matches(" (system directory)").trim();
        if line.starts_with("#include <...> search starts here:") {
            inside = true;
        } else if line.starts_with("End of search list") {
            inside = false;
        } else if inside && !l.is_empty() {
            // 目录里带 `..`（bin/../picolibc/include），规范化后再按前缀过滤
            let p = Path::new(l);
            let full = fs::canonicalize(p)
                .map(|c| c.display().to_string())
                .unwrap_or_else(|_| l.to_string());
            let in_toolchain = prefix
                .as_ref()
                .map(|pre| full.starts_with(pre.as_str()))
                .unwrap_or(true);
            if in_toolchain {
                dirs.push(full);
            }
        }
    }
    dirs
}
