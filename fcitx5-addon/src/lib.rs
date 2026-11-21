//! Flype 的 fcitx5 插件。
//!
//! 功能都在 `src/flype_engine.cpp` 里（跟 fcitx5 的 C++ API 打交道）；这个 crate 目前只
//! 负责用 cargo + `build.rs` 把 C++ 编成 `libflype.so`，以及把两边各自需要的符号钉住。
//! 等引擎迁到 Rust 之前，这里没有别的活。

extern crate flype_core;

use std::ffi::c_void;

unsafe extern "C" {
    /// C++ 侧的插件工厂（`build.rs` 把原来的导出名换成了这个，见那边的注释）
    fn flype_cpp_factory() -> *mut c_void;
}

/// fcitx5 按这个名字找插件工厂（5.1.8 起的 FCITX_ADDON_FACTORY_V2）
#[unsafe(no_mangle)]
pub extern "C" fn fcitx_addon_factory_instance_flype() -> *mut c_void {
    unsafe { flype_cpp_factory() }
}

/// 把 flype-core 的 rlib 带进链接（C++ 调的是它的 C ABI）。
///
/// 这个 crate 里没有别的地方用 flype-core，光写依赖 rustc 不会把 rlib 交给链接器。
/// fcitx5 不会调用它。
#[unsafe(no_mangle)]
pub extern "C" fn flype_addon_link_anchor() -> usize {
    flype_core::ffi::flype_engine_new as *const () as usize
}
