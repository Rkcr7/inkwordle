fn main() {
    if std::env::var("CARGO_FEATURE_TAKEOVER").is_ok() {
        // libquill.so + libqsgepaper.so from the quill project.
        let quill = concat!(env!("CARGO_MANIFEST_DIR"), "/../quill");
        println!("cargo:rustc-link-search=native={quill}/build");
        println!("cargo:rustc-link-search=native={quill}/vendor");
        println!("cargo:rustc-link-lib=dylib=quill");
        println!("cargo:rustc-link-lib=dylib=qsgepaper");
        println!("cargo:rustc-link-arg=-Wl,-rpath,/home/root/quill:/usr/lib/plugins/scenegraph");
        // Resolve libquill's transitive Qt deps at link time from the SDK
        // sysroot. rpath-link only (NOT link-search: the SDK's libc/libm are
        // linker scripts with absolute paths that break outside --sysroot).
        if let Ok(home) = std::env::var("HOME") {
            let sysroot = format!("{home}/rm-sdk-3.26/sysroots/cortexa53-crypto-remarkable-linux/usr/lib");
            println!("cargo:rustc-link-arg=-Wl,-rpath-link,{sysroot}");
        }
    }
}
