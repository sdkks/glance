fn main() {
    println!("cargo:rustc-check-cfg=cfg(avif_native_built)");
    println!("cargo:rerun-if-changed=src/decode/avif_shim.c");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_AVIF_NATIVE").is_none()
        || !matches!(target.as_str(), "macos" | "linux")
    {
        println!(
            "cargo:warning=AVIF support not built: libavif/dav1d not found (feature disabled or unsupported target)"
        );
        return;
    }
    let probe = |name, version| {
        pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version(version)
            .probe(name)
    };
    let (Ok(avif), Ok(dav1d)) = (probe("libavif", "1.4.2"), probe("dav1d", "1.5.4")) else {
        println!(
            "cargo:warning=AVIF support not built: libavif/dav1d not found (requires libavif >=1.4.2, dav1d >=1.5.4)"
        );
        return;
    };
    let mut shim = cc::Build::new();
    shim.file("src/decode/avif_shim.c");
    for path in &avif.include_paths {
        shim.include(path);
    }
    shim.warnings(true).compile("glance_avif");
    for path in &avif.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    // Only libavif is called; it selects dav1d explicitly. No encoder APIs.
    for lib in &avif.libs {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rustc-cfg=avif_native_built");
    println!("cargo:rustc-env=GLANCE_LIBAVIF_VERSION={}", avif.version);
    println!("cargo:rustc-env=GLANCE_DAV1D_VERSION={}", dav1d.version);
}
