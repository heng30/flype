//! 按键行为测试的入口；用例都在 `test/key_harness.cpp`（构建时把它的 `main` 改了名）。
//!
//! `#[link]` 是给这个 bin 自己点名的依赖：build script 的 `rustc-link-lib` 只跟着 lib 目标走，
//! bin 这里得自己写。

#[link(name = "flype_key_harness_cpp", kind = "static")]
#[link(name = "flype_key_harness_entry", kind = "static")]
#[link(name = "Fcitx5Core")]
#[link(name = "Fcitx5Utils")]
#[link(name = "Fcitx5Config")]
#[link(name = "stdc++")]
unsafe extern "C" {
    /// C++ 侧的入口（`test/key_harness.cpp` 的 main，构建时改名为 flype_key_harness_main）
    fn flype_key_harness_entry(argc: std::ffi::c_int, argv: *mut *mut std::ffi::c_char) -> std::ffi::c_int;
}

fn main() {
    // C++ 侧不用进程参数，自己拼固定的 fcitx5 启动参数
    std::process::exit(unsafe { flype_key_harness_entry(0, std::ptr::null_mut()) });
}
