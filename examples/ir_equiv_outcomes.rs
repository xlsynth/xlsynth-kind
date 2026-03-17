use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use arbitrary::Unstructured;
use xlsynth::IrPackage;
use xlsynth_kind2::{EquivError, EquivOptions, EquivOutcome, prove_ir_text_step_equivalence};
use xlsynth_pir::ir::Type as PirType;
use xlsynth_pir::ir_fuzz::{generate_fuzz_sample_with_type, generate_ir_fn};

const TOP_NAME: &str = "fuzz_test";

fn recurrence_types(width: usize) -> (Vec<PirType>, PirType) {
    let bits = PirType::Bits(width);
    (
        vec![bits.clone(), bits.clone()],
        PirType::Tuple(vec![Box::new(bits.clone()), Box::new(bits)]),
    )
}

fn build_ir_from_bytes(data: &[u8]) -> Option<String> {
    let mut u = Unstructured::new(data);
    let width = usize::from(u.int_in_range::<u8>(1..=8).ok()?);
    let (param_types, return_type) = recurrence_types(width);
    let sample = generate_fuzz_sample_with_type(&mut u, &param_types, &return_type).ok()?;

    let mut package = IrPackage::new("fuzz_package").ok()?;
    generate_ir_fn(sample.ops, &mut package, Some(&return_type)).ok()?;
    Some(package.to_string())
}

fn load_inputs(args: &[String]) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    if args.is_empty() {
        let corpus_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fuzz")
            .join("corpus")
            .join("step_roundtrip_equiv");
        let mut entries = Vec::new();
        for entry in fs::read_dir(corpus_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                entries.push((name, fs::read(path)?));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(entries);
    }

    let mut entries = Vec::new();
    for arg in args {
        let path = PathBuf::from(arg);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(arg)
            .to_string();
        entries.push((name, fs::read(path)?));
    }
    Ok(entries)
}

fn summarize_error(err: &EquivError) -> &'static str {
    match err {
        EquivError::Export(_) => "export_skip",
        EquivError::Pipeline(_) => "pipeline_skip",
        EquivError::Xlsynth(_) => "xls_skip",
        EquivError::Kind2(_) => "kind2_error",
        EquivError::Io(_) => "io_error",
        EquivError::ToolFailure(_) => "tool_error",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let inputs = load_inputs(&args)?;
    let mut equivalent = 0usize;
    let mut unknown = 0usize;
    let mut disproved = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for (name, data) in inputs {
        let Some(ir_text) = build_ir_from_bytes(&data) else {
            println!("{name}: sample did not generate a recurrence-shaped IR function");
            skipped += 1;
            continue;
        };

        match prove_ir_text_step_equivalence(
            Path::new("fuzz_generated.ir"),
            &ir_text,
            Some(TOP_NAME),
            &EquivOptions::default(),
        ) {
            Ok(check) => match &check.outcome {
                EquivOutcome::Equivalent => {
                    equivalent += 1;
                    println!("{name}: equivalent");
                }
                EquivOutcome::Unknown(message) => {
                    unknown += 1;
                    let summary = message.lines().next().unwrap_or("unknown");
                    println!("{name}: unknown: {summary}");
                }
                EquivOutcome::Disproved(message) => {
                    disproved += 1;
                    let summary = message.lines().next().unwrap_or("disproved");
                    println!("{name}: disproved: {summary}");
                    println!("{ir_text}");
                }
            },
            Err(
                err @ (EquivError::Export(_) | EquivError::Pipeline(_) | EquivError::Xlsynth(_)),
            ) => {
                skipped += 1;
                println!("{name}: {}", summarize_error(&err));
            }
            Err(err) => {
                errors += 1;
                println!("{name}: {}: {err}", summarize_error(&err));
                println!("{ir_text}");
            }
        }
    }

    println!(
        "summary: equivalent={equivalent} unknown={unknown} disproved={disproved} skipped={skipped} errors={errors}"
    );
    Ok(())
}
