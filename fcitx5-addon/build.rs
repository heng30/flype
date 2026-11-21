//! fcitx5 插件的构建脚本（取代原来的 CMakeLists.txt）。
//!
//! 引擎还是 C++（`src/flype_engine.cpp`），用 cc 编成静态库，再由这个 crate 的 cdylib
//! 链成 `libflype.so`。只支持 fcitx5 5.1.11 以上。fcitx5 的头文件和库优先问 pkg-config；
//! 没有 .pc 时（Nix 的 fcitx5 只装 CMake 配置）从 `libFcitx5Core.so` 反推安装前缀。

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const LIB: &str = "libFcitx5Core.so";

/// 插件要求的 fcitx5 最低版本：5.1.11 起才有 showCustomInputMethodInformation
const MIN_FCITX: &str = "5.1.11";

/// fcitx5 的头文件目录和库目录
struct Fcitx {
    /// 头文件目录（`fcitx/addonfactory.h` 所在的那一层，逐个组件一个）
    includes: Vec<PathBuf>,
    /// `libFcitx5Core.so` 所在的目录
    lib_dir: PathBuf,
}

impl Fcitx {
    /// 按 环境变量 → pkg-config → 已知库目录 的顺序探测
    fn find() -> Self {
        let env_includes = env_paths("FLYPE_FCITX_INCLUDE_DIRS");
        let env_libs = env_paths("FLYPE_FCITX_LIB_DIRS");

        if let Some((pc_includes, pc_lib_dir)) = pkg_config() {
            return Self {
                includes: if env_includes.is_empty() {
                    pc_includes
                } else {
                    env_includes
                },
                lib_dir: env_libs.first().cloned().unwrap_or(pc_lib_dir),
            };
        }

        let mut dirs = env_libs;
        dirs.extend(default_lib_dirs());
        let lib = dirs
            .iter()
            .map(|dir| dir.join(LIB))
            .find(|path| path.exists())
            .unwrap_or_else(|| {
                panic!(
                    "找不到 {LIB}。装好 fcitx5 开发包，或用 FLYPE_FCITX_LIB_DIRS 指定库目录。"
                )
            });
        let lib_dir = lib.parent().unwrap_or(Path::new("/usr/lib")).to_path_buf();
        // Nix 的 profile 里全是符号链接，按真实路径才找得到 include
        let real = fs::canonicalize(&lib).unwrap_or_else(|_| lib.clone());
        let prefix = real
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("/usr"));
        let includes = if env_includes.is_empty() {
            includes_for_prefix(prefix)
        } else {
            env_includes
        };
        Self { includes, lib_dir }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest
        .parent()
        .expect("fcitx5-addon 必须放在 workspace 里")
        .to_path_buf();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();

    let fcitx = Fcitx::find();

    // 插件运行时要知道装完的设置程序、系统词库目录在哪
    let prefix = env::var("FLYPE_PREFIX").unwrap_or_else(|_| "/usr/local".to_string());
    let config_tool = env::var("FLYPE_CONFIG_TOOL")
        .unwrap_or_else(|_| format!("{prefix}/bin/flype-config"));
    let system_data = env::var("FLYPE_SYSTEM_DATA_DIR")
        .unwrap_or_else(|_| format!("{prefix}/share/flype"));

    write_conf(
        &manifest.join("conf/flype.conf.in"),
        &out.join("flype.conf"),
        &version,
    );
    write_conf(
        &manifest.join("conf/flype-addon.conf.in"),
        &out.join("flype-addon.conf"),
        &version,
    );
    // 安装脚本从固定的 target/addon 拿这两个清单（OUT_DIR 的哈希路径太难找）
    let addon_out = target_dir(&out).join("addon");
    for name in ["flype.conf", "flype-addon.conf"] {
        fs::create_dir_all(&addon_out).unwrap();
        fs::copy(out.join(name), addon_out.join(name)).unwrap();
    }

