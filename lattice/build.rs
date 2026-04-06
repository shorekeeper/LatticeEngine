use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/shaders/passthrough.comp");

    let compiler = shaderc::Compiler::new().expect("shaderc: cannot create compiler");
    let mut opts = shaderc::CompileOptions::new().expect("shaderc: cannot create options");
    opts.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_2 as u32,
    );
    opts.set_optimization_level(shaderc::OptimizationLevel::Performance);

    let src = std::fs::read_to_string("src/shaders/passthrough.comp")
        .expect("Failed to read passthrough.comp");

    let artifact = compiler
        .compile_into_spirv(&src, shaderc::ShaderKind::Compute, "passthrough.comp", "main", Some(&opts))
        .expect("Failed to compile passthrough.comp");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("passthrough.comp.spv");
    std::fs::write(&out_path, artifact.as_binary_u8()).expect("Failed to write SPIR-V");

    println!(
        "cargo:warning=Compiled passthrough.comp -> {len} bytes of SPIR-V",
        len = artifact.as_binary_u8().len()
    );
}