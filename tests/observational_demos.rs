use std::path::Path;

use xlsynth_kind2::{
    FrontendOptions, Kind2CheckOutcome, Kind2Options, check_kind2_model, compile_dslx_path,
    emit_observational_equivalence_lustre, export_pir_fn_to_named_lustre_step,
};

#[allow(dead_code)]
#[path = "../examples/demos/observational/mod.rs"]
mod observational_demos;

#[test]
fn all_checked_in_observational_demos_prove_valid() {
    let kind2_options = Kind2Options::default();
    if !kind2_options.runner.is_available() {
        eprintln!(
            "Skipping demo proofs because {:?} is unavailable",
            kind2_options.runner
        );
        return;
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let frontend_options = FrontendOptions::default();
    for demo in observational_demos::valid_demos() {
        let dslx_path = repo_root.join(demo.file_rel_path);
        let lhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.lhs_entry), &frontend_options)
            .unwrap_or_else(|err| panic!("failed to compile {} lhs: {err}", demo.slug));
        let rhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.rhs_entry), &frontend_options)
            .unwrap_or_else(|err| panic!("failed to compile {} rhs: {err}", demo.slug));

        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap_or_else(|err| panic!("failed to export {} lhs: {err}", demo.slug));
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap_or_else(|err| panic!("failed to export {} rhs: {err}", demo.slug));
        let model = emit_observational_equivalence_lustre(&lhs, &rhs, demo.lhs_init, demo.rhs_init)
            .unwrap_or_else(|err| {
                panic!(
                    "failed to build observational model for {}: {err}",
                    demo.slug
                )
            });
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &kind2_options,
        )
        .unwrap_or_else(|err| panic!("Kind2 failed for {}: {err}", demo.slug));
        assert!(
            matches!(outcome, Kind2CheckOutcome::Valid(_)),
            "expected {} to prove valid, got {:?}",
            demo.slug,
            outcome
        );
    }
}

#[test]
fn all_checked_in_invalid_observational_demos_prove_invalid() {
    let kind2_options = Kind2Options::default();
    if !kind2_options.runner.is_available() {
        eprintln!(
            "Skipping demo proofs because {:?} is unavailable",
            kind2_options.runner
        );
        return;
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let frontend_options = FrontendOptions::default();
    for demo in observational_demos::invalid_demos() {
        let dslx_path = repo_root.join(demo.file_rel_path);
        let lhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.lhs_entry), &frontend_options)
            .unwrap_or_else(|err| panic!("failed to compile {} lhs: {err}", demo.slug));
        let rhs_artifacts = compile_dslx_path(&dslx_path, Some(demo.rhs_entry), &frontend_options)
            .unwrap_or_else(|err| panic!("failed to compile {} rhs: {err}", demo.slug));

        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap_or_else(|err| panic!("failed to export {} lhs: {err}", demo.slug));
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap_or_else(|err| panic!("failed to export {} rhs: {err}", demo.slug));
        let model = emit_observational_equivalence_lustre(&lhs, &rhs, demo.lhs_init, demo.rhs_init)
            .unwrap_or_else(|err| {
                panic!(
                    "failed to build observational model for {}: {err}",
                    demo.slug
                )
            });
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &kind2_options,
        )
        .unwrap_or_else(|err| panic!("Kind2 failed for {}: {err}", demo.slug));
        assert!(
            matches!(outcome, Kind2CheckOutcome::Invalid(_)),
            "expected {} to prove invalid, got {:?}",
            demo.slug,
            outcome
        );
    }
}
