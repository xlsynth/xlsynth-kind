mod equiv;
mod kind2_smt;
mod lustre;
mod observational;
mod pipeline;
mod pir_lustre;
mod proof_plan;
mod transition;
mod xls_smt;

pub use crate::equiv::{
    EquivBundle, EquivCheck, EquivError, EquivOptions, EquivOutcome, build_step_equivalence_miter,
    prove_dslx_path_step_equivalence, prove_dslx_text_step_equivalence,
    prove_frontend_step_equivalence, prove_ir_text_step_equivalence,
};
pub use crate::kind2_smt::{
    Kind2CheckOutcome, Kind2Error, Kind2Options, Kind2Runner, check_kind2_model,
    emit_kind2_system_smt, extract_kind2_counterexample, extract_kind2_progress_lines,
    extract_kind2_runtime_line,
};
pub use crate::lustre::EmitError;
pub use crate::observational::{
    ObservationalEquivalenceError, ObservationalEquivalenceModel,
    emit_combined_assumption_lustre, emit_combined_relation_lustre,
    emit_invariant_init_lustre, emit_invariant_step_obligation_lustre,
    emit_observational_equivalence_lustre, emit_observational_equivalence_lustre_with_assumptions,
    emit_observational_equivalence_lustre_with_assumptions_and_relation,
    emit_observational_equivalence_lustre_with_assumptions_relation_and_observations,
    emit_observation_equivalence_relation_lustre,
    emit_relation_init_lustre, emit_relation_invariant_lustre,
    emit_relation_step_obligation_lustre, emit_relation_step_obligation_lustre_with_observations,
    emit_state_equality_relation_lustre,
    emit_state_projection_init_lustre, emit_state_projection_relation_lustre,
    emit_state_projection_step_obligation_lustre,
};
pub use crate::pipeline::{
    FrontendArtifacts, FrontendOptions, PipelineError, RecurrenceSignature, compile_dslx_path,
    compile_dslx_text, infer_recurrence_signature, parse_ir_text,
};
pub use crate::pir_lustre::{
    AggregateBitsSpec, AggregateFieldSpec, ExportedAssumptionLustre,
    ExportedNextStateAssumptionLustre, ExportedNextStateProjectionLustre,
    ExportedObservationLustre, ExportedRelationLustre, ExportedStateProjectionLustre,
    ExportedStepLustre, ObservationParamLayout, PirLustreError, StateProjectionParamLayout,
    export_pir_fn_to_lustre_assumption, export_pir_fn_to_lustre_observation,
    export_pir_fn_to_lustre_relation, export_pir_fn_to_lustre_state_projection,
    export_pir_fn_to_lustre_step, export_pir_fn_to_named_lustre_assumption,
    export_pir_fn_to_named_lustre_assumption_with_aggregates,
    export_pir_fn_to_named_lustre_assumption_with_state_aggregate,
    export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates,
    export_pir_fn_to_named_lustre_next_state_projection_with_aggregates,
    export_pir_fn_to_named_lustre_observation,
    export_pir_fn_to_named_lustre_observation_with_aggregates,
    export_pir_fn_to_named_lustre_observation_with_state_aggregate,
    export_pir_fn_to_named_lustre_relation, export_pir_fn_to_named_lustre_relation_with_aggregates,
    export_pir_fn_to_named_lustre_relation_with_state_aggregates,
    export_pir_fn_to_named_lustre_state_projection,
    export_pir_fn_to_named_lustre_state_projection_with_aggregates,
    export_pir_fn_to_named_lustre_state_projection_with_state_aggregate,
    export_pir_fn_to_named_lustre_full_step, export_pir_fn_to_named_lustre_step,
};
pub use crate::proof_plan::{
    AuxProofKind, AuxProofSpec, MainProofKind, MainProofSpec, ProofPlan, ProofPlanError,
    ordered_aux_specs, parse_proof_plan,
};
pub use crate::transition::{
    Expr, NextState, Output, Property, State, TransitionSystem, Type, VarDecl,
};
pub use crate::xls_smt::emit_ir_function_as_smtlib;
