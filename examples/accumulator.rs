use std::path::Path;

use xlsynth_kind2::{FrontendOptions, compile_dslx_path};

fn main() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("dslx")
        .join("accumulator.x");
    let artifacts = compile_dslx_path(&path, Some("step"), &FrontendOptions::default())
        .expect("failed to compile DSLX to PIR");
    let signature = artifacts
        .recurrence_signature()
        .expect("top function does not match recurrence shape");

    println!("DSLX path: {}", path.display());
    println!("IR top function: {}", artifacts.top_fn().unwrap().name);
    println!(
        "Recurrence signature: input {}: {}, state {}: {}, output: {}",
        signature.input_name,
        signature.input_ty,
        signature.state_name,
        signature.state_ty,
        signature.output_ty
    );
    println!();
    println!("{}", artifacts.ir_text);
}
