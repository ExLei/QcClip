fn main() {
    if cfg!(debug_assertions)
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // GPUI 在未优化构建中渲染复杂页面时，默认的 1 MiB 主线程栈不足。
        println!("cargo:rustc-link-arg-bin=QcClip=/STACK:8388608");
    }

    println!("cargo:rerun-if-changed=assets/icon.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    embed_resource::compile("assets/icon.rc", embed_resource::NONE)
        .manifest_required()
        .expect("无法将应用图标嵌入 Windows 二进制");
}
