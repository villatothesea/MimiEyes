fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../dist/macos/Info-unbundled.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        // `cargo run` has no .app Info.plist. NSBundle also reads privacy
        // declarations from this Mach-O section, so source builds can request
        // microphone access without requiring a separate packaging step.
        // hz: the bin ships as `mimieyes` (VENDORED.md #1).
        println!(
            "cargo:rustc-link-arg-bin=mimieyes=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    // hz: MimiEyes embeds its own icon (`mimieyes.rc` / `mimieyes.ico`,
    // VENDORED.md #1); upstream's zeron.rc stays untouched.
    println!("cargo:rerun-if-changed=../../dist/windows/mimieyes.rc");
    println!("cargo:rerun-if-changed=../../dist/windows/mimieyes.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile_for(
            "../../dist/windows/mimieyes.rc",
            // hz: the bin ships as `mimieyes` (VENDORED.md #1).
            &["mimieyes"],
            embed_resource::NONE,
        )
        .manifest_required()
        .expect("Windows app icon resource compilation failed");
    }
}
