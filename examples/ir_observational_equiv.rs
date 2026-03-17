use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use std::collections::BTreeMap;

use xlsynth_kind2::{
    AuxProofKind, ExportedAssumptionLustre, ExportedRelationLustre, ExportedStepLustre,
    FrontendArtifacts, Kind2CheckOutcome, Kind2Options, ObservationParamLayout,
    ObservationalEquivalenceModel, ProofPlan, check_kind2_model,
    emit_combined_assumption_lustre, emit_combined_relation_lustre,
    emit_invariant_init_lustre, emit_invariant_step_obligation_lustre,
    emit_observation_equivalence_relation_lustre, emit_observational_equivalence_lustre,
    emit_observational_equivalence_lustre_with_assumptions,
    emit_observational_equivalence_lustre_with_assumptions_and_relation,
    emit_observational_equivalence_lustre_with_assumptions_relation_and_observations,
    emit_state_equality_relation_lustre,
    emit_relation_init_lustre, emit_relation_invariant_lustre,
    emit_relation_step_obligation_lustre, emit_relation_step_obligation_lustre_with_observations,
    emit_state_projection_init_lustre, emit_state_projection_relation_lustre,
    emit_state_projection_step_obligation_lustre,
    ordered_aux_specs, parse_proof_plan,
    export_pir_fn_to_named_lustre_assumption_with_aggregates,
    export_pir_fn_to_named_lustre_full_step,
    export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates,
    export_pir_fn_to_named_lustre_next_state_projection_with_aggregates,
    export_pir_fn_to_named_lustre_observation_with_aggregates,
    export_pir_fn_to_named_lustre_relation_with_aggregates,
    export_pir_fn_to_named_lustre_state_projection_with_aggregates,
    export_pir_fn_to_named_lustre_step, extract_kind2_counterexample, extract_kind2_progress_lines,
    extract_kind2_runtime_line, parse_ir_text,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LustreMetrics {
    bytes: usize,
    lines: usize,
    node_defs: usize,
    local_bindings: usize,
    assignments: usize,
    ifs: usize,
    mods: usize,
}

fn lustre_metrics(text: &str) -> LustreMetrics {
    let mut node_defs = 0usize;
    let mut local_bindings = 0usize;
    let mut assignments = 0usize;
    let mut in_var_block = false;
    let mut in_let_block = false;
    for line in text.lines() {
        if line.starts_with("node ") {
            node_defs += 1;
        }
        match line {
            "var" => {
                in_var_block = true;
                in_let_block = false;
                continue;
            }
            "let" => {
                in_var_block = false;
                in_let_block = true;
                continue;
            }
            "tel" => {
                in_var_block = false;
                in_let_block = false;
                continue;
            }
            _ => {}
        }
        if in_var_block && line.starts_with("  ") && line.ends_with(';') {
            local_bindings += 1;
        }
        if in_let_block && line.starts_with("  ") && line.contains('=') && line.ends_with(';') {
            assignments += 1;
        }
    }
    LustreMetrics {
        bytes: text.len(),
        lines: text.lines().count(),
        node_defs,
        local_bindings,
        assignments,
        ifs: text.match_indices("if ").count(),
        mods: text.match_indices(" mod ").count(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: cargo run --example ir_observational_equiv -- \\
         <lhs_ir> <lhs_top> <rhs_ir> <rhs_top> \\
         [--proof-plan PLAN] \\
         [--lhs-init N] [--rhs-init N] [--lhs-assumption-top TOP] [--rhs-assumption-top TOP] \\
         [--observation-ir IR] [--observation-top TOP] [--lhs-observation-ir IR] [--lhs-observation-top TOP] [--rhs-observation-ir IR] [--rhs-observation-top TOP] \\
         [--state-projection-ir IR] [--state-projection-top TOP] [--lhs-state-projection-ir IR] [--lhs-state-projection-top TOP] [--rhs-state-projection-ir IR] [--rhs-state-projection-top TOP] \\
         [--relation-ir IR] [--relation-top TOP] [--decompose-relation] \\
         [--dump-lustre-dir DIR] \\
         [--kind2-arg ARG]... [--show-progress]"
    );
    std::process::exit(2);
}

fn maybe_dump_lustre(
    dump_lustre_dir: Option<&PathBuf>,
    file_name: &str,
    model: &ObservationalEquivalenceModel,
) {
    let Some(dump_lustre_dir) = dump_lustre_dir else {
        return;
    };
    fs::create_dir_all(dump_lustre_dir).expect("failed to create Lustre dump directory");
    let dump_path = dump_lustre_dir.join(file_name);
    fs::write(&dump_path, &model.lustre_text).expect("failed to write Lustre dump");
    println!("wrote lustre model: {}", dump_path.display());
    let metrics = lustre_metrics(&model.lustre_text);
    println!(
        "lustre metrics: bytes={} lines={} node_defs={} local_bindings={} assignments={} ifs={} mods={}",
        metrics.bytes,
        metrics.lines,
        metrics.node_defs,
        metrics.local_bindings,
        metrics.assignments,
        metrics.ifs,
        metrics.mods
    );
}

fn canonicalize_matching_input_aggregate_types(
    lhs: &ExportedStepLustre,
    rhs: &mut ExportedStepLustre,
) {
    let (Some(lhs_input), Some(rhs_input)) =
        (lhs.input_aggregate.as_ref(), rhs.input_aggregate.as_mut())
    else {
        return;
    };
    if lhs_input.fields != rhs_input.fields || lhs_input.type_name == rhs_input.type_name {
        return;
    }
    let old_type_name = rhs_input.type_name.clone();
    let new_type_name = lhs_input.type_name.clone();
    rhs_input.type_name = new_type_name.clone();
    for text in [
        &mut rhs.lustre_text,
        &mut rhs.state_only_lustre_text,
        &mut rhs.full_step_lustre_text,
    ] {
        *text = text.replace(&old_type_name, &new_type_name);
    }
    for type_def in &mut rhs.type_defs {
        *type_def = type_def.replace(&old_type_name, &new_type_name);
    }
}

fn renamed_step_export(
    step: &ExportedStepLustre,
    node_name: &str,
    state_only_node_name: &str,
) -> ExportedStepLustre {
    let mut renamed = step.clone();
    let old_node_name = renamed.node_name.clone();
    let old_state_only_node_name = renamed.state_only_node_name.clone();
    renamed.node_name = node_name.to_string();
    renamed.state_only_node_name = state_only_node_name.to_string();
    for text in [
        &mut renamed.lustre_text,
        &mut renamed.state_only_lustre_text,
        &mut renamed.full_step_lustre_text,
    ] {
        *text = text.replace(&old_state_only_node_name, state_only_node_name);
        *text = text.replace(&old_node_name, node_name);
    }
    for type_def in &mut renamed.type_defs {
        *type_def = type_def.replace(&old_state_only_node_name, state_only_node_name);
        *type_def = type_def.replace(&old_node_name, node_name);
    }
    if let Some(aggregate) = renamed.input_aggregate.as_mut() {
        aggregate.type_name = aggregate.type_name.replace(&old_node_name, node_name);
    }
    if let Some(aggregate) = renamed.state_aggregate.as_mut() {
        aggregate.type_name = aggregate.type_name.replace(&old_node_name, node_name);
    }
    if let Some(aggregate) = renamed.output_aggregate.as_mut() {
        aggregate.type_name = aggregate.type_name.replace(&old_node_name, node_name);
    }
    renamed
}

struct StepProofContext {
    lhs_path: PathBuf,
    lhs_top: String,
    lhs_text: String,
    lhs_artifacts: FrontendArtifacts,
    rhs_path: PathBuf,
    rhs_top: String,
    rhs_text: String,
    rhs_artifacts: Option<FrontendArtifacts>,
    same_step: bool,
    lhs: ExportedStepLustre,
    rhs: ExportedStepLustre,
}

enum ProvedAux {
    CrossCopy {
        relation: ExportedRelationLustre,
    },
    LhsInvariant {
        assumption: ExportedAssumptionLustre,
    },
    RhsInvariant {
        assumption: ExportedAssumptionLustre,
    },
}

struct DependencyRefs<'a> {
    relations: Vec<&'a ExportedRelationLustre>,
    lhs_invariants: Vec<&'a ExportedAssumptionLustre>,
    rhs_invariants: Vec<&'a ExportedAssumptionLustre>,
}

fn build_plan_step_context(
    lhs_path: PathBuf,
    lhs_top: String,
    rhs_path: PathBuf,
    rhs_top: String,
) -> StepProofContext {
    let lhs_text = fs::read_to_string(&lhs_path).expect("failed to read lhs ir");
    let rhs_text = fs::read_to_string(&rhs_path).expect("failed to read rhs ir");
    let lhs_artifacts =
        parse_ir_text(&lhs_path, &lhs_text, Some(&lhs_top)).expect("failed to parse lhs ir");
    let same_step = lhs_path == rhs_path && lhs_top == rhs_top;
    let rhs_artifacts = if same_step {
        None
    } else {
        Some(parse_ir_text(&rhs_path, &rhs_text, Some(&rhs_top)).expect("failed to parse rhs ir"))
    };
    let lhs = export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
        .expect("failed to export lhs");
    let mut rhs = if same_step {
        lhs.clone()
    } else {
        export_pir_fn_to_named_lustre_step(
            rhs_artifacts.as_ref().unwrap().top_fn().unwrap(),
            "step_impl_b",
        )
        .expect("failed to export rhs")
    };
    canonicalize_matching_input_aggregate_types(&lhs, &mut rhs);
    StepProofContext {
        lhs_path,
        lhs_top,
        lhs_text,
        lhs_artifacts,
        rhs_path,
        rhs_top,
        rhs_text,
        rhs_artifacts,
        same_step,
        lhs,
        rhs,
    }
}

fn default_ir_source<'a>(
    explicit_path: Option<&'a PathBuf>,
    default_path: &'a PathBuf,
    default_text: &'a str,
) -> (PathBuf, String) {
    match explicit_path {
        Some(path) if path != default_path => (
            path.clone(),
            fs::read_to_string(path).expect("failed to read helper ir"),
        ),
        Some(path) => (path.clone(), default_text.to_string()),
        None => (default_path.clone(), default_text.to_string()),
    }
}

