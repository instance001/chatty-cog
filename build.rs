use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=vendor/llama/include/llama.h");
    println!("cargo:rerun-if-changed=vendor/llama/include/ggml.h");
    println!("cargo:rerun-if-changed=vendor/llama/include/ggml-alloc.h");
    println!("cargo:rerun-if-changed=vendor/llama/include/ggml-backend.h");

    // Help bindgen find libclang on Windows if LLVM is installed in the common location.
    // Users can still override with LIBCLANG_PATH.
    if env::var_os("LIBCLANG_PATH").is_none() {
        let default = PathBuf::from(r"C:\Program Files\LLVM\bin");
        if default.exists() {
            println!("cargo:rustc-env=LIBCLANG_PATH={}", default.display());
        }
    }

    let include_dir = PathBuf::from("vendor/llama/include");
    let header = include_dir.join("llama.h");

    let bindings = bindgen::Builder::default()
        .header(header.display().to_string())
        .clang_arg(format!("-I{}", include_dir.display()))
        .allowlist_type("llama_.*")
        .allowlist_type("ggml_.*")
        .allowlist_var("LLAMA_.*")
        .allowlist_var("GGML_.*")
        .allowlist_function("llama_.*")
        .blocklist_type("max_align_t")
        .derive_default(true)
        .generate_comments(false)
        .layout_tests(false)
        .generate()
        .expect("bindgen failed to generate llama.cpp bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"));
    bindings
        .write_to_file(out_path.join("llama_sys.rs"))
        .expect("failed to write bindings");
}