    println!("cargo:rustc-link-search=native={}", fcitx.lib_dir.display());
    for lib in ["Fcitx5Core", "Fcitx5Config", "Fcitx5Utils"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    // 插件工厂只有 fcitx5 在运行时按名字找，编译期没人引用；--gc-sections 会把它连同
    // 整个 C++ 静态库一起丢掉，Rust 的 cdylib 又不会把非 Rust 符号放进动态符号表。
    // 所以把 C++ 的工厂改个名，由 lib.rs 里同名的 Rust 导出符号转一手（这也顺带钉住了链接）。
    let factory = "fcitx_addon_factory_instance_flype";

    let version_def = quoted(&version);
    let config_tool_def = quoted(&config_tool);
    let data_def = quoted(&system_data);
    let mut engine = cc::Build::new();
    engine
        .cpp(true)
        .std("c++20")
        .warnings(false)
        .include(manifest.join("src"))
        .include(root.join("core/include"))
        .file(manifest.join("src/flype_engine.cpp"))
        .file(manifest.join("src/clipboard_copy.cpp"))
        .define("FLYPE_ADDON_VERSION", version_def.as_str())
        .define("FLYPE_CONFIG_TOOL", config_tool_def.as_str())
        .define("FLYPE_SYSTEM_DATA_DIR", data_def.as_str())
        .define(factory, "flype_cpp_factory");
    for include in &fcitx.includes {
        engine.include(include);
    }
    engine.compile("flype_addon_cpp");

    if env::var_os("CARGO_FEATURE_KEY_HARNESS").is_some() {
        build_key_harness(&manifest, &fcitx, &version);
    }

    for path in [
        "src/flype_engine.cpp",
        "src/flype_engine.h",
        "src/clipboard_copy.cpp",
        "src/clipboard_copy.h",
        "src/data_files.h",
        "src/flype_log.h",
        "test/key_harness.cpp",
        "conf/flype.conf.in",
        "conf/flype-addon.conf.in",
        "../core/include/flype_core.h",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=FLYPE_PREFIX");
    println!("cargo:rerun-if-env-changed=FLYPE_CONFIG_TOOL");
    println!("cargo:rerun-if-env-changed=FLYPE_SYSTEM_DATA_DIR");
    println!("cargo:rerun-if-env-changed=FLYPE_FCITX_INCLUDE_DIRS");
    println!("cargo:rerun-if-env-changed=FLYPE_FCITX_LIB_DIRS");
}

/// 按键行为测试：`main` 改名后由 Rust 的薄壳调用（C++ 的 main 没法直接当库函数用）
fn build_key_harness(manifest: &Path, fcitx: &Fcitx, version: &str) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let root = manifest.parent().unwrap();

    let entry_src = out.join("key_harness_entry.cpp");
    fs::write(
        &entry_src,
        "int flype_key_harness_main(int, char **argv);\n\
         extern \"C\" int flype_key_harness_entry(int argc, char **argv) {\n\
             return flype_key_harness_main(argc, argv);\n\
         }\n",
    )
    .unwrap();

    let mut harness = cc::Build::new();
    harness
        .cpp(true)
        .std("c++20")
        .warnings(false)
        .include(manifest.join("src"))
        .include(root.join("core/include"))
        .file(manifest.join("test/key_harness.cpp"))
        .define("main", "flype_key_harness_main")
        .define("FLYPE_ADDON_VERSION", quoted(version).as_str());
    for include in &fcitx.includes {
        harness.include(include);
    }
    harness.compile("flype_key_harness_cpp");

    let mut entry = cc::Build::new();
    entry
        .cpp(true)
        .std("c++20")
        .warnings(false)
        .include(manifest.join("src"))
        .file(manifest.join("src/clipboard_copy.cpp"))
        .file(&entry_src);
    for include in &fcitx.includes {
        entry.include(include);
    }
    entry.compile("flype_key_harness_entry");
}

fn write_conf(src: &Path, dst: &Path, version: &str) {
    let text = fs::read_to_string(src)
        .unwrap_or_else(|err| panic!("读不到 {}：{err}", src.display()));
    fs::write(
        dst,
        text.replace("@PROJECT_VERSION@", version)
            .replace("@FLYPE_FCITX_CORE_DEP@", MIN_FCITX),
    )
    .unwrap();
}

/// 环境变量里的路径列表（`:` 分隔，和 PATH 一样）
fn env_paths(name: &str) -> Vec<PathBuf> {
    env::var(name)
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default()
}

fn quoted(value: &str) -> String {
    format!("\"{value}\"")
}

/// 问 pkg-config 要头文件目录和库目录；没有 fcitx5 的 .pc 就返回 None
fn pkg_config() -> Option<(Vec<PathBuf>, PathBuf)> {
    let ask = |args: &[&str]| -> Option<String> {
        let output = Command::new("pkg-config").args(args).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    ask(&["--exists", "fcitx5core"])?;
    let cflags = ask(&["--cflags", "fcitx5core", "fcitx5config", "fcitx5utils"])?;
    let libs = ask(&["--libs-only-L", "fcitx5core", "fcitx5config", "fcitx5utils"])?;
    let flags = |text: &str, prefix: &str| -> Vec<PathBuf> {
        text.split_whitespace()
            .filter_map(|word| word.strip_prefix(prefix))
            .map(PathBuf::from)
            .collect()
    };
    let includes = flags(&cflags, "-I");
    let lib_dir = flags(&libs, "-L").into_iter().next()?;
    Some((includes, lib_dir))
}

/// 出厂默认的库目录；Nix 的 fcitx5 常常只在 per-user profile 里
fn default_lib_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(user) = env::var("USER") {
        dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/lib")));
    }
    if let Ok(profiles) = env::var("NIX_PROFILES") {
        dirs.extend(
            profiles
                .split_whitespace()
                .flat_map(|profile| ["lib", "lib64"].map(|sub| Path::new(profile).join(sub))),
        );
    }
    if let Ok(path) = env::var("LD_LIBRARY_PATH") {
        dirs.extend(env::split_paths(&path));
    }
    dirs.extend(
        [
            "/usr/local/lib",
            "/usr/lib",
            "/usr/lib64",
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/lib",
            "/lib64",
        ]
        .map(PathBuf::from),
    );
    dirs
}

/// fcitx5 各组件头文件目录（源码安装时可能直接是 `include/fcitx`）
fn includes_for_prefix(prefix: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["Core", "Config", "Utils"]
        .iter()
        .map(|component| prefix.join("include/Fcitx5").join(component))
        .collect();
    dirs.push(prefix.join("include"));
    dirs.retain(|dir| {
        ["fcitx", "fcitx-config", "fcitx-utils"]
            .iter()
            .any(|name| dir.join(name).is_dir())
    });
    dirs
}

/// `<target>/<profile>/build/<pkg>-<hash>/out` → `<target>/<profile>`
fn target_dir(out: &Path) -> PathBuf {
    out.ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| out.to_path_buf())
}