fn effective_lhs_env<'a>(
    plan: &'a ProofPlan,
    aux: &'a xlsynth_kind2::AuxProofSpec,
) -> Option<(&'a str, Option<&'a PathBuf>)> {
    aux.lhs_assumption_top
        .as_deref()
        .map(|top| (top, aux.lhs_assumption_ir.as_ref()))
        .or_else(|| {
            plan.main
                .lhs_assumption_top
                .as_deref()
                .map(|top| (top, plan.main.lhs_assumption_ir.as_ref()))
        })
}

fn effective_rhs_env<'a>(
    plan: &'a ProofPlan,
    aux: &'a xlsynth_kind2::AuxProofSpec,
) -> Option<(&'a str, Option<&'a PathBuf>)> {
    aux.rhs_assumption_top
        .as_deref()
        .map(|top| (top, aux.rhs_assumption_ir.as_ref()))
        .or_else(|| {
            plan.main
                .rhs_assumption_top
                .as_deref()
                .map(|top| (top, plan.main.rhs_assumption_ir.as_ref()))
        })
}

fn lhs_helper_source(
    ctx: &StepProofContext,
    explicit_path: Option<&PathBuf>,
) -> (PathBuf, String) {
    default_ir_source(explicit_path, &ctx.lhs_path, &ctx.lhs_text)
}

fn rhs_helper_source(
    ctx: &StepProofContext,
    explicit_path: Option<&PathBuf>,
) -> (PathBuf, String) {
    default_ir_source(explicit_path, &ctx.rhs_path, &ctx.rhs_text)
}

fn export_lhs_assumption(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> ExportedAssumptionLustre {
    let (path, text) = lhs_helper_source(ctx, helper_path);
    let artifacts = parse_ir_text(&path, &text, Some(top)).expect("failed to parse lhs assumption");
    export_pir_fn_to_named_lustre_assumption_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.lhs.input_aggregate.as_ref(),
        ctx.lhs.state_aggregate.as_ref(),
    )
    .expect("failed to export lhs assumption")
}

fn export_rhs_assumption(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> ExportedAssumptionLustre {
    let (path, text) = rhs_helper_source(ctx, helper_path);
    let artifacts = parse_ir_text(&path, &text, Some(top)).expect("failed to parse rhs assumption");
    export_pir_fn_to_named_lustre_assumption_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.rhs.input_aggregate.as_ref(),
        ctx.rhs.state_aggregate.as_ref(),
    )
    .expect("failed to export rhs assumption")
}

fn export_lhs_observation(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedObservationLustre {
    let (path, text) = lhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse lhs observation");
    export_pir_fn_to_named_lustre_observation_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.lhs.input_aggregate.as_ref(),
        ctx.lhs.state_aggregate.as_ref(),
        ctx.lhs.output_aggregate.as_ref(),
    )
    .expect("failed to export lhs observation")
}

fn export_rhs_observation(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedObservationLustre {
    let (path, text) = rhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse rhs observation");
    export_pir_fn_to_named_lustre_observation_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.rhs.input_aggregate.as_ref(),
        ctx.rhs.state_aggregate.as_ref(),
        ctx.rhs.output_aggregate.as_ref(),
    )
    .expect("failed to export rhs observation")
}

fn export_lhs_state_projection(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedStateProjectionLustre {
    let (path, text) = lhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse lhs state projection");
    export_pir_fn_to_named_lustre_state_projection_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.lhs.input_aggregate.as_ref(),
        ctx.lhs.state_aggregate.as_ref(),
    )
    .expect("failed to export lhs state projection")
}

fn export_rhs_state_projection(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedStateProjectionLustre {
    let (path, text) = rhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse rhs state projection");
    export_pir_fn_to_named_lustre_state_projection_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.rhs.input_aggregate.as_ref(),
        ctx.rhs.state_aggregate.as_ref(),
    )
    .expect("failed to export rhs state projection")
}

fn export_lhs_next_state_projection(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedNextStateProjectionLustre {
    let (path, text) = lhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse lhs state projection");
    export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
        ctx.lhs_artifacts.top_fn().unwrap(),
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.lhs.input_aggregate.as_ref(),
        ctx.lhs.state_aggregate.as_ref(),
    )
    .expect("failed to export lhs next state projection")
}

fn export_rhs_next_state_projection(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> xlsynth_kind2::ExportedNextStateProjectionLustre {
    let (path, text) = rhs_helper_source(ctx, helper_path);
    let artifacts =
        parse_ir_text(&path, &text, Some(top)).expect("failed to parse rhs state projection");
    let rhs_step_fn = if ctx.same_step {
        ctx.lhs_artifacts.top_fn().unwrap()
    } else {
        ctx.rhs_artifacts.as_ref().unwrap().top_fn().unwrap()
    };
    export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
        rhs_step_fn,
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.rhs.input_aggregate.as_ref(),
        ctx.rhs.state_aggregate.as_ref(),
    )
    .expect("failed to export rhs next state projection")
}

fn export_relation(
    ctx: &StepProofContext,
    helper_path: Option<&PathBuf>,
    top: &str,
    node_name: &str,
) -> ExportedRelationLustre {
    let (path, text) = lhs_helper_source(ctx, helper_path);
    let artifacts = parse_ir_text(&path, &text, Some(top)).expect("failed to parse relation");
    export_pir_fn_to_named_lustre_relation_with_aggregates(
        artifacts.top_fn().unwrap(),
        node_name,
        ctx.lhs.input_aggregate.as_ref(),
        ctx.lhs.state_aggregate.as_ref(),
        ctx.rhs.state_aggregate.as_ref(),
    )
    .expect("failed to export relation")
}

fn collect_dependency_refs<'a>(
    names: &[String],
    proved: &'a BTreeMap<String, ProvedAux>,
) -> DependencyRefs<'a> {
    let mut relations = Vec::new();
    let mut lhs_invariants = Vec::new();
    let mut rhs_invariants = Vec::new();
    for name in names {
        match proved
            .get(name)
            .unwrap_or_else(|| panic!("missing proved aux dependency `{name}`"))
        {
            ProvedAux::CrossCopy { relation } => relations.push(relation),
            ProvedAux::LhsInvariant { assumption } => lhs_invariants.push(assumption),
            ProvedAux::RhsInvariant { assumption } => rhs_invariants.push(assumption),
        }
    }
    DependencyRefs {
        relations,
        lhs_invariants,
        rhs_invariants,
    }
}

fn plan_dump_dir(base: Option<&PathBuf>, stem: impl AsRef<str>) -> Option<PathBuf> {
    base.map(|base| base.join(stem.as_ref()))
}

fn maybe_combine_dependency_relations(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    node_name: &str,
    relations: &[&ExportedRelationLustre],
) -> Option<ExportedRelationLustre> {
    match relations {
        [] => None,
        [single] => Some((*single).clone()),
        _ => emit_combined_relation_lustre(lhs, rhs, node_name, relations)
            .expect("failed to combine dependency relations"),
    }
}

fn same_helper_top(
    ctx: &StepProofContext,
    lhs_ir: Option<&PathBuf>,
    lhs_top: &str,
    rhs_ir: Option<&PathBuf>,
    rhs_top: &str,
) -> bool {
    ctx.same_step && lhs_top == rhs_top && lhs_ir == rhs_ir
}

fn require_no_cross_copy_deps(
    aux_name: &str,
    deps: &DependencyRefs<'_>,
) {
    if !deps.relations.is_empty() {
        eprintln!(
            "aux `{aux_name}` is a single-side invariant and cannot depend on cross-copy aux lemmas"
        );
        std::process::exit(2);
    }
}

fn print_plan_context(ctx: &StepProofContext) {
    println!("lhs ir: {}", ctx.lhs_path.display());
    println!("lhs top: {}", ctx.lhs_top);
    println!("rhs ir: {}", ctx.rhs_path.display());
    println!("rhs top: {}", ctx.rhs_top);
}

fn run_observational_model(
    label: &str,
    model: &ObservationalEquivalenceModel,
    kind2_options: &Kind2Options,
    dump_dir: Option<&PathBuf>,
    file_name: &str,
    show_progress: bool,
) -> Kind2CheckOutcome {
    maybe_dump_lustre(dump_dir, file_name, model);
    let start = Instant::now();
    let outcome = check_kind2_model(&model.lustre_text, model.top_node.as_str(), kind2_options)
        .expect("failed to run Kind2");
    let wall = start.elapsed();
    report_outcome(label, &outcome, wall, show_progress);
    outcome
}

