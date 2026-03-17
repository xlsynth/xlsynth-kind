use std::env;
use std::path::Path;
use std::time::Instant;

use xlsynth_kind2::{
    FrontendOptions, Kind2CheckOutcome, Kind2Options, check_kind2_model, compile_dslx_path,
    emit_observational_equivalence_lustre, export_pir_fn_to_named_lustre_step,
    extract_kind2_counterexample, extract_kind2_progress_lines, extract_kind2_runtime_line,
};

#[path = "demos/observational/mod.rs"]
mod observational_demos;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "--list") {
        for demo in observational_demos::all_demos() {
            println!(
                "{} [{}]: {}",
                demo.slug,
                match demo.expectation {
                    observational_demos::DemoExpectation::Valid => "valid",
                    observational_demos::DemoExpectation::Invalid => "invalid",
                },
                demo.description
            );
        }
        return;
    }
    let show_progress = args.iter().any(|arg| arg == "--show-progress");
    let demo_slug = args.iter().find(|arg| !arg.starts_with("--"));
    let demo = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .and_then(|slug| observational_demos::find_demo(slug))
        .unwrap_or_else(|| {
            observational_demos::all_demos()
                .first()
                .expect("expected at least one observational demo")
        });
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dslx_path = repo_root.join(demo.file_rel_path);

    let frontend_options = FrontendOptions::default();
    let lhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.lhs_entry), &frontend_options)
        .expect("failed to compile lhs DSLX");
    let rhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.rhs_entry), &frontend_options)
        .expect("failed to compile rhs DSLX");

    let lhs = export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
        .expect("failed to export lhs step");
    let rhs = export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
        .expect("failed to export rhs step");

    let model = emit_observational_equivalence_lustre(&lhs, &rhs, demo.lhs_init, demo.rhs_init)
        .expect("failed to build observational equivalence model");
    let mut kind2_options = Kind2Options::default();
    if show_progress {
        kind2_options.verbosity = kind2_options.verbosity.max(2);
        kind2_options.live_output = true;
    }
    kind2_options.print_cex = true;
    let kind2_start = Instant::now();
    let outcome = check_kind2_model(&model.lustre_text, model.top_node.as_str(), &kind2_options)
        .expect("failed to run Kind2");
    let kind2_wall = kind2_start.elapsed();

    println!("demo: {}", demo.slug);
    if let Some(demo_slug) = demo_slug {
        println!("requested demo: {demo_slug}");
    }
    println!("description: {}", demo.description);
    println!("dslx path: {}", dslx_path.display());
    println!("lhs entry: {}", demo.lhs_entry);
    println!("rhs entry: {}", demo.rhs_entry);
    println!("lhs state width: {}", lhs.state_width);
    println!("rhs state width: {}", rhs.state_width);
    println!("proof property: {}", model.property_name);
    println!(
        "expected result: {}",
        match demo.expectation {
            observational_demos::DemoExpectation::Valid => "valid",
            observational_demos::DemoExpectation::Invalid => "invalid",
        }
    );
    let summary = match &outcome {
        Kind2CheckOutcome::Valid(summary) => {
            println!("Kind2 result: valid");
            if let Some(line) = summary.lines().find(|line| line.contains(": valid")) {
                println!("{line}");
            }
            summary
        }
        Kind2CheckOutcome::Invalid(summary) => {
            println!("Kind2 result: invalid");
            if let Some(line) = summary.lines().find(|line| line.contains(": invalid")) {
                println!("{line}");
            }
            summary
        }
        Kind2CheckOutcome::Unknown(summary) => {
            println!("Kind2 result: unknown");
            if let Some(line) = summary.lines().find(|line| line.contains(": unknown")) {
                println!("{line}");
            }
            summary
        }
    };
    println!("Kind2 wall time: {:.3}s", kind2_wall.as_secs_f64());
    if let Some(line) = extract_kind2_runtime_line(summary) {
        println!("{line}");
    }
    if matches!(outcome, Kind2CheckOutcome::Invalid(_)) {
        if let Some(counterexample) = extract_kind2_counterexample(summary) {
            println!("counterexample:");
            println!("{counterexample}");
        }
    }
    if show_progress {
        println!("kind2 progress:");
        for line in extract_kind2_progress_lines(summary) {
            println!("  {line}");
        }
    }
}