fn run_decomposed_relation_model(
    label_prefix: &str,
    init_model: &ObservationalEquivalenceModel,
    step_model: &ObservationalEquivalenceModel,
    kind2_options: &Kind2Options,
    dump_dir: Option<&PathBuf>,
    show_progress: bool,
) -> bool {
    let obligation_options = decomposed_relation_kind2_options(kind2_options);
    maybe_dump_lustre(dump_dir, "init.lus", init_model);
    let init_start = Instant::now();
    let init_outcome = check_kind2_model(
        &init_model.lustre_text,
        init_model.top_node.as_str(),
        &obligation_options,
    )
    .expect("failed to run Kind2 on init");
    report_outcome(
        &format!("{label_prefix} init"),
        &init_outcome,
        init_start.elapsed(),
        show_progress,
    );
    if !matches!(init_outcome, Kind2CheckOutcome::Valid(_)) {
        return false;
    }
    maybe_dump_lustre(dump_dir, "step.lus", step_model);
    let step_start = Instant::now();
    let step_outcome = check_kind2_model(
        &step_model.lustre_text,
        step_model.top_node.as_str(),
        &obligation_options,
    )
    .expect("failed to run Kind2 on step");
    report_outcome(
        &format!("{label_prefix} step"),
        &step_outcome,
        step_start.elapsed(),
        show_progress,
    );
    matches!(step_outcome, Kind2CheckOutcome::Valid(_))
}

fn proof_plan_main_kind2_options(base: &Kind2Options) -> Kind2Options {
    decomposed_relation_kind2_options(base)
}

fn run_proof_plan(
    ctx: &StepProofContext,
    lhs_init: u128,
    rhs_init: u128,
    plan: &ProofPlan,
    dump_lustre_dir: Option<&PathBuf>,
    kind2_options: &Kind2Options,
    show_progress: bool,
) {
    let ordered_aux = ordered_aux_specs(plan).expect("failed to order aux proof plan");
    let mut proved = BTreeMap::<String, ProvedAux>::new();
    for aux in ordered_aux {
        let deps = collect_dependency_refs(&aux.depends_on, &proved);
        let aux_dump_dir = plan_dump_dir(dump_lustre_dir, &aux.name);
        match aux.kind {
            AuxProofKind::ObservationEquiv => {
                let lhs_env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let rhs_env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
                lhs_assumptions.extend(deps.lhs_invariants.iter().copied());
                let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
                rhs_assumptions.extend(deps.rhs_invariants.iter().copied());
                let lhs_assumption = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &lhs_assumptions,
                )
                .expect("failed to combine lhs assumptions");
                let rhs_assumption = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &rhs_assumptions,
                )
                .expect("failed to combine rhs assumptions");
                let dep_relation = emit_combined_relation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    &format!("{}_deps", aux.name),
                    &deps.relations,
                )
                .expect("failed to combine dependency relations");
                let lhs_observation =
                    export_lhs_observation(ctx, aux.lhs_ir.as_ref(), aux.lhs_top.as_deref().unwrap(), &format!("{}_lhs_obs", aux.name));
                let rhs_observation =
                    export_rhs_observation(ctx, aux.rhs_ir.as_ref(), aux.rhs_top.as_deref().unwrap(), &format!("{}_rhs_obs", aux.name));
                let derived_relation = emit_observation_equivalence_relation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    &lhs_observation,
                    &rhs_observation,
                    &format!("{}_relation", aux.name),
                )
                .expect("failed to derive observation relation");
                let model = emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_init,
                    rhs_init,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    dep_relation.as_ref(),
                    Some(&lhs_observation),
                    Some(&rhs_observation),
                )
                .expect("failed to build aux observation model");
                let outcome = run_observational_model(
                    &format!("aux {}", aux.name),
                    &model,
                    kind2_options,
                    aux_dump_dir.as_ref(),
                    "observational_equivalence.lus",
                    show_progress,
                );
                if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                    std::process::exit(1);
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::CrossCopy {
                        relation: derived_relation,
                    },
                );
            }
            AuxProofKind::StateProjectionEquiv => {
                let lhs_env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let rhs_env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
                lhs_assumptions.extend(deps.lhs_invariants.iter().copied());
                let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
                rhs_assumptions.extend(deps.rhs_invariants.iter().copied());
                let lhs_assumption = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &lhs_assumptions,
                )
                .expect("failed to combine lhs assumptions");
                let rhs_assumption = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &rhs_assumptions,
                )
                .expect("failed to combine rhs assumptions");
                let dep_relation = emit_combined_relation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    &format!("{}_deps", aux.name),
                    &deps.relations,
                )
                .expect("failed to combine dependency relations");
                let lhs_projection = export_lhs_state_projection(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    &format!("{}_lhs_abs", aux.name),
                );
                let rhs_projection = if same_helper_top(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    aux.rhs_ir.as_ref(),
                    aux.rhs_top.as_deref().unwrap(),
                ) {
                    lhs_projection.clone()
                } else {
                    export_rhs_state_projection(
                        ctx,
                        aux.rhs_ir.as_ref(),
                        aux.rhs_top.as_deref().unwrap(),
                        &format!("{}_rhs_abs", aux.name),
                    )
                };
                let lhs_next_projection = export_lhs_next_state_projection(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    &format!("{}_lhs_next_abs", aux.name),
                );
                let rhs_next_projection = if same_helper_top(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    aux.rhs_ir.as_ref(),
                    aux.rhs_top.as_deref().unwrap(),
                ) {
                    lhs_next_projection.clone()
                } else {
                    export_rhs_next_state_projection(
                        ctx,
                        aux.rhs_ir.as_ref(),
                        aux.rhs_top.as_deref().unwrap(),
                        &format!("{}_rhs_next_abs", aux.name),
                    )
                };
                let derived_relation = emit_state_projection_relation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    &lhs_projection,
                    &rhs_projection,
                    &format!("{}_relation", aux.name),
                )
                .expect("failed to derive state projection relation");
                let init_model = emit_state_projection_init_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_init,
                    rhs_init,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    &lhs_projection,
                    &rhs_projection,
                )
                .expect("failed to build aux state projection init model");
                let step_model = emit_state_projection_step_obligation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    dep_relation.as_ref(),
                    None,
                    None,
                    &lhs_projection,
                    &rhs_projection,
                    None,
                    None,
                    Some(&lhs_next_projection),
                    Some(&rhs_next_projection),
                )
                .expect("failed to build aux state projection step model");
                if !run_decomposed_relation_model(
                    &format!("aux {}", aux.name),
                    &init_model,
                    &step_model,
                    kind2_options,
                    aux_dump_dir.as_ref(),
                    show_progress,
                ) {
                    std::process::exit(1);
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::CrossCopy {
                        relation: derived_relation,
                    },
                );
            }
            AuxProofKind::StateProjectionRelation => {
                let lhs_env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let rhs_env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
                lhs_assumptions.extend(deps.lhs_invariants.iter().copied());
                let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
                rhs_assumptions.extend(deps.rhs_invariants.iter().copied());
                let lhs_assumption = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &lhs_assumptions,
                )
                .expect("failed to combine lhs assumptions");
                let rhs_assumption = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &rhs_assumptions,
                )
                .expect("failed to combine rhs assumptions");
                let lhs_projection = export_lhs_state_projection(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    &format!("{}_lhs_abs", aux.name),
                );
                let rhs_projection = if same_helper_top(
                    ctx,
                    aux.lhs_ir.as_ref(),
                    aux.lhs_top.as_deref().unwrap(),
                    aux.rhs_ir.as_ref(),
                    aux.rhs_top.as_deref().unwrap(),
                ) {
                    lhs_projection.clone()
                } else {
                    export_rhs_state_projection(
                        ctx,
                        aux.rhs_ir.as_ref(),
                        aux.rhs_top.as_deref().unwrap(),
                        &format!("{}_rhs_abs", aux.name),
                    )
                };
                let base_relation = emit_state_projection_relation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    &lhs_projection,
                    &rhs_projection,
                    &format!("{}_relation", aux.name),
                )
                .expect("failed to derive state projection relation");
                let strengthened_relation = if deps.relations.is_empty() {
                    base_relation.clone()
                } else {
                    let mut relation_terms = deps.relations.clone();
                    relation_terms.push(&base_relation);
                    emit_combined_relation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        &format!("{}_strengthened", aux.name),
                        &relation_terms,
                    )
                    .expect("failed to combine relation terms")
                    .expect("relation set is non-empty")
                };
                if aux.decompose_relation {
                    let init_model = emit_relation_init_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation init model");
                    let step_model = emit_relation_step_obligation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation step model");
                    if !run_decomposed_relation_model(
                        &format!("aux {}", aux.name),
                        &init_model,
                        &step_model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        show_progress,
                    ) {
                        std::process::exit(1);
                    }
                } else {
                    let model = emit_relation_invariant_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation model");
                    let outcome = run_observational_model(
                        &format!("aux {}", aux.name),
                        &model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        "relation_invariant.lus",
                        show_progress,
                    );
                    if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                        std::process::exit(1);
                    }
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::CrossCopy {
                        relation: base_relation,
                    },
                );
            }
            AuxProofKind::StateRelation => {
                let lhs_env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let rhs_env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
                lhs_assumptions.extend(deps.lhs_invariants.iter().copied());
                let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
                rhs_assumptions.extend(deps.rhs_invariants.iter().copied());
                let lhs_assumption = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &lhs_assumptions,
                )
                .expect("failed to combine lhs assumptions");
                let rhs_assumption = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &rhs_assumptions,
                )
                .expect("failed to combine rhs assumptions");
                let base_relation =
                    emit_state_equality_relation_lustre(&ctx.lhs, &ctx.rhs, &format!("{}_relation", aux.name))
                        .expect("failed to derive direct state relation");
                let strengthened_relation = if deps.relations.is_empty() {
                    base_relation.clone()
                } else {
                    let mut relation_terms = deps.relations.clone();
                    relation_terms.push(&base_relation);
                    emit_combined_relation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        &format!("{}_strengthened", aux.name),
                        &relation_terms,
                    )
                    .expect("failed to combine relation terms")
                    .expect("relation set is non-empty")
                };
                if aux.decompose_relation {
                    let init_model = emit_relation_init_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation init model");
                    let step_model = emit_relation_step_obligation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation step model");
                    if !run_decomposed_relation_model(
                        &format!("aux {}", aux.name),
                        &init_model,
                        &step_model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        show_progress,
                    ) {
                        std::process::exit(1);
                    }
                } else {
                    let model = emit_relation_invariant_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation model");
                    let outcome = run_observational_model(
                        &format!("aux {}", aux.name),
                        &model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        "relation_invariant.lus",
                        show_progress,
                    );
                    if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                        std::process::exit(1);
                    }
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::CrossCopy {
                        relation: base_relation,
                    },
                );
            }
            AuxProofKind::Relation => {
                let lhs_env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let rhs_env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
                lhs_assumptions.extend(deps.lhs_invariants.iter().copied());
                let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
                rhs_assumptions.extend(deps.rhs_invariants.iter().copied());
                let lhs_assumption = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &lhs_assumptions,
                )
                .expect("failed to combine lhs assumptions");
                let rhs_assumption = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &rhs_assumptions,
                )
                .expect("failed to combine rhs assumptions");
                let base_relation =
                    export_relation(ctx, aux.ir.as_ref(), aux.top.as_deref().unwrap(), &format!("{}_base_relation", aux.name));
                let strengthened_relation = if deps.relations.is_empty() {
                    base_relation.clone()
                } else {
                    let mut relation_terms = deps.relations.clone();
                    relation_terms.push(&base_relation);
                    emit_combined_relation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        &format!("{}_strengthened", aux.name),
                        &relation_terms,
                    )
                    .expect("failed to combine relation terms")
                    .expect("relation set is non-empty")
                };
                if aux.decompose_relation {
                    let init_model = emit_relation_init_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation init model");
                    let step_model = emit_relation_step_obligation_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation step model");
                    if !run_decomposed_relation_model(
                        &format!("aux {}", aux.name),
                        &init_model,
                        &step_model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        show_progress,
                    ) {
                        std::process::exit(1);
                    }
                } else {
                    let model = emit_relation_invariant_lustre(
                        &ctx.lhs,
                        &ctx.rhs,
                        lhs_init,
                        rhs_init,
                        lhs_assumption.as_ref(),
                        rhs_assumption.as_ref(),
                        &strengthened_relation,
                    )
                    .expect("failed to build aux relation model");
                    let outcome = run_observational_model(
                        &format!("aux {}", aux.name),
                        &model,
                        kind2_options,
                        aux_dump_dir.as_ref(),
                        "relation_invariant.lus",
                        show_progress,
                    );
                    if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                        std::process::exit(1);
                    }
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::CrossCopy {
                        relation: base_relation,
                    },
                );
            }
            AuxProofKind::LhsInvariant => {
                require_no_cross_copy_deps(&aux.name, &deps);
                let env = effective_lhs_env(plan, aux).map(|(top, ir)| {
                    export_lhs_assumption(ctx, ir, top, &format!("{}_lhs_env", aux.name))
                });
                let mut assumptions = env.as_ref().into_iter().collect::<Vec<_>>();
                assumptions.extend(deps.lhs_invariants.iter().copied());
                let combined_env = emit_combined_assumption_lustre(
                    &ctx.lhs,
                    &format!("{}_lhs_assumption", aux.name),
                    &assumptions,
                )
                .expect("failed to combine lhs invariant assumptions");
                let invariant =
                    export_lhs_assumption(ctx, aux.ir.as_ref(), aux.top.as_deref().unwrap(), &format!("{}_lhs_invariant", aux.name));
                let init_model = emit_invariant_init_lustre(
                    &ctx.lhs,
                    lhs_init,
                    combined_env.as_ref(),
                    &invariant,
                )
                .expect("failed to build lhs invariant init model");
                let step_model = emit_invariant_step_obligation_lustre(
                    &ctx.lhs,
                    combined_env.as_ref(),
                    &invariant,
                )
                .expect("failed to build lhs invariant step model");
                if !run_decomposed_relation_model(
                    &format!("aux {}", aux.name),
                    &init_model,
                    &step_model,
                    kind2_options,
                    aux_dump_dir.as_ref(),
                    show_progress,
                ) {
                    std::process::exit(1);
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::LhsInvariant { assumption: invariant },
                );
            }
            AuxProofKind::RhsInvariant => {
                require_no_cross_copy_deps(&aux.name, &deps);
                let env = effective_rhs_env(plan, aux).map(|(top, ir)| {
                    export_rhs_assumption(ctx, ir, top, &format!("{}_rhs_env", aux.name))
                });
                let mut assumptions = env.as_ref().into_iter().collect::<Vec<_>>();
                assumptions.extend(deps.rhs_invariants.iter().copied());
                let combined_env = emit_combined_assumption_lustre(
                    &ctx.rhs,
                    &format!("{}_rhs_assumption", aux.name),
                    &assumptions,
                )
                .expect("failed to combine rhs invariant assumptions");
                let invariant =
                    export_rhs_assumption(ctx, aux.ir.as_ref(), aux.top.as_deref().unwrap(), &format!("{}_rhs_invariant", aux.name));
                let init_model = emit_invariant_init_lustre(
                    &ctx.rhs,
                    rhs_init,
                    combined_env.as_ref(),
                    &invariant,
                )
                .expect("failed to build rhs invariant init model");
                let step_model = emit_invariant_step_obligation_lustre(
                    &ctx.rhs,
                    combined_env.as_ref(),
                    &invariant,
                )
                .expect("failed to build rhs invariant step model");
                if !run_decomposed_relation_model(
                    &format!("aux {}", aux.name),
                    &init_model,
                    &step_model,
                    kind2_options,
                    aux_dump_dir.as_ref(),
                    show_progress,
                ) {
                    std::process::exit(1);
                }
                proved.insert(
                    aux.name.clone(),
                    ProvedAux::RhsInvariant { assumption: invariant },
                );
            }
        }
    }

    let main_deps = collect_dependency_refs(&plan.main.depends_on, &proved);
    let lhs_env = plan
        .main
        .lhs_assumption_top
        .as_deref()
        .map(|top| export_lhs_assumption(ctx, plan.main.lhs_assumption_ir.as_ref(), top, "main_lhs_env"));
    let rhs_env = plan
        .main
        .rhs_assumption_top
        .as_deref()
        .map(|top| export_rhs_assumption(ctx, plan.main.rhs_assumption_ir.as_ref(), top, "main_rhs_env"));
    let mut lhs_assumptions = lhs_env.as_ref().into_iter().collect::<Vec<_>>();
    lhs_assumptions.extend(main_deps.lhs_invariants.iter().copied());
    let mut rhs_assumptions = rhs_env.as_ref().into_iter().collect::<Vec<_>>();
    rhs_assumptions.extend(main_deps.rhs_invariants.iter().copied());
    let lhs_assumption = emit_combined_assumption_lustre(&ctx.lhs, "main_lhs_assumption", &lhs_assumptions)
        .expect("failed to combine main lhs assumptions");
    let rhs_assumption = emit_combined_assumption_lustre(&ctx.rhs, "main_rhs_assumption", &rhs_assumptions)
        .expect("failed to combine main rhs assumptions");
    let dep_relation = maybe_combine_dependency_relations(
        &ctx.lhs,
        &ctx.rhs,
        "main_dep_relation",
        &main_deps.relations,
    );
    let main_kind2_options = proof_plan_main_kind2_options(kind2_options);
    print_plan_context(ctx);
    match plan.main.kind {
        xlsynth_kind2::MainProofKind::Observational => {
            let lhs_observation = plan
                .main
                .lhs_observation_top
                .as_deref()
                .map(|top| export_lhs_observation(ctx, plan.main.lhs_observation_ir.as_ref(), top, "main_lhs_observation"));
            let rhs_observation = plan
                .main
                .rhs_observation_top
                .as_deref()
                .map(|top| export_rhs_observation(ctx, plan.main.rhs_observation_ir.as_ref(), top, "main_rhs_observation"));
            let model = emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
                &ctx.lhs,
                &ctx.rhs,
                lhs_init,
                rhs_init,
                lhs_assumption.as_ref(),
                rhs_assumption.as_ref(),
                dep_relation.as_ref(),
                lhs_observation.as_ref(),
                rhs_observation.as_ref(),
            )
            .expect("failed to build main observational model");
            let outcome = run_observational_model(
                "main theorem",
                &model,
                &main_kind2_options,
                dump_lustre_dir,
                "observational_equivalence.lus",
                show_progress,
            );
            if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                std::process::exit(1);
            }
        }
        xlsynth_kind2::MainProofKind::StateProjection => {
            let lhs_projection = export_lhs_state_projection(
                ctx,
                plan.main.lhs_state_projection_ir.as_ref(),
                plan.main.lhs_state_projection_top.as_deref().unwrap(),
                "main_lhs_state_projection",
            );
            let rhs_projection = export_rhs_state_projection(
                ctx,
                plan.main.rhs_state_projection_ir.as_ref(),
                plan.main.rhs_state_projection_top.as_deref().unwrap(),
                "main_rhs_state_projection",
            );
            let lhs_observation = plan
                .main
                .lhs_observation_top
                .as_deref()
                .map(|top| export_lhs_observation(ctx, plan.main.lhs_observation_ir.as_ref(), top, "main_lhs_observation"));
            let rhs_observation = plan
                .main
                .rhs_observation_top
                .as_deref()
                .map(|top| export_rhs_observation(ctx, plan.main.rhs_observation_ir.as_ref(), top, "main_rhs_observation"));
            let needs_step_output = match (lhs_observation.as_ref(), rhs_observation.as_ref()) {
                (None, None) => true,
                (Some(lhs_observation), Some(rhs_observation)) => {
                    matches!(
                        lhs_observation.param_layout,
                        ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
                    ) || matches!(
                        rhs_observation.param_layout,
                        ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
                    )
                }
                _ => true,
            };
            let lhs_next_projection = if !needs_step_output {
                Some(export_lhs_next_state_projection(
                    ctx,
                    plan.main.lhs_state_projection_ir.as_ref(),
                    plan.main.lhs_state_projection_top.as_deref().unwrap(),
                    "main_lhs_next_state_projection",
                ))
            } else {
                None
            };
            let rhs_next_projection = if !needs_step_output {
                Some(export_rhs_next_state_projection(
                    ctx,
                    plan.main.rhs_state_projection_ir.as_ref(),
                    plan.main.rhs_state_projection_top.as_deref().unwrap(),
                    "main_rhs_next_state_projection",
                ))
            } else {
                None
            };
            let init_model = emit_state_projection_init_lustre(
                &ctx.lhs,
                &ctx.rhs,
                lhs_init,
                rhs_init,
                lhs_assumption.as_ref(),
                rhs_assumption.as_ref(),
                &lhs_projection,
                &rhs_projection,
            )
            .expect("failed to build main state projection init model");
            let step_model = emit_state_projection_step_obligation_lustre(
                &ctx.lhs,
                &ctx.rhs,
                lhs_assumption.as_ref(),
                rhs_assumption.as_ref(),
                dep_relation.as_ref(),
                lhs_observation.as_ref(),
                rhs_observation.as_ref(),
                &lhs_projection,
                &rhs_projection,
                None,
                None,
                lhs_next_projection.as_ref(),
                rhs_next_projection.as_ref(),
            )
            .expect("failed to build main state projection step model");
            if !run_decomposed_relation_model(
                "main theorem",
                &init_model,
                &step_model,
                &main_kind2_options,
                dump_lustre_dir,
                show_progress,
            ) {
                std::process::exit(1);
            }
        }
        xlsynth_kind2::MainProofKind::Relation => {
            let base_relation = export_relation(
                ctx,
                plan.main.relation_ir.as_ref(),
                plan.main.relation_top.as_deref().unwrap(),
                "main_base_relation",
            );
            let mut relation_terms = main_deps.relations.clone();
            relation_terms.push(&base_relation);
            let strengthened_relation = emit_combined_relation_lustre(
                &ctx.lhs,
                &ctx.rhs,
                "main_strengthened_relation",
                &relation_terms,
            )
            .expect("failed to combine main relation terms")
            .expect("relation set is non-empty");
            if plan.main.decompose_relation {
                let init_model = emit_relation_init_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_init,
                    rhs_init,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    &strengthened_relation,
                )
                .expect("failed to build main relation init model");
                let step_model = emit_relation_step_obligation_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    &strengthened_relation,
                )
                .expect("failed to build main relation step model");
                if !run_decomposed_relation_model(
                    "main theorem",
                    &init_model,
                    &step_model,
                    &main_kind2_options,
                    dump_lustre_dir,
                    show_progress,
                ) {
                    std::process::exit(1);
                }
            } else {
                let model = emit_relation_invariant_lustre(
                    &ctx.lhs,
                    &ctx.rhs,
                    lhs_init,
                    rhs_init,
                    lhs_assumption.as_ref(),
                    rhs_assumption.as_ref(),
                    &strengthened_relation,
                )
                .expect("failed to build main relation model");
                let outcome = run_observational_model(
                    "main theorem",
                    &model,
                    &main_kind2_options,
                    dump_lustre_dir,
                    "relation_invariant.lus",
                    show_progress,
                );
                if !matches!(outcome, Kind2CheckOutcome::Valid(_)) {
                    std::process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 4 {
        usage();
    }

    let mut positional = Vec::new();
    let mut lhs_init = 0u128;
    let mut rhs_init = 0u128;
    let mut proof_plan_path: Option<PathBuf> = None;
    let mut lhs_assumption_top: Option<String> = None;
    let mut rhs_assumption_top: Option<String> = None;
    let mut shared_observation_ir: Option<String> = None;
    let mut shared_observation_top: Option<String> = None;
    let mut lhs_observation_ir: Option<String> = None;
    let mut lhs_observation_top: Option<String> = None;
    let mut rhs_observation_ir: Option<String> = None;
    let mut rhs_observation_top: Option<String> = None;
    let mut shared_state_projection_ir: Option<String> = None;
    let mut shared_state_projection_top: Option<String> = None;
    let mut lhs_state_projection_ir: Option<String> = None;
    let mut lhs_state_projection_top: Option<String> = None;
    let mut rhs_state_projection_ir: Option<String> = None;
    let mut rhs_state_projection_top: Option<String> = None;
    let mut relation_ir: Option<String> = None;
    let mut relation_top: Option<String> = None;
    let mut decompose_relation = false;
    let mut dump_lustre_dir: Option<PathBuf> = None;
    let mut kind2_args: Vec<String> = Vec::new();
    let mut show_progress = false;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--lhs-init" => {
                index += 1;
                lhs_init = args.get(index).unwrap_or_else(|| usage()).parse().unwrap();
            }
            "--rhs-init" => {
                index += 1;
                rhs_init = args.get(index).unwrap_or_else(|| usage()).parse().unwrap();
            }
            "--proof-plan" => {
                index += 1;
                proof_plan_path = Some(PathBuf::from(
                    args.get(index).unwrap_or_else(|| usage()).to_string(),
                ));
            }
            "--lhs-assumption-top" => {
                index += 1;
                lhs_assumption_top = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--rhs-assumption-top" => {
                index += 1;
                rhs_assumption_top = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--observation-ir" => {
                index += 1;
                shared_observation_ir =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--observation-top" => {
                index += 1;
                shared_observation_top =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--lhs-observation-ir" => {
                index += 1;
                lhs_observation_ir = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--lhs-observation-top" => {
                index += 1;
                lhs_observation_top = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--rhs-observation-ir" => {
                index += 1;
                rhs_observation_ir = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--rhs-observation-top" => {
                index += 1;
                rhs_observation_top = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--state-projection-ir" => {
                index += 1;
                shared_state_projection_ir =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--state-projection-top" => {
                index += 1;
                shared_state_projection_top =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--lhs-state-projection-ir" => {
                index += 1;
                lhs_state_projection_ir =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--lhs-state-projection-top" => {
                index += 1;
                lhs_state_projection_top =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--rhs-state-projection-ir" => {
                index += 1;
                rhs_state_projection_ir =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--rhs-state-projection-top" => {
                index += 1;
                rhs_state_projection_top =
                    Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--relation-ir" => {
                index += 1;
                relation_ir = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--relation-top" => {
                index += 1;
                relation_top = Some(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--decompose-relation" => {
                decompose_relation = true;
            }
            "--dump-lustre-dir" => {
                index += 1;
                dump_lustre_dir = Some(PathBuf::from(
                    args.get(index).unwrap_or_else(|| usage()).to_string(),
                ));
            }
            "--kind2-arg" => {
                index += 1;
                kind2_args.push(args.get(index).unwrap_or_else(|| usage()).to_string());
            }
            "--show-progress" => {
                show_progress = true;
            }
            other => positional.push(other.to_string()),
        }
        index += 1;
    }
    if positional.len() != 4 {
        usage();
    }

    let lhs_path = PathBuf::from(&positional[0]);
    let lhs_top = &positional[1];
    let rhs_path = PathBuf::from(&positional[2]);
    let rhs_top = &positional[3];
    if let Some(proof_plan_path) = proof_plan_path.as_ref() {
        let legacy_helper_flags_present = lhs_assumption_top.is_some()
            || rhs_assumption_top.is_some()
            || lhs_observation_top.is_some()
            || rhs_observation_top.is_some()
            || lhs_state_projection_top.is_some()
            || rhs_state_projection_top.is_some()
            || relation_top.is_some()
            || relation_ir.is_some()
            || lhs_observation_ir.is_some()
            || rhs_observation_ir.is_some()
            || lhs_state_projection_ir.is_some()
            || rhs_state_projection_ir.is_some()
            || shared_observation_ir.is_some()
            || shared_observation_top.is_some()
            || shared_state_projection_ir.is_some()
            || shared_state_projection_top.is_some()
            || decompose_relation;
        if legacy_helper_flags_present {
            eprintln!("--proof-plan cannot be combined with legacy helper-selection flags");
            std::process::exit(2);
        }
        let mut kind2_options = Kind2Options::default();
        if show_progress {
            kind2_options.verbosity = kind2_options.verbosity.max(2);
            kind2_options.live_output = true;
        }
        kind2_options.print_cex = true;
        kind2_options.extra_args = kind2_args;
        let ctx = build_plan_step_context(
            lhs_path.clone(),
            lhs_top.to_string(),
            rhs_path.clone(),
            rhs_top.to_string(),
        );
        let plan = parse_proof_plan(proof_plan_path).expect("failed to parse proof plan");
        run_proof_plan(
            &ctx,
            lhs_init,
            rhs_init,
            &plan,
            dump_lustre_dir.as_ref(),
            &kind2_options,
            show_progress,
        );
        return;
    }
    if lhs_observation_top.is_none() {
        lhs_observation_top = shared_observation_top.clone();
    }
    if rhs_observation_top.is_none() {
        rhs_observation_top = shared_observation_top;
    }
    if lhs_observation_ir.is_none() {
        lhs_observation_ir = shared_observation_ir.clone();
    }
    if rhs_observation_ir.is_none() {
        rhs_observation_ir = shared_observation_ir;
    }
    if lhs_state_projection_top.is_none() {
        lhs_state_projection_top = shared_state_projection_top.clone();
    }
    if rhs_state_projection_top.is_none() {
        rhs_state_projection_top = shared_state_projection_top;
    }
    if lhs_state_projection_ir.is_none() {
        lhs_state_projection_ir = shared_state_projection_ir.clone();
    }
    if rhs_state_projection_ir.is_none() {
        rhs_state_projection_ir = shared_state_projection_ir;
    }
    if relation_top.is_some()
        && (lhs_state_projection_top.is_some() || rhs_state_projection_top.is_some())
    {
        eprintln!("cannot combine relation and state projection helpers in one proof run");
        std::process::exit(2);
    }
    if lhs_state_projection_top.is_some() != rhs_state_projection_top.is_some() {
        eprintln!("state projection proof requires both lhs and rhs state projection tops");
        std::process::exit(2);
    }

    let lhs_text = fs::read_to_string(&lhs_path).expect("failed to read lhs ir");
    let rhs_text = fs::read_to_string(&rhs_path).expect("failed to read rhs ir");
    let lhs_artifacts =
        parse_ir_text(&lhs_path, &lhs_text, Some(lhs_top)).expect("failed to parse lhs ir");
    let same_step = lhs_path == rhs_path && lhs_top == rhs_top;
    let rhs_artifacts = if same_step {
        None
    } else {
        Some(parse_ir_text(&rhs_path, &rhs_text, Some(rhs_top)).expect("failed to parse rhs ir"))
    };

    let needs_state_only_export = relation_top.is_some() || lhs_state_projection_top.is_some();
    let export_step = |func, node_name| {
        if needs_state_only_export {
            export_pir_fn_to_named_lustre_step(func, node_name)
        } else {
            export_pir_fn_to_named_lustre_full_step(func, node_name)
        }
    };
    let lhs = export_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
        .expect("failed to export lhs");
    let mut rhs = if same_step {
        renamed_step_export(&lhs, "step_impl_b", "step_impl_b_state_only")
    } else {
        export_step(rhs_artifacts.as_ref().unwrap().top_fn().unwrap(), "step_impl_b")
            .expect("failed to export rhs")
    };
    canonicalize_matching_input_aggregate_types(&lhs, &mut rhs);
    let lhs_assumption = lhs_assumption_top.as_deref().map(|assumption_top| {
        let assumption_artifacts = parse_ir_text(&lhs_path, &lhs_text, Some(assumption_top))
            .expect("failed to parse lhs assumption ir");
        export_pir_fn_to_named_lustre_assumption_with_aggregates(
            assumption_artifacts.top_fn().unwrap(),
            "lhs_assumption",
            lhs.input_aggregate.as_ref(),
            lhs.state_aggregate.as_ref(),
        )
        .expect("failed to export lhs assumption")
    });
    let rhs_assumption = rhs_assumption_top.as_deref().map(|assumption_top| {
        let assumption_artifacts = parse_ir_text(&rhs_path, &rhs_text, Some(assumption_top))
            .expect("failed to parse rhs assumption ir");
        export_pir_fn_to_named_lustre_assumption_with_aggregates(
            assumption_artifacts.top_fn().unwrap(),
            "rhs_assumption",
            rhs.input_aggregate.as_ref(),
            rhs.state_aggregate.as_ref(),
        )
        .expect("failed to export rhs assumption")
    });
    let lhs_observation = lhs_observation_top.as_deref().map(|observation_top| {
        let observation_path = lhs_observation_ir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| lhs_path.clone());
        let observation_text =
            fs::read_to_string(&observation_path).expect("failed to read lhs observation ir");
        let observation_artifacts =
            parse_ir_text(&observation_path, &observation_text, Some(observation_top))
                .expect("failed to parse lhs observation ir");
        export_pir_fn_to_named_lustre_observation_with_aggregates(
            observation_artifacts.top_fn().unwrap(),
            "lhs_observation",
            lhs.input_aggregate.as_ref(),
            lhs.state_aggregate.as_ref(),
            lhs.output_aggregate.as_ref(),
        )
        .expect("failed to export lhs observation")
    });
    let rhs_observation = rhs_observation_top.as_deref().map(|observation_top| {
        let observation_path = rhs_observation_ir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| rhs_path.clone());
        let observation_text =
            fs::read_to_string(&observation_path).expect("failed to read rhs observation ir");
        let observation_artifacts =
            parse_ir_text(&observation_path, &observation_text, Some(observation_top))
                .expect("failed to parse rhs observation ir");
        export_pir_fn_to_named_lustre_observation_with_aggregates(
            observation_artifacts.top_fn().unwrap(),
            "rhs_observation",
            rhs.input_aggregate.as_ref(),
            rhs.state_aggregate.as_ref(),
            rhs.output_aggregate.as_ref(),
        )
        .expect("failed to export rhs observation")
    });
    let needs_step_output = match (lhs_observation.as_ref(), rhs_observation.as_ref()) {
        (None, None) => true,
        (Some(lhs_observation), Some(rhs_observation)) => {
            matches!(
                lhs_observation.param_layout,
                ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
            ) || matches!(
                rhs_observation.param_layout,
                ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
            )
        }
        _ => true,
    };
    let lhs_state_projection = lhs_state_projection_top
        .as_deref()
        .map(|state_projection_top| {
            let state_projection_path = lhs_state_projection_ir
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| lhs_path.clone());
            let state_projection_text = fs::read_to_string(&state_projection_path)
                .expect("failed to read lhs state projection ir");
            let state_projection_artifacts = parse_ir_text(
                &state_projection_path,
                &state_projection_text,
                Some(state_projection_top),
            )
            .expect("failed to parse lhs state projection ir");
            export_pir_fn_to_named_lustre_state_projection_with_aggregates(
                state_projection_artifacts.top_fn().unwrap(),
                "lhs_state_projection",
                lhs.input_aggregate.as_ref(),
                lhs.state_aggregate.as_ref(),
            )
            .expect("failed to export lhs state projection")
        });
    let rhs_state_projection = rhs_state_projection_top
        .as_deref()
        .map(|state_projection_top| {
            let state_projection_path = rhs_state_projection_ir
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| rhs_path.clone());
            let state_projection_text = fs::read_to_string(&state_projection_path)
                .expect("failed to read rhs state projection ir");
            let state_projection_artifacts = parse_ir_text(
                &state_projection_path,
                &state_projection_text,
                Some(state_projection_top),
            )
            .expect("failed to parse rhs state projection ir");
            export_pir_fn_to_named_lustre_state_projection_with_aggregates(
                state_projection_artifacts.top_fn().unwrap(),
                "rhs_state_projection",
                rhs.input_aggregate.as_ref(),
                rhs.state_aggregate.as_ref(),
            )
            .expect("failed to export rhs state projection")
        });
    let needs_i_next = lhs_state_projection
        .as_ref()
        .and_then(|projection| projection.input_width)
        .is_some()
        || rhs_state_projection
            .as_ref()
            .and_then(|projection| projection.input_width)
            .is_some();
    let lhs_next_assumption = if !needs_step_output && needs_i_next {
        lhs_assumption_top.as_deref().map(|assumption_top| {
            let assumption_artifacts = parse_ir_text(&lhs_path, &lhs_text, Some(assumption_top))
                .expect("failed to parse lhs assumption ir");
            export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates(
                lhs_artifacts.top_fn().unwrap(),
                assumption_artifacts.top_fn().unwrap(),
                "lhs_next_assumption",
                lhs.input_aggregate.as_ref(),
                lhs.state_aggregate.as_ref(),
            )
            .expect("failed to export lhs next assumption")
        })
    } else {
        None
    };
    let rhs_next_assumption = if !needs_step_output && needs_i_next {
        rhs_assumption_top.as_deref().map(|assumption_top| {
            let assumption_artifacts = parse_ir_text(&rhs_path, &rhs_text, Some(assumption_top))
                .expect("failed to parse rhs assumption ir");
            export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates(
                if same_step {
                    lhs_artifacts.top_fn().unwrap()
                } else {
                    rhs_artifacts.as_ref().unwrap().top_fn().unwrap()
                },
                assumption_artifacts.top_fn().unwrap(),
                "rhs_next_assumption",
                rhs.input_aggregate.as_ref(),
                rhs.state_aggregate.as_ref(),
            )
            .expect("failed to export rhs next assumption")
        })
    } else {
        None
    };
    let lhs_next_state_projection = if !needs_step_output {
        lhs_state_projection_top
            .as_deref()
            .map(|state_projection_top| {
                let state_projection_path = lhs_state_projection_ir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| lhs_path.clone());
                let state_projection_text = fs::read_to_string(&state_projection_path)
                    .expect("failed to read lhs state projection ir");
                let state_projection_artifacts = parse_ir_text(
                    &state_projection_path,
                    &state_projection_text,
                    Some(state_projection_top),
                )
                .expect("failed to parse lhs state projection ir");
                export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
                    lhs_artifacts.top_fn().unwrap(),
                    state_projection_artifacts.top_fn().unwrap(),
                    "lhs_next_state_projection",
                    lhs.input_aggregate.as_ref(),
                    lhs.state_aggregate.as_ref(),
                )
                .expect("failed to export lhs next state projection")
            })
    } else {
        None
    };
    let rhs_next_state_projection = if !needs_step_output {
        rhs_state_projection_top
            .as_deref()
            .map(|state_projection_top| {
                let state_projection_path = rhs_state_projection_ir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| rhs_path.clone());
                let state_projection_text = fs::read_to_string(&state_projection_path)
                    .expect("failed to read rhs state projection ir");
                let state_projection_artifacts = parse_ir_text(
                    &state_projection_path,
                    &state_projection_text,
                    Some(state_projection_top),
                )
                .expect("failed to parse rhs state projection ir");
                export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
                    if same_step {
                        lhs_artifacts.top_fn().unwrap()
                    } else {
                        rhs_artifacts.as_ref().unwrap().top_fn().unwrap()
                    },
                    state_projection_artifacts.top_fn().unwrap(),
                    "rhs_next_state_projection",
                    rhs.input_aggregate.as_ref(),
                    rhs.state_aggregate.as_ref(),
                )
                .expect("failed to export rhs next state projection")
            })
    } else {
        None
    };
    let relation = relation_top.as_deref().map(|relation_top| {
        let relation_path = relation_ir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| lhs_path.clone());
        let relation_text = fs::read_to_string(&relation_path).expect("failed to read relation ir");
        let relation_artifacts = parse_ir_text(&relation_path, &relation_text, Some(relation_top))
            .expect("failed to parse relation ir");
        export_pir_fn_to_named_lustre_relation_with_aggregates(
            relation_artifacts.top_fn().unwrap(),
            "relation",
            lhs.input_aggregate.as_ref(),
            lhs.state_aggregate.as_ref(),
            rhs.state_aggregate.as_ref(),
        )
        .expect("failed to export relation")
    });

    let mut kind2_options = Kind2Options::default();
    if show_progress {
        kind2_options.verbosity = kind2_options.verbosity.max(2);
        kind2_options.live_output = true;
    }
    kind2_options.print_cex = true;
    kind2_options.extra_args = kind2_args;

    if lhs_state_projection.is_some() || rhs_state_projection.is_some() {
        let obligation_options = decomposed_relation_kind2_options(&kind2_options);
        let init_model = emit_state_projection_init_lustre(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            lhs_state_projection
                .as_ref()
                .expect("missing lhs state projection"),
            rhs_state_projection
                .as_ref()
                .expect("missing rhs state projection"),
        )
        .expect("failed to build state projection init model");
        maybe_dump_lustre(
            dump_lustre_dir.as_ref(),
            "state_projection_init.lus",
            &init_model,
        );
        let init_start = Instant::now();
        let init_outcome = check_kind2_model(
            &init_model.lustre_text,
            init_model.top_node.as_str(),
            &obligation_options,
        )
        .expect("failed to run Kind2 on state projection init");
        let init_wall = init_start.elapsed();
        report_outcome(
            "init state projection",
            &init_outcome,
            init_wall,
            show_progress,
        );
        if !matches!(init_outcome, Kind2CheckOutcome::Valid(_)) {
            std::process::exit(1);
        }

        let step_model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            None,
            lhs_observation.as_ref(),
            rhs_observation.as_ref(),
            lhs_state_projection
                .as_ref()
                .expect("missing lhs state projection"),
            rhs_state_projection
                .as_ref()
                .expect("missing rhs state projection"),
            lhs_next_assumption.as_ref(),
            rhs_next_assumption.as_ref(),
            lhs_next_state_projection.as_ref(),
            rhs_next_state_projection.as_ref(),
        )
        .expect("failed to build state projection step model");
        maybe_dump_lustre(
            dump_lustre_dir.as_ref(),
            "state_projection_step.lus",
            &step_model,
        );
        let step_start = Instant::now();
        let step_outcome = check_kind2_model(
            &step_model.lustre_text,
            step_model.top_node.as_str(),
            &obligation_options,
        )
        .expect("failed to run Kind2 on state projection step");
        let step_wall = step_start.elapsed();
        report_outcome("step obligation", &step_outcome, step_wall, show_progress);
        if !matches!(step_outcome, Kind2CheckOutcome::Valid(_)) {
            std::process::exit(1);
        }

        println!("lhs ir: {}", lhs_path.display());
        println!("lhs top: {lhs_top}");
        println!("rhs ir: {}", rhs_path.display());
        println!("rhs top: {rhs_top}");
        if let Some(lhs_assumption_top) = lhs_assumption_top.as_deref() {
            println!("lhs assumption top: {lhs_assumption_top}");
        }
        if let Some(rhs_assumption_top) = rhs_assumption_top.as_deref() {
            println!("rhs assumption top: {rhs_assumption_top}");
        }
        if let Some(lhs_observation_top) = lhs_observation_top.as_deref() {
            println!("lhs observation top: {lhs_observation_top}");
        }
        if let Some(rhs_observation_top) = rhs_observation_top.as_deref() {
            println!("rhs observation top: {rhs_observation_top}");
        }
        if let Some(lhs_state_projection_top) = lhs_state_projection_top.as_deref() {
            println!("lhs state projection top: {lhs_state_projection_top}");
        }
        if let Some(rhs_state_projection_top) = rhs_state_projection_top.as_deref() {
            println!("rhs state projection top: {rhs_state_projection_top}");
        }
        println!("proof strategy: decomposed state projection proof");
        return;
    }

    if decompose_relation {
        let relation = relation
            .as_ref()
            .expect("--decompose-relation requires --relation-top");
        let obligation_options = decomposed_relation_kind2_options(&kind2_options);

        let init_model = emit_relation_init_lustre(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            relation,
        )
        .expect("failed to build relation init model");
        maybe_dump_lustre(dump_lustre_dir.as_ref(), "relation_init.lus", &init_model);
        let init_start = Instant::now();
        let init_outcome = check_kind2_model(
            &init_model.lustre_text,
            init_model.top_node.as_str(),
            &obligation_options,
        )
        .expect("failed to run Kind2 on relation init");
        let init_wall = init_start.elapsed();
        report_outcome("init relation", &init_outcome, init_wall, show_progress);
        if !matches!(init_outcome, Kind2CheckOutcome::Valid(_)) {
            std::process::exit(1);
        }

        let step_model = if lhs_observation.is_some() || rhs_observation.is_some() {
            emit_relation_step_obligation_lustre_with_observations(
                &lhs,
                &rhs,
                lhs_assumption.as_ref(),
                rhs_assumption.as_ref(),
                relation,
                lhs_observation.as_ref(),
                rhs_observation.as_ref(),
            )
            .expect("failed to build relation step model")
        } else {
            emit_relation_step_obligation_lustre(
                &lhs,
                &rhs,
                lhs_assumption.as_ref(),
                rhs_assumption.as_ref(),
                relation,
            )
            .expect("failed to build relation step model")
        };
        maybe_dump_lustre(dump_lustre_dir.as_ref(), "relation_step.lus", &step_model);
        let step_start = Instant::now();
        let step_outcome = check_kind2_model(
            &step_model.lustre_text,
            step_model.top_node.as_str(),
            &obligation_options,
        )
        .expect("failed to run Kind2 on relation step");
        let step_wall = step_start.elapsed();
        report_outcome("step obligation", &step_outcome, step_wall, show_progress);
        if !matches!(step_outcome, Kind2CheckOutcome::Valid(_)) {
            std::process::exit(1);
        }

        println!("lhs ir: {}", lhs_path.display());
        println!("lhs top: {lhs_top}");
        println!("rhs ir: {}", rhs_path.display());
        println!("rhs top: {rhs_top}");
        if let Some(lhs_assumption_top) = lhs_assumption_top.as_deref() {
            println!("lhs assumption top: {lhs_assumption_top}");
        }
        if let Some(rhs_assumption_top) = rhs_assumption_top.as_deref() {
            println!("rhs assumption top: {rhs_assumption_top}");
        }
        if let Some(relation_top) = relation_top.as_deref() {
            println!("relation top: {relation_top}");
        }
        if let Some(lhs_observation_top) = lhs_observation_top.as_deref() {
            println!("lhs observation top: {lhs_observation_top}");
        }
        if let Some(rhs_observation_top) = rhs_observation_top.as_deref() {
            println!("rhs observation top: {rhs_observation_top}");
        }
        println!("proof strategy: decomposed relation proof");
        return;
    }

    if let Some(relation) = relation.as_ref() {
        let relation_model = emit_relation_invariant_lustre(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            relation,
        )
        .expect("failed to build relation invariant model");
        maybe_dump_lustre(
            dump_lustre_dir.as_ref(),
            "relation_invariant.lus",
            &relation_model,
        );
        let relation_start = Instant::now();
        let relation_outcome = check_kind2_model(
            &relation_model.lustre_text,
            relation_model.top_node.as_str(),
            &kind2_options,
        )
        .expect("failed to run Kind2 on relation");
        let relation_wall = relation_start.elapsed();
        match &relation_outcome {
            Kind2CheckOutcome::Valid(summary) => {
                println!("relation result: valid");
                println!("relation wall time: {:.3}s", relation_wall.as_secs_f64());
                if let Some(line) = extract_kind2_runtime_line(summary) {
                    println!("{line}");
                }
            }
            Kind2CheckOutcome::Invalid(summary) => {
                println!("relation result: invalid");
                println!("relation wall time: {:.3}s", relation_wall.as_secs_f64());
                if let Some(line) = extract_kind2_runtime_line(summary) {
                    println!("{line}");
                }
                if let Some(counterexample) = extract_kind2_counterexample(summary) {
                    println!("relation counterexample:");
                    println!("{counterexample}");
                }
                std::process::exit(1);
            }
            Kind2CheckOutcome::Unknown(summary) => {
                println!("relation result: unknown");
                println!("relation wall time: {:.3}s", relation_wall.as_secs_f64());
                if let Some(line) = extract_kind2_runtime_line(summary) {
                    println!("{line}");
                }
                if show_progress {
                    println!("relation kind2 progress:");
                    for line in extract_kind2_progress_lines(summary) {
                        println!("  {line}");
                    }
                }
                std::process::exit(1);
            }
        }
    }

    let model = if lhs_observation.is_some() || rhs_observation.is_some() {
        emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            relation.as_ref(),
            lhs_observation.as_ref(),
            rhs_observation.as_ref(),
        )
    } else if relation.is_some() {
        emit_observational_equivalence_lustre_with_assumptions_and_relation(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
            relation.as_ref(),
        )
    } else if lhs_assumption.is_some() || rhs_assumption.is_some() {
        emit_observational_equivalence_lustre_with_assumptions(
            &lhs,
            &rhs,
            lhs_init,
            rhs_init,
            lhs_assumption.as_ref(),
            rhs_assumption.as_ref(),
        )
    } else {
        emit_observational_equivalence_lustre(&lhs, &rhs, lhs_init, rhs_init)
    }
    .expect("failed to build observational equivalence model");
    maybe_dump_lustre(
        dump_lustre_dir.as_ref(),
        "observational_equivalence.lus",
        &model,
    );

    let kind2_start = Instant::now();
    let outcome = check_kind2_model(&model.lustre_text, model.top_node.as_str(), &kind2_options)
        .expect("failed to run Kind2");
    let kind2_wall = kind2_start.elapsed();

    println!("lhs ir: {}", lhs_path.display());
    println!("lhs top: {lhs_top}");
    println!("rhs ir: {}", rhs_path.display());
    println!("rhs top: {rhs_top}");
    if let Some(lhs_assumption_top) = lhs_assumption_top.as_deref() {
        println!("lhs assumption top: {lhs_assumption_top}");
    }
    if let Some(rhs_assumption_top) = rhs_assumption_top.as_deref() {
        println!("rhs assumption top: {rhs_assumption_top}");
    }
    if let Some(relation_top) = relation_top.as_deref() {
        println!("relation top: {relation_top}");
    }
    if let Some(lhs_observation_top) = lhs_observation_top.as_deref() {
        println!("lhs observation top: {lhs_observation_top}");
    }
    if let Some(rhs_observation_top) = rhs_observation_top.as_deref() {
        println!("rhs observation top: {rhs_observation_top}");
    }
    println!("lhs state width: {}", lhs.state_width);
    println!("rhs state width: {}", rhs.state_width);
    println!("lhs output width: {}", lhs.output_width);
    println!("rhs output width: {}", rhs.output_width);
    println!("proof property: {}", model.property_name);

    let summary = match &outcome {
        Kind2CheckOutcome::Valid(summary) => {
            println!("Kind2 result: valid");
            summary
        }
        Kind2CheckOutcome::Invalid(summary) => {
            println!("Kind2 result: invalid");
            summary
        }
        Kind2CheckOutcome::Unknown(summary) => {
            println!("Kind2 result: unknown");
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

#[cfg(test)]
mod tests {
    use super::{
        decomposed_relation_kind2_options, maybe_dump_lustre, proof_plan_main_kind2_options,
    };
    use tempfile::tempdir;
    use xlsynth_kind2::{Kind2Options, ObservationalEquivalenceModel};

    #[test]
    fn maybe_dump_lustre_writes_expected_file() {
        let tempdir = tempdir().unwrap();
        let model = ObservationalEquivalenceModel {
            lustre_text:
                "node main() returns (ok: bool);\nlet\n  ok = true;\n  --%PROPERTY ok;\ntel\n"
                    .to_string(),
            top_node: "main".to_string(),
            property_name: "ok".to_string(),
        };
        maybe_dump_lustre(Some(&tempdir.path().to_path_buf()), "sample.lus", &model);
        let written = std::fs::read_to_string(tempdir.path().join("sample.lus")).unwrap();
        assert_eq!(written, model.lustre_text);
    }

    #[test]
    fn decomposed_options_append_user_overrides_last() {
        let mut base = Kind2Options::default();
        base.extra_args = vec![
            "--enable".to_string(),
            "IC3IA".to_string(),
            "--disable".to_string(),
            "IND".to_string(),
        ];
        let options = decomposed_relation_kind2_options(&base);
        let suffix = options.extra_args[options.extra_args.len() - 4..].to_vec();
        assert_eq!(
            suffix,
            vec![
                "--enable".to_string(),
                "IC3IA".to_string(),
                "--disable".to_string(),
                "IND".to_string(),
            ]
        );
    }

    #[test]
    fn proof_plan_main_options_use_reduced_profile() {
        let mut base = Kind2Options::default();
        base.extra_args = vec!["--enable".to_string(), "IC3IA".to_string()];
        let options = proof_plan_main_kind2_options(&base);
        let joined = options.extra_args.join(" ");
        assert!(joined.contains("--disable IC3IA"));
        assert!(joined.contains("--disable INVGEN"));
        assert!(joined.contains("--check_subproperties false"));
        assert!(joined.ends_with("--enable IC3IA"));
    }
}

fn decomposed_relation_kind2_options(base: &Kind2Options) -> Kind2Options {
    let mut options = base.clone();
    let user_args = options.extra_args.clone();
    options.extra_args = vec![];
    options.extra_args.extend(
        [
            "--enable",
            "BMC",
            "--enable",
            "IND",
            "--disable",
            "IND2",
            "--disable",
            "IC3QE",
            "--disable",
            "IC3IA",
            "--disable",
            "INVGEN",
            "--disable",
            "INVGENOS",
            "--disable",
            "INVGENINT",
            "--disable",
            "INVGENINTOS",
            "--disable",
            "INVGENMACH",
            "--disable",
            "INVGENMACHOS",
            "--disable",
            "INVGENREAL",
            "--disable",
            "INVGENREALOS",
            "--check_subproperties",
            "false",
        ]
        .into_iter()
        .map(str::to_string),
    );
    options.extra_args.extend(user_args);
    options
}

fn report_outcome(
    label: &str,
    outcome: &Kind2CheckOutcome,
    wall: std::time::Duration,
    show_progress: bool,
) {
    match outcome {
        Kind2CheckOutcome::Valid(summary) => {
            println!("{label} result: valid");
            println!("{label} wall time: {:.3}s", wall.as_secs_f64());
            if let Some(line) = extract_kind2_runtime_line(summary) {
                println!("{line}");
            }
        }
        Kind2CheckOutcome::Invalid(summary) => {
            println!("{label} result: invalid");
            println!("{label} wall time: {:.3}s", wall.as_secs_f64());
            if let Some(line) = extract_kind2_runtime_line(summary) {
                println!("{line}");
            }
            if let Some(counterexample) = extract_kind2_counterexample(summary) {
                println!("{label} counterexample:");
                println!("{counterexample}");
            }
        }
        Kind2CheckOutcome::Unknown(summary) => {
            println!("{label} result: unknown");
            println!("{label} wall time: {:.3}s", wall.as_secs_f64());
            if let Some(line) = extract_kind2_runtime_line(summary) {
                println!("{line}");
            }
            if show_progress {
                println!("{label} kind2 progress:");
                for line in extract_kind2_progress_lines(summary) {
                    println!("  {line}");
                }
            }
        }
    }
}
