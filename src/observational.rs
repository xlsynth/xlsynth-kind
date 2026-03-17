use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::pir_lustre::{
    AggregateBitsSpec, ExportedAssumptionLustre, ExportedNextStateAssumptionLustre,
    ExportedNextStateProjectionLustre, ExportedObservationLustre, ExportedRelationLustre,
    ExportedStateProjectionLustre, ExportedStepLustre, ObservationParamLayout,
    StateProjectionParamLayout, bits_use_native_chunks, native_bits_type_ref,
    native_lustre_type_ref, wide_bits_chunk_widths,
};
use xlsynth::{IrBits, IrValue, XlsynthError, ir_value::IrFormatPreference};
use xlsynth_pir::ir;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationalEquivalenceModel {
    pub lustre_text: String,
    pub top_node: String,
    pub property_name: String,
}

#[derive(Debug)]
pub enum ObservationalEquivalenceError {
    InputTypeMismatch {
        lhs_width: usize,
        rhs_width: usize,
    },
    OutputTypeMismatch {
        lhs_width: usize,
        rhs_width: usize,
    },
    InitOutOfRange {
        which: &'static str,
        value: u128,
        state_width: usize,
    },
    AssumptionInputTypeMismatch {
        which: &'static str,
        step_width: usize,
        assumption_width: usize,
    },
    AssumptionStateTypeMismatch {
        which: &'static str,
        step_width: usize,
        assumption_width: usize,
    },
    ObservationMissing {
        which: &'static str,
    },
    ObservationInputTypeMismatch {
        which: &'static str,
        step_width: usize,
        observation_width: usize,
    },
    ObservationStateTypeMismatch {
        which: &'static str,
        step_width: usize,
        observation_width: usize,
    },
    ObservationOutputTypeMismatch {
        which: &'static str,
        step_width: usize,
        observation_width: usize,
    },
    ObservationTypeMismatch {
        lhs_width: usize,
        rhs_width: usize,
    },
    ObservationRelationRequiresInputState {
        lhs_layout: ObservationParamLayout,
        rhs_layout: ObservationParamLayout,
    },
    StateProjectionMissing {
        which: &'static str,
    },
    StateProjectionInputTypeMismatch {
        which: &'static str,
        step_width: usize,
        projection_width: usize,
    },
    StateProjectionStateTypeMismatch {
        which: &'static str,
        step_width: usize,
        projection_width: usize,
    },
    StateProjectionTypeMismatch {
        lhs_width: usize,
        rhs_width: usize,
    },
    StateRelationRequiresMatchingLayouts {
        lhs_type: String,
        rhs_type: String,
    },
    RelationInputTypeMismatch {
        step_width: usize,
        relation_width: usize,
    },
    RelationLhsStateTypeMismatch {
        step_width: usize,
        relation_width: usize,
    },
    RelationRhsStateTypeMismatch {
        step_width: usize,
        relation_width: usize,
    },
    DecomposedRelationRequiresCurrentInput {
        relation_name: String,
    },
    Xlsynth(XlsynthError),
}

impl Display for ObservationalEquivalenceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTypeMismatch {
                lhs_width,
                rhs_width,
            } => write!(
                f,
                "observational equivalence requires matching input widths, got {lhs_width} and {rhs_width}"
            ),
            Self::OutputTypeMismatch {
                lhs_width,
                rhs_width,
            } => write!(
                f,
                "observational equivalence requires matching output widths, got {lhs_width} and {rhs_width}"
            ),
            Self::InitOutOfRange {
                which,
                value,
                state_width,
            } => write!(
                f,
                "{which} init value {value} does not fit in bits[{state_width}]"
            ),
            Self::AssumptionInputTypeMismatch {
                which,
                step_width,
                assumption_width,
            } => write!(
                f,
                "{which} assumption input width {assumption_width} did not match step input width {step_width}"
            ),
            Self::AssumptionStateTypeMismatch {
                which,
                step_width,
                assumption_width,
            } => write!(
                f,
                "{which} assumption state width {assumption_width} did not match step state width {step_width}"
            ),
            Self::ObservationMissing { which } => {
                write!(f, "{which} observation helper was missing")
            }
            Self::ObservationInputTypeMismatch {
                which,
                step_width,
                observation_width,
            } => write!(
                f,
                "{which} observation input width {observation_width} did not match step input width {step_width}"
            ),
            Self::ObservationStateTypeMismatch {
                which,
                step_width,
                observation_width,
            } => write!(
                f,
                "{which} observation state width {observation_width} did not match step state width {step_width}"
            ),
            Self::ObservationOutputTypeMismatch {
                which,
                step_width,
                observation_width,
            } => write!(
                f,
                "{which} observation output width {observation_width} did not match step output width {step_width}"
            ),
            Self::ObservationTypeMismatch {
                lhs_width,
                rhs_width,
            } => write!(
                f,
                "observation helpers must return matching widths, got {lhs_width} and {rhs_width}"
            ),
            Self::ObservationRelationRequiresInputState {
                lhs_layout,
                rhs_layout,
            } => write!(
                f,
                "observation-derived relation requires both helpers to use input/state layout, got {lhs_layout:?} and {rhs_layout:?}"
            ),
            Self::StateProjectionMissing { which } => {
                write!(f, "{which} state projection helper was missing")
            }
            Self::StateProjectionInputTypeMismatch {
                which,
                step_width,
                projection_width,
            } => write!(
                f,
                "{which} state projection input width {projection_width} did not match step input width {step_width}"
            ),
            Self::StateProjectionStateTypeMismatch {
                which,
                step_width,
                projection_width,
            } => write!(
                f,
                "{which} state projection state width {projection_width} did not match step state width {step_width}"
            ),
            Self::StateProjectionTypeMismatch {
                lhs_width,
                rhs_width,
            } => write!(
                f,
                "state projection helpers must return matching widths, got {lhs_width} and {rhs_width}"
            ),
            Self::StateRelationRequiresMatchingLayouts {
                lhs_type,
                rhs_type,
            } => write!(
                f,
                "direct state relation requires matching exported state layouts, got {lhs_type} and {rhs_type}"
            ),
            Self::RelationInputTypeMismatch {
                step_width,
                relation_width,
            } => write!(
                f,
                "relation input width {relation_width} did not match step input width {step_width}"
            ),
            Self::RelationLhsStateTypeMismatch {
                step_width,
                relation_width,
            } => write!(
                f,
                "relation lhs state width {relation_width} did not match lhs step state width {step_width}"
            ),
            Self::RelationRhsStateTypeMismatch {
                step_width,
                relation_width,
            } => write!(
                f,
                "relation rhs state width {relation_width} did not match rhs step state width {step_width}"
            ),
            Self::DecomposedRelationRequiresCurrentInput { relation_name } => write!(
                f,
                "decomposed relation proof does not support relation `{relation_name}` without a current input parameter when a previous-input parameter is present"
            ),
            Self::Xlsynth(err) => write!(f, "{err}"),
        }
    }
}

impl Error for ObservationalEquivalenceError {}

fn strip_type_defs_prefix(lustre_text: &str, type_defs: &[String]) -> String {
    if type_defs.is_empty() {
        return lustre_text.trim_end().to_string();
    }
    let prefix = format!("{}\n", type_defs.join("\n"));
    lustre_text
        .strip_prefix(&prefix)
        .unwrap_or(lustre_text)
        .trim_end()
        .to_string()
}

fn strip_nested_property_lines(lustre_text: &str) -> String {
    lustre_text
        .lines()
        .filter(|line| !line.trim_start().starts_with("--%PROPERTY "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn push_unique_type_defs(dest: &mut Vec<String>, defs: &[String]) {
    for def in defs {
        if !dest.iter().any(|existing| existing == def) {
            dest.push(def.clone());
        }
    }
}

fn push_supporting_export(
    dest: &mut Vec<String>,
    type_defs: &mut Vec<String>,
    text: &str,
    defs: &[String],
) {
    push_unique_type_defs(type_defs, defs);
    let stripped = strip_nested_property_lines(&strip_type_defs_prefix(text, defs));
    if !dest.iter().any(|existing| existing == &stripped) {
        dest.push(stripped);
    }
}

fn push_step_supporting_node(
    dest: &mut Vec<String>,
    type_defs: &mut Vec<String>,
    step: &ExportedStepLustre,
    needs_full_step: bool,
) {
    push_unique_type_defs(type_defs, &step.type_defs);
    let node_text = if needs_full_step {
        step.full_step_lustre_text.trim_end().to_string()
    } else {
        step.state_only_lustre_text.trim_end().to_string()
    };
    let node_text = strip_nested_property_lines(&node_text);
    if !dest.iter().any(|existing| existing == &node_text) {
        dest.push(node_text);
    }
}

fn push_step_type_defs(type_defs: &mut Vec<String>, step: &ExportedStepLustre) {
    push_unique_type_defs(type_defs, &step.type_defs);
}

fn state_type_name(step: &ExportedStepLustre) -> String {
    step.state_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| native_lustre_type_ref(&step.state_ty).expect("unsupported state type"))
}

fn input_type_name(step: &ExportedStepLustre) -> String {
    step.input_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| native_lustre_type_ref(&step.input_ty).expect("unsupported input type"))
}

fn output_type_name(step: &ExportedStepLustre) -> String {
    step.output_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| {
            native_lustre_type_ref(&step.output_ty).expect("unsupported output type")
        })
}

fn state_var_decl(var_name: &str, step: &ExportedStepLustre) -> String {
    format!("{var_name}: {};", state_type_name(step))
}

fn input_var_decl(var_name: &str, step: &ExportedStepLustre) -> String {
    format!("{var_name}: {};", input_type_name(step))
}

fn output_var_decl(var_name: &str, step: &ExportedStepLustre) -> String {
    format!("{var_name}: {};", output_type_name(step))
}

fn state_input_decl(var_name: &str, step: &ExportedStepLustre) -> String {
    format!("{var_name}: {}", state_type_name(step))
}

fn input_input_decl(var_name: &str, step: &ExportedStepLustre) -> String {
    format!("{var_name}: {}", input_type_name(step))
}

fn scalar_literal_expr(value: &str, width: usize) -> String {
    match width {
        2..=8 => format!("(uint8 {value})"),
        9..=16 => format!("(uint16 {value})"),
        17..=32 => format!("(uint32 {value})"),
        33..=64 => format!("(uint64 {value})"),
        _ => value.to_string(),
    }
}

fn scalar_mask_decimal(width: usize) -> Result<String, ObservationalEquivalenceError> {
    Ok(IrValue::from_bits(&IrBits::all_ones(width))
        .to_string_fmt(IrFormatPreference::UnsignedDecimal)?
        .trim_start_matches(&format!("bits[{width}]:"))
        .to_string())
}

fn scalar_range_expr(
    var_name: &str,
    width: usize,
) -> Result<String, ObservationalEquivalenceError> {
    match width {
        2..=7 | 9..=15 | 17..=31 | 33..=63 => Ok(format!(
            "({var_name} <= {})",
            scalar_literal_expr(&scalar_mask_decimal(width)?, width)
        )),
        8 | 16 | 32 | 64 => Ok("true".to_string()),
        _ => {
            let upper = power_of_two_decimal(width)?;
            Ok(format!("(0 <= {var_name}) and ({var_name} < {upper})"))
        }
    }
}

fn wide_bits_range_expr(
    var_name: &str,
    width: usize,
) -> Result<String, ObservationalEquivalenceError> {
    let mut terms = Vec::new();
    for (index, chunk_width) in wide_bits_chunk_widths(width).into_iter().enumerate() {
        let term = scalar_range_expr(&format!("{var_name}.f{index}"), chunk_width)?;
        if term != "true" {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        Ok("true".to_string())
    } else {
        Ok(terms.join(" and "))
    }
}

fn type_range_expr(
    var_name: &str,
    ty: &ir::Type,
) -> Result<String, ObservationalEquivalenceError> {
    match ty {
        ir::Type::Bits(width) => {
            if *width <= 1 {
                Ok("true".to_string())
            } else if bits_use_native_chunks(*width) {
                wide_bits_range_expr(var_name, *width)
            } else {
                scalar_range_expr(var_name, *width)
            }
        }
        ir::Type::Tuple(members) => {
            let mut terms = Vec::new();
            for (index, member) in members.iter().enumerate() {
                let term = type_range_expr(&format!("{var_name}.f{index}"), member)?;
                if term != "true" {
                    terms.push(term);
                }
            }
            if terms.is_empty() {
                Ok("true".to_string())
            } else {
                Ok(terms.join(" and "))
            }
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            let mut terms = Vec::new();
            for index in 0..*element_count {
                let term = type_range_expr(&format!("{var_name}[{index}]"), element_type)?;
                if term != "true" {
                    terms.push(term);
                }
            }
            if terms.is_empty() {
                Ok("true".to_string())
            } else {
                Ok(terms.join(" and "))
            }
        }
        _ => unreachable!("unsupported exported type"),
    }
}

fn state_range_expr(
    var_name: &str,
    step: &ExportedStepLustre,
) -> Result<String, ObservationalEquivalenceError> {
    if let Some(aggregate) = &step.state_aggregate {
        aggregate_range_expr(var_name, aggregate)
    } else {
        type_range_expr(var_name, &step.state_ty)
    }
}

fn input_range_expr(
    var_name: &str,
    step: &ExportedStepLustre,
) -> Result<String, ObservationalEquivalenceError> {
    if let Some(aggregate) = &step.input_aggregate {
        aggregate_range_expr(var_name, aggregate)
    } else {
        type_range_expr(var_name, &step.input_ty)
    }
}

fn aggregate_range_expr(
    var_name: &str,
    aggregate: &AggregateBitsSpec,
) -> Result<String, ObservationalEquivalenceError> {
    let mut terms = Vec::new();
    for field in &aggregate.fields {
        if field.width <= 1 {
            continue;
        }
        terms.push(scalar_range_expr(
            &format!("{var_name}.{}", field.name),
            field.width,
        )?);
    }
    if terms.is_empty() {
        Ok("true".to_string())
    } else {
        Ok(terms.join(" and "))
    }
}

fn state_init_expr(
    step: &ExportedStepLustre,
    value: u128,
) -> Result<String, ObservationalEquivalenceError> {
    if let Some(aggregate) = &step.state_aggregate {
        aggregate_init_expr(aggregate, value)
    } else {
        type_init_expr(&step.state_ty, value)
    }
}

fn aggregate_init_expr(
    aggregate: &AggregateBitsSpec,
    value: u128,
) -> Result<String, ObservationalEquivalenceError> {
    let mut parts = vec![String::new(); aggregate.fields.len()];
    let mut lsb_offset = 0usize;
    for (index, field) in aggregate.fields.iter().enumerate().rev() {
        let field_value = if field.width == 0 {
            0
        } else if lsb_offset >= 128 {
            0
        } else if field.width >= 128 - lsb_offset {
            value >> lsb_offset
        } else {
            let mask = (1u128 << field.width) - 1;
            (value >> lsb_offset) & mask
        };
        parts[index] = if field.width == 1 {
            if field_value == 0 {
                "false".to_string()
            } else {
                "true".to_string()
            }
        } else {
            scalar_literal_expr(&field_value.to_string(), field.width)
        };
        lsb_offset += field.width;
    }
    let assignments = aggregate
        .fields
        .iter()
        .zip(parts.iter())
        .map(|(field, value)| format!("{} = {}", field.name, value))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(format!("{} {{ {} }}", aggregate.type_name, assignments))
}

fn zero_value_expr(
    aggregate: Option<&AggregateBitsSpec>,
    ty: &ir::Type,
    width: usize,
) -> Result<String, ObservationalEquivalenceError> {
    if let Some(aggregate) = aggregate {
        aggregate_init_expr(aggregate, 0)
    } else {
        match ty {
            ir::Type::Bits(bit_count) if bits_use_native_chunks(*bit_count) => type_init_expr(ty, 0),
            ir::Type::Bits(_) => Ok(scalar_literal_expr("0", width)),
            ir::Type::Tuple(_) | ir::Type::Array(_) => type_init_expr(ty, 0),
            _ => unreachable!("unsupported exported type"),
        }
    }
}

fn emit_aggregate_pair_equality(
    lhs: &str,
    rhs: &str,
    lhs_aggregate: &AggregateBitsSpec,
    rhs_aggregate: &AggregateBitsSpec,
) -> String {
    let terms = lhs_aggregate
        .fields
        .iter()
        .zip(rhs_aggregate.fields.iter())
        .map(|(lhs_field, rhs_field)| {
            format!("({lhs}.{} = {rhs}.{})", lhs_field.name, rhs_field.name)
        })
        .collect::<Vec<_>>();
    if terms.is_empty() {
        "true".to_string()
    } else {
        format!("({})", terms.join(" and "))
    }
}

fn type_supports_native_equality(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::Bits(_) => true,
        ir::Type::Tuple(members) => members
            .iter()
            .all(|member| type_supports_native_equality(member)),
        ir::Type::Array(_) => false,
        _ => false,
    }
}

fn emit_type_equality(lhs: &str, rhs: &str, ty: &ir::Type) -> String {
    match ty {
        ir::Type::Bits(_) => format!("({lhs} = {rhs})"),
        ir::Type::Tuple(members) => {
            if type_supports_native_equality(ty) {
                return format!("({lhs} = {rhs})");
            }
            let terms = members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    emit_type_equality(&format!("{lhs}.f{index}"), &format!("{rhs}.f{index}"), member)
                })
                .collect::<Vec<_>>();
            if terms.is_empty() {
                "true".to_string()
            } else {
                format!("({})", terms.join(" and "))
            }
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            let terms = (0..*element_count)
                .map(|index| {
                    if type_supports_native_equality(element_type) {
                        format!("({lhs}[{index}] = {rhs}[{index}])")
                    } else {
                        emit_type_equality(
                            &format!("{lhs}[{index}]"),
                            &format!("{rhs}[{index}]"),
                            element_type,
                        )
                    }
                })
                .collect::<Vec<_>>();
            if terms.is_empty() {
                "true".to_string()
            } else {
                format!("({})", terms.join(" and "))
            }
        }
        _ => unreachable!("unsupported exported type"),
    }
}

fn emit_output_equality(
    lhs: &str,
    rhs: &str,
    lhs_step: &ExportedStepLustre,
    rhs_step: &ExportedStepLustre,
) -> Result<String, ObservationalEquivalenceError> {
    match (&lhs_step.output_aggregate, &rhs_step.output_aggregate) {
        (Some(lhs_aggregate), Some(rhs_aggregate))
            if lhs_aggregate
                .fields
                .iter()
                .map(|field| field.width)
                .eq(rhs_aggregate.fields.iter().map(|field| field.width)) =>
        {
            Ok(emit_aggregate_pair_equality(
                lhs,
                rhs,
                lhs_aggregate,
                rhs_aggregate,
            ))
        }
        (None, None)
            if lhs_step.output_ty == rhs_step.output_ty
                && !matches!(lhs_step.output_ty, ir::Type::Bits(_)) =>
        {
            Ok(emit_type_equality(lhs, rhs, &lhs_step.output_ty))
        }
        (lhs_aggregate, rhs_aggregate) => {
            let lhs_expr = if let Some(aggregate) = lhs_aggregate {
                aggregate_pack_var_expr(lhs, aggregate)?
            } else {
                lhs.to_string()
            };
            let rhs_expr = if let Some(aggregate) = rhs_aggregate {
                aggregate_pack_var_expr(rhs, aggregate)?
            } else {
                rhs.to_string()
            };
            emit_segmented_int_equality(
                &lhs_expr,
                &rhs_expr,
                lhs_step.output_segment_widths.as_deref(),
            )
        }
    }
}

fn collect_type_leaf_widths(ty: &ir::Type, widths: &mut Vec<usize>) {
    match ty {
        ir::Type::Bits(width) => {
            if bits_use_native_chunks(*width) {
                widths.extend(wide_bits_chunk_widths(*width));
            } else {
                widths.push(*width);
            }
        }
        ir::Type::Tuple(members) => {
            for member in members {
                collect_type_leaf_widths(member, widths);
            }
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            for _ in 0..*element_count {
                collect_type_leaf_widths(element_type, widths);
            }
        }
        _ => unreachable!("unsupported exported type"),
    }
}

fn type_init_expr(ty: &ir::Type, value: u128) -> Result<String, ObservationalEquivalenceError> {
    let mut widths = Vec::new();
    collect_type_leaf_widths(ty, &mut widths);
    let mut leaves = vec![String::new(); widths.len()];
    let mut lsb_offset = 0usize;
    for (index, width) in widths.iter().enumerate().rev() {
        let field_value = if *width == 0 {
            0
        } else if lsb_offset >= 128 {
            0
        } else if *width >= 128 - lsb_offset {
            value >> lsb_offset
        } else {
            let mask = (1u128 << *width) - 1;
            (value >> lsb_offset) & mask
        };
        leaves[index] = if *width == 1 {
            if field_value == 0 {
                "false".to_string()
            } else {
                "true".to_string()
            }
        } else {
            scalar_literal_expr(&field_value.to_string(), *width)
        };
        lsb_offset += *width;
    }
    let mut next_index = 0usize;
    Ok(type_init_expr_from_leaves(ty, &leaves, &mut next_index))
}

fn type_init_expr_from_leaves(ty: &ir::Type, leaves: &[String], next_index: &mut usize) -> String {
    match ty {
        ir::Type::Bits(width) => {
            if bits_use_native_chunks(*width) {
                let fields = wide_bits_chunk_widths(*width)
                    .into_iter()
                    .enumerate()
                    .map(|(index, _)| {
                        let expr = leaves[*next_index].clone();
                        *next_index += 1;
                        format!("f{index} = {expr}")
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!(
                    "{} {{ {} }}",
                    native_bits_type_ref(*width).expect("unsupported chunked bits type"),
                    fields
                )
            } else {
                let expr = leaves[*next_index].clone();
                *next_index += 1;
                expr
            }
        }
        ir::Type::Tuple(members) => {
            let assignments = members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    format!(
                        "f{index} = {}",
                        type_init_expr_from_leaves(member, leaves, next_index)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{} {{ {} }}",
                native_lustre_type_ref(ty).expect("unsupported tuple type"),
                assignments
            )
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            let elements = (0..*element_count)
                .map(|_| type_init_expr_from_leaves(element_type, leaves, next_index))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{elements}]")
        }
        _ => unreachable!("unsupported exported type"),
    }
}

fn state_projection_type_name(projection: &ExportedStateProjectionLustre) -> String {
    projection
        .abstraction_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| {
            native_bits_type_ref(projection.abstraction_width)
                .expect("unsupported state projection bits type")
        })
}

fn observation_type_name(observation: &ExportedObservationLustre) -> String {
    observation
        .observation_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| {
            native_bits_type_ref(observation.observation_width)
                .expect("unsupported observation bits type")
        })
}

fn observation_var_decl(var_name: &str, observation: &ExportedObservationLustre) -> String {
    format!("{var_name}: {};", observation_type_name(observation))
}

fn next_state_projection_type_name(projection: &ExportedNextStateProjectionLustre) -> String {
    projection
        .abstraction_aggregate
        .as_ref()
        .map(|aggregate| aggregate.type_name.clone())
        .unwrap_or_else(|| {
            native_bits_type_ref(projection.abstraction_width)
                .expect("unsupported next state projection bits type")
        })
}

fn state_projection_var_decl(var_name: &str, projection: &ExportedStateProjectionLustre) -> String {
    format!("{var_name}: {};", state_projection_type_name(projection))
}

fn next_state_projection_var_decl(
    var_name: &str,
    projection: &ExportedNextStateProjectionLustre,
) -> String {
    format!(
        "{var_name}: {};",
        next_state_projection_type_name(projection)
    )
}

fn aggregates_have_matching_widths(lhs: &AggregateBitsSpec, rhs: &AggregateBitsSpec) -> bool {
    lhs.fields
        .iter()
        .map(|field| field.width)
        .eq(rhs.fields.iter().map(|field| field.width))
}

fn emit_state_projection_equality(
    lhs: &str,
    rhs: &str,
    lhs_projection: &ExportedStateProjectionLustre,
    rhs_projection: &ExportedStateProjectionLustre,
) -> Result<String, ObservationalEquivalenceError> {
    match (
        lhs_projection.abstraction_aggregate.as_ref(),
        rhs_projection.abstraction_aggregate.as_ref(),
    ) {
        (Some(lhs_aggregate), Some(rhs_aggregate))
            if aggregates_have_matching_widths(lhs_aggregate, rhs_aggregate) =>
        {
            Ok(emit_aggregate_pair_equality(
                lhs,
                rhs,
                lhs_aggregate,
                rhs_aggregate,
            ))
        }
        (lhs_aggregate, rhs_aggregate) => {
            let lhs_expr = if let Some(aggregate) = lhs_aggregate {
                aggregate_pack_var_expr(lhs, aggregate)?
            } else {
                lhs.to_string()
            };
            let rhs_expr = if let Some(aggregate) = rhs_aggregate {
                aggregate_pack_var_expr(rhs, aggregate)?
            } else {
                rhs.to_string()
            };
            emit_segmented_int_equality(
                &lhs_expr,
                &rhs_expr,
                lhs_projection.abstraction_segment_widths.as_deref(),
            )
        }
    }
}

fn emit_observation_equality(
    lhs: &str,
    rhs: &str,
    lhs_observation: &ExportedObservationLustre,
    rhs_observation: &ExportedObservationLustre,
) -> Result<String, ObservationalEquivalenceError> {
    match (
        lhs_observation.observation_aggregate.as_ref(),
        rhs_observation.observation_aggregate.as_ref(),
    ) {
        (Some(lhs_aggregate), Some(rhs_aggregate))
            if aggregates_have_matching_widths(lhs_aggregate, rhs_aggregate) =>
        {
            Ok(emit_aggregate_pair_equality(
                lhs,
                rhs,
                lhs_aggregate,
                rhs_aggregate,
            ))
        }
        (lhs_aggregate, rhs_aggregate) => {
            let lhs_expr = if let Some(aggregate) = lhs_aggregate {
                aggregate_pack_var_expr(lhs, aggregate)?
            } else {
                lhs.to_string()
            };
            let rhs_expr = if let Some(aggregate) = rhs_aggregate {
                aggregate_pack_var_expr(rhs, aggregate)?
            } else {
                rhs.to_string()
            };
            emit_segmented_int_equality(
                &lhs_expr,
                &rhs_expr,
                lhs_observation.observation_segment_widths.as_deref(),
            )
        }
    }
}

fn aggregate_pack_var_expr(
    var_name: &str,
    aggregate: &AggregateBitsSpec,
) -> Result<String, ObservationalEquivalenceError> {
    let mut expr = None::<String>;
    for field in &aggregate.fields {
        let field_expr = if field.width == 1 {
            format!("(if {var_name}.{} then 1 else 0)", field.name)
        } else {
            format!("{var_name}.{}", field.name)
        };
        expr = Some(match expr {
            None => field_expr,
            Some(prev) => format!(
                "(({prev} * {}) + {field_expr})",
                power_of_two_decimal(field.width)?
            ),
        });
    }
    Ok(expr.unwrap_or_else(|| "0".to_string()))
}

pub fn emit_observational_equivalence_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
        lhs, rhs, lhs_init, rhs_init, None, None, None, None, None,
    )
}

pub fn emit_observational_equivalence_lustre_with_assumptions(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
        lhs,
        rhs,
        lhs_init,
        rhs_init,
        lhs_assumption,
        rhs_assumption,
        None,
        None,
        None,
    )
}

pub fn emit_observational_equivalence_lustre_with_assumptions_and_relation(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: Option<&ExportedRelationLustre>,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
        lhs,
        rhs,
        lhs_init,
        rhs_init,
        lhs_assumption,
        rhs_assumption,
        relation,
        None,
        None,
    )
}

pub fn emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: Option<&ExportedRelationLustre>,
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    if lhs.output_width != rhs.output_width {
        return Err(ObservationalEquivalenceError::OutputTypeMismatch {
            lhs_width: lhs.output_width,
            rhs_width: rhs.output_width,
        });
    }
    ensure_init_fits("lhs", lhs_init, lhs.state_width)?;
    ensure_init_fits("rhs", rhs_init, rhs.state_width)?;
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_observation_pair(lhs, rhs, lhs_observation, rhs_observation)?;
    validate_relation(lhs, rhs, relation)?;
    if can_use_reflexive_observational_fast_path(
        lhs,
        rhs,
        lhs_init,
        rhs_init,
        lhs_observation,
        rhs_observation,
    ) {
        return emit_reflexive_observational_equivalence_lustre(lhs, lhs_init);
    }

    let top_node = "main".to_string();
    let property_name = "outputs_match".to_string();
    let input_zero_expr = zero_value_expr(lhs.input_aggregate.as_ref(), &lhs.input_ty, lhs.input_width)?;
    let lhs_init_expr = state_init_expr(lhs, lhs_init)?;
    let rhs_init_expr = state_init_expr(rhs, rhs_init)?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, lhs, true);
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, rhs, true);
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }
    if let Some(relation) = relation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &relation.lustre_text,
            &relation.type_defs,
        );
    }
    if let Some(lhs_observation) = lhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_observation.lustre_text,
            &lhs_observation.type_defs,
        );
    }
    if let Some(rhs_observation) = rhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_observation.lustre_text,
            &rhs_observation.type_defs,
        );
    }

    let mut vars = vec![
        state_var_decl("s_a", lhs),
        state_var_decl("s_a_next", lhs),
        state_var_decl("s_b", rhs),
        state_var_decl("s_b_next", rhs),
        "lhs_keepalive: bool;".to_string(),
        "rhs_keepalive: bool;".to_string(),
        "input_in_range: bool;".to_string(),
        "input_history_in_range: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s_a = {lhs_init_expr} -> pre(s_a_next);"),
        format!("s_b = {rhs_init_expr} -> pre(s_b_next);"),
        format!(
            "(o_a, s_a_next, lhs_keepalive) = {}(i, s_a);",
            lhs.node_name
        ),
        format!(
            "(o_b, s_b_next, rhs_keepalive) = {}(i, s_b);",
            rhs.node_name
        ),
        format!("input_in_range = {};", input_range_expr("i", lhs)?),
        "input_history_in_range = input_in_range and (true -> pre(input_history_in_range));"
            .to_string(),
    ];
    let outputs_match_expr = if let (Some(lhs_observation), Some(rhs_observation)) =
        (lhs_observation, rhs_observation)
    {
        vars.push(observation_var_decl("obs_a", lhs_observation));
        vars.push(observation_var_decl("obs_b", rhs_observation));
        equations.push(emit_observation_equation(
            "obs_a",
            lhs_observation,
            "i",
            "s_a",
            "o_a",
        ));
        equations.push(emit_observation_equation(
            "obs_b",
            rhs_observation,
            "i",
            "s_b",
            "o_b",
        ));
        emit_observation_equality("obs_a", "obs_b", lhs_observation, rhs_observation)?
    } else {
        emit_output_equality("o_a", "o_b", lhs, rhs)?
    };

    let needs_prev_input = lhs_assumption.is_some()
        || rhs_assumption.is_some()
        || relation.and_then(|r| r.prev_input_name.as_ref()).is_some();
    if needs_prev_input {
        vars.push(input_var_decl("i_prev", lhs));
        equations.push(format!("i_prev = {input_zero_expr} -> pre(i);"));
    }

    let proof_domain = if lhs_assumption.is_some() || rhs_assumption.is_some() {
        vars.push("assumption_history: bool;".to_string());
        let lhs_assumption_now =
            emit_assumption_equation("lhs_assumption_now", lhs_assumption, "i", "s_a", "i_prev");
        let rhs_assumption_now =
            emit_assumption_equation("rhs_assumption_now", rhs_assumption, "i", "s_b", "i_prev");
        if lhs_assumption_now.is_some() {
            vars.push("lhs_assumption_now: bool;".to_string());
        }
        if rhs_assumption_now.is_some() {
            vars.push("rhs_assumption_now: bool;".to_string());
        }
        if let Some(lhs_assumption_now) = lhs_assumption_now {
            equations.push(lhs_assumption_now);
        }
        if let Some(rhs_assumption_now) = rhs_assumption_now {
            equations.push(rhs_assumption_now);
        }
        let assumption_now = match (lhs_assumption.is_some(), rhs_assumption.is_some()) {
            (true, true) => "lhs_assumption_now and rhs_assumption_now".to_string(),
            (true, false) => "lhs_assumption_now".to_string(),
            (false, true) => "rhs_assumption_now".to_string(),
            (false, false) => unreachable!(),
        };
        equations.push(format!(
            "assumption_history = ({assumption_now}) and (true -> pre(assumption_history));"
        ));
        "input_history_in_range and assumption_history".to_string()
    } else {
        "input_history_in_range".to_string()
    };
    let proof_domain = if relation.is_some() {
        vars.push("relation_now: bool;".to_string());
        equations.push(emit_relation_equation(
            "relation_now",
            relation.unwrap(),
            "i",
            "s_a",
            "s_b",
            "i_prev",
        ));
        format!("({proof_domain}) and relation_now")
    } else {
        proof_domain
    };
    equations.push(format!(
        "{property_name} = (not ({proof_domain})) or ({outputs_match_expr});"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}(i: {}) returns (o_a: {}; o_b: {}; {property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_type_name(lhs),
        output_type_name(lhs),
        output_type_name(rhs),
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

fn can_use_reflexive_observational_fast_path(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
) -> bool {
    lhs.node_name == rhs.node_name
        && lhs.lustre_text == rhs.lustre_text
        && lhs_init == rhs_init
        && lhs_observation.is_none()
        && rhs_observation.is_none()
}

fn emit_reflexive_observational_equivalence_lustre(
    step: &ExportedStepLustre,
    init: u128,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    ensure_init_fits("lhs", init, step.state_width)?;
    let top_node = "main".to_string();
    let property_name = "outputs_match".to_string();
    let init_expr = state_init_expr(step, init)?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, step);
    let mut supporting_nodes = Vec::new();
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, step, true);
    let vars = vec![
        state_var_decl("s", step),
        state_var_decl("s_next", step),
        "keepalive: bool;".to_string(),
        "input_in_range: bool;".to_string(),
        "input_history_in_range: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s = {init_expr} -> pre(s_next);"),
        format!("(o_a, s_next, keepalive) = {}(i, s);", step.node_name),
        "o_b = o_a;".to_string(),
        format!("input_in_range = {};", input_range_expr("i", step)?),
        "input_history_in_range = input_in_range and (true -> pre(input_history_in_range));"
            .to_string(),
    ];
    let outputs_match_expr = emit_output_equality("o_a", "o_b", step, step)?;
    equations.push(format!(
        "{property_name} = (not (input_history_in_range)) or ({outputs_match_expr});"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}(i: {}) returns (o_a: {}; o_b: {}; {property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_type_name(step),
        output_type_name(step),
        output_type_name(step),
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_relation_invariant_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: &ExportedRelationLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    ensure_init_fits("lhs", lhs_init, lhs.state_width)?;
    ensure_init_fits("rhs", rhs_init, rhs.state_width)?;
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_relation(lhs, rhs, Some(relation))?;

    let top_node = "main".to_string();
    let property_name = "relation_holds".to_string();
    let input_zero_expr = zero_value_expr(lhs.input_aggregate.as_ref(), &lhs.input_ty, lhs.input_width)?;
    let lhs_init_expr = state_init_expr(lhs, lhs_init)?;
    let rhs_init_expr = state_init_expr(rhs, rhs_init)?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, lhs, false);
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, rhs, false);
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &relation.lustre_text,
        &relation.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }

    let mut vars = vec![
        state_var_decl("s_a", lhs),
        state_var_decl("s_a_next", lhs),
        state_var_decl("s_b", rhs),
        state_var_decl("s_b_next", rhs),
        "input_in_range: bool;".to_string(),
        "input_history_in_range: bool;".to_string(),
        "relation_now: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s_a = {lhs_init_expr} -> pre(s_a_next);"),
        format!("s_b = {rhs_init_expr} -> pre(s_b_next);"),
        format!("s_a_next = {}(i, s_a);", lhs.state_only_node_name),
        format!("s_b_next = {}(i, s_b);", rhs.state_only_node_name),
        format!("input_in_range = {};", input_range_expr("i", lhs)?),
        "input_history_in_range = input_in_range and (true -> pre(input_history_in_range));"
            .to_string(),
    ];
    let needs_prev_input =
        lhs_assumption.is_some() || rhs_assumption.is_some() || relation.prev_input_name.is_some();
    if needs_prev_input {
        vars.push(input_var_decl("i_prev", lhs));
        equations.push(format!("i_prev = {input_zero_expr} -> pre(i);"));
    }
    let proof_domain = if lhs_assumption.is_some() || rhs_assumption.is_some() {
        vars.push("assumption_history: bool;".to_string());
        let lhs_assumption_now =
            emit_assumption_equation("lhs_assumption_now", lhs_assumption, "i", "s_a", "i_prev");
        let rhs_assumption_now =
            emit_assumption_equation("rhs_assumption_now", rhs_assumption, "i", "s_b", "i_prev");
        if lhs_assumption_now.is_some() {
            vars.push("lhs_assumption_now: bool;".to_string());
        }
        if rhs_assumption_now.is_some() {
            vars.push("rhs_assumption_now: bool;".to_string());
        }
        if let Some(lhs_assumption_now) = lhs_assumption_now {
            equations.push(lhs_assumption_now);
        }
        if let Some(rhs_assumption_now) = rhs_assumption_now {
            equations.push(rhs_assumption_now);
        }
        let assumption_now = match (lhs_assumption.is_some(), rhs_assumption.is_some()) {
            (true, true) => "lhs_assumption_now and rhs_assumption_now".to_string(),
            (true, false) => "lhs_assumption_now".to_string(),
            (false, true) => "rhs_assumption_now".to_string(),
            (false, false) => unreachable!(),
        };
        equations.push(format!(
            "assumption_history = ({assumption_now}) and (true -> pre(assumption_history));"
        ));
        "input_history_in_range and assumption_history".to_string()
    } else {
        "input_history_in_range".to_string()
    };
    equations.push(emit_relation_equation(
        "relation_now",
        relation,
        "i",
        "s_a",
        "s_b",
        "i_prev",
    ));
    equations.push(format!(
        "{property_name} = (not ({proof_domain})) or relation_now;"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}(i: {}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_type_name(lhs),
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_combined_assumption_lustre(
    step: &ExportedStepLustre,
    node_name: &str,
    assumptions: &[&ExportedAssumptionLustre],
) -> Result<Option<ExportedAssumptionLustre>, ObservationalEquivalenceError> {
    if assumptions.is_empty() {
        return Ok(None);
    }
    for assumption in assumptions {
        validate_assumption("combined", step, Some(assumption))?;
    }
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, step);
    let mut supporting_nodes = Vec::new();
    for assumption in assumptions {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &assumption.lustre_text,
            &assumption.type_defs,
        );
    }
    let needs_prev_input = assumptions
        .iter()
        .any(|assumption| assumption.prev_input_name.is_some());
    let mut vars = Vec::new();
    let mut terms = Vec::new();
    let mut equations = Vec::new();
    for (index, assumption) in assumptions.iter().enumerate() {
        let var_name = format!("a_{index}");
        vars.push(format!("{var_name}: bool;"));
        equations.push(
            emit_assumption_equation(&var_name, Some(assumption), "i", "s", "i_prev")
                .expect("combined assumption member is always present"),
        );
        terms.push(var_name);
    }
    let ok_expr = terms.join(" and ");
    equations.push(format!("ok = {ok_expr};"));
    let params = if needs_prev_input {
        format!(
            "{}; {}; {}",
            input_input_decl("i", step),
            state_input_decl("s", step),
            input_input_decl("i_prev", step),
        )
    } else {
        format!(
            "{}; {}",
            input_input_decl("i", step),
            state_input_decl("s", step),
        )
    };
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {node_name}({params}) returns (ok: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n"
    );
    Ok(Some(ExportedAssumptionLustre {
        lustre_text,
        type_defs,
        node_name: node_name.to_string(),
        input_name: "i".to_string(),
        state_name: "s".to_string(),
        prev_input_name: needs_prev_input.then_some("i_prev".to_string()),
        input_width: step.input_width,
        state_width: step.state_width,
    }))
}

pub fn emit_combined_relation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    node_name: &str,
    relations: &[&ExportedRelationLustre],
) -> Result<Option<ExportedRelationLustre>, ObservationalEquivalenceError> {
    if relations.is_empty() {
        return Ok(None);
    }
    for relation in relations {
        validate_relation(lhs, rhs, Some(relation))?;
    }
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    for relation in relations {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &relation.lustre_text,
            &relation.type_defs,
        );
    }
    let needs_input = relations.iter().any(|relation| relation.input_width.is_some());
    let needs_prev_input = relations
        .iter()
        .any(|relation| relation.prev_input_name.is_some());
    let mut vars = Vec::new();
    let mut terms = Vec::new();
    let mut equations = Vec::new();
    for (index, relation) in relations.iter().enumerate() {
        let var_name = format!("r_{index}");
        vars.push(format!("{var_name}: bool;"));
        equations.push(emit_relation_equation(
            &var_name,
            relation,
            "i",
            "s_a",
            "s_b",
            "i_prev",
        ));
        terms.push(var_name);
    }
    equations.push(format!("ok = {};", terms.join(" and ")));
    let mut params = Vec::new();
    if needs_input {
        params.push(input_input_decl("i", lhs));
    }
    params.push(state_input_decl("s_a", lhs));
    params.push(state_input_decl("s_b", rhs));
    if needs_prev_input {
        params.push(input_input_decl("i_prev", lhs));
    }
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {node_name}({}) returns (ok: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        params.join("; "),
    );
    Ok(Some(ExportedRelationLustre {
        lustre_text,
        type_defs,
        node_name: node_name.to_string(),
        input_name: needs_input.then_some("i".to_string()),
        lhs_state_name: "s_a".to_string(),
        rhs_state_name: "s_b".to_string(),
        prev_input_name: needs_prev_input.then_some("i_prev".to_string()),
        input_width: needs_input.then_some(lhs.input_width),
        lhs_state_width: lhs.state_width,
        rhs_state_width: rhs.state_width,
    }))
}

pub fn emit_observation_equivalence_relation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_observation: &ExportedObservationLustre,
    rhs_observation: &ExportedObservationLustre,
    node_name: &str,
) -> Result<ExportedRelationLustre, ObservationalEquivalenceError> {
    validate_observation_pair(lhs, rhs, Some(lhs_observation), Some(rhs_observation))?;
    if lhs_observation.param_layout != ObservationParamLayout::InputState
        || rhs_observation.param_layout != ObservationParamLayout::InputState
    {
        return Err(ObservationalEquivalenceError::ObservationRelationRequiresInputState {
            lhs_layout: lhs_observation.param_layout,
            rhs_layout: rhs_observation.param_layout,
        });
    }
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &lhs_observation.lustre_text,
        &lhs_observation.type_defs,
    );
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &rhs_observation.lustre_text,
        &rhs_observation.type_defs,
    );
    let vars = vec![
        observation_var_decl("obs_a", lhs_observation),
        observation_var_decl("obs_b", rhs_observation),
    ];
    let equations = vec![
        emit_observation_equation("obs_a", lhs_observation, "i", "s_a", "unused_output"),
        emit_observation_equation("obs_b", rhs_observation, "i", "s_b", "unused_output"),
        format!(
            "ok = {};",
            emit_observation_equality("obs_a", "obs_b", lhs_observation, rhs_observation)?
        ),
    ];
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {node_name}({}; {}; {}) returns (ok: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_input_decl("i", lhs),
        state_input_decl("s_a", lhs),
        state_input_decl("s_b", rhs),
    );
    Ok(ExportedRelationLustre {
        lustre_text,
        type_defs,
        node_name: node_name.to_string(),
        input_name: Some("i".to_string()),
        lhs_state_name: "s_a".to_string(),
        rhs_state_name: "s_b".to_string(),
        prev_input_name: None,
        input_width: Some(lhs.input_width),
        lhs_state_width: lhs.state_width,
        rhs_state_width: rhs.state_width,
    })
}

pub fn emit_state_projection_relation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_state_projection: &ExportedStateProjectionLustre,
    rhs_state_projection: &ExportedStateProjectionLustre,
    node_name: &str,
) -> Result<ExportedRelationLustre, ObservationalEquivalenceError> {
    validate_state_projection_pair(
        lhs,
        rhs,
        Some(lhs_state_projection),
        Some(rhs_state_projection),
    )?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &lhs_state_projection.lustre_text,
        &lhs_state_projection.type_defs,
    );
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &rhs_state_projection.lustre_text,
        &rhs_state_projection.type_defs,
    );
    let needs_input = lhs_state_projection.input_width.is_some()
        || rhs_state_projection.input_width.is_some();
    let needs_prev_input = lhs_state_projection.prev_input_name.is_some()
        || rhs_state_projection.prev_input_name.is_some();
    let vars = vec![
        state_projection_var_decl("abs_a", lhs_state_projection),
        state_projection_var_decl("abs_b", rhs_state_projection),
    ];
    let equations = vec![
        emit_state_projection_equation("abs_a", lhs_state_projection, "i", "s_a", "i_prev"),
        emit_state_projection_equation("abs_b", rhs_state_projection, "i", "s_b", "i_prev"),
        format!(
            "ok = {};",
            emit_state_projection_equality("abs_a", "abs_b", lhs_state_projection, rhs_state_projection)?
        ),
    ];
    let mut params = Vec::new();
    if needs_input {
        params.push(input_input_decl("i", lhs));
    }
    params.push(state_input_decl("s_a", lhs));
    params.push(state_input_decl("s_b", rhs));
    if needs_prev_input {
        params.push(input_input_decl("i_prev", lhs));
    }
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {node_name}({}) returns (ok: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        params.join("; "),
    );
    Ok(ExportedRelationLustre {
        lustre_text,
        type_defs,
        node_name: node_name.to_string(),
        input_name: needs_input.then_some("i".to_string()),
        lhs_state_name: "s_a".to_string(),
        rhs_state_name: "s_b".to_string(),
        prev_input_name: needs_prev_input.then_some("i_prev".to_string()),
        input_width: needs_input.then_some(lhs.input_width),
        lhs_state_width: lhs.state_width,
        rhs_state_width: rhs.state_width,
    })
}

pub fn emit_state_equality_relation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    node_name: &str,
) -> Result<ExportedRelationLustre, ObservationalEquivalenceError> {
    if state_type_name(lhs) != state_type_name(rhs) {
        return Err(ObservationalEquivalenceError::StateRelationRequiresMatchingLayouts {
            lhs_type: state_type_name(lhs),
            rhs_type: state_type_name(rhs),
        });
    }
    let equality = match (lhs.state_aggregate.as_ref(), rhs.state_aggregate.as_ref()) {
        (Some(lhs_aggregate), Some(rhs_aggregate))
            if aggregates_have_matching_widths(lhs_aggregate, rhs_aggregate) =>
        {
            emit_aggregate_pair_equality("s_a", "s_b", lhs_aggregate, rhs_aggregate)
        }
        (None, None) if lhs.state_ty == rhs.state_ty => emit_type_equality("s_a", "s_b", &lhs.state_ty),
        _ => {
            return Err(ObservationalEquivalenceError::StateRelationRequiresMatchingLayouts {
                lhs_type: state_type_name(lhs),
                rhs_type: state_type_name(rhs),
            });
        }
    };
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let supporting_nodes_text = type_defs.join("\n");
    let lustre_text = if supporting_nodes_text.is_empty() {
        format!(
            "node {node_name}({}; {}) returns (ok: bool);\n\
             let\n\
               ok = {equality};\n\
             tel\n",
            state_input_decl("s_a", lhs),
            state_input_decl("s_b", rhs),
        )
    } else {
        format!(
            "{supporting_nodes_text}\n\
             node {node_name}({}; {}) returns (ok: bool);\n\
             let\n\
               ok = {equality};\n\
             tel\n",
            state_input_decl("s_a", lhs),
            state_input_decl("s_b", rhs),
        )
    };
    Ok(ExportedRelationLustre {
        lustre_text,
        type_defs,
        node_name: node_name.to_string(),
        input_name: None,
        lhs_state_name: "s_a".to_string(),
        rhs_state_name: "s_b".to_string(),
        prev_input_name: None,
        input_width: None,
        lhs_state_width: lhs.state_width,
        rhs_state_width: rhs.state_width,
    })
}

pub fn emit_invariant_init_lustre(
    step: &ExportedStepLustre,
    init: u128,
    assumption: Option<&ExportedAssumptionLustre>,
    invariant: &ExportedAssumptionLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    ensure_init_fits("state", init, step.state_width)?;
    validate_assumption("env", step, assumption)?;
    validate_assumption("invariant", step, Some(invariant))?;

    let top_node = "main".to_string();
    let property_name = "invariant_holds_init".to_string();
    let state_init = state_init_expr(step, init)?;
    let input_zero_expr = zero_value_expr(
        step.input_aggregate.as_ref(),
        &step.input_ty,
        step.input_width,
    )?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, step);
    let mut supporting_nodes = Vec::new();
    if let Some(assumption) = assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &assumption.lustre_text,
            &assumption.type_defs,
        );
    }
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &invariant.lustre_text,
        &invariant.type_defs,
    );
    let needs_i_prev = assumption
        .and_then(|assumption| assumption.prev_input_name.as_ref())
        .is_some()
        || invariant.prev_input_name.is_some();
    let mut vars = vec![
        state_var_decl("s", step),
        "input_in_range: bool;".to_string(),
        "state_in_range: bool;".to_string(),
        "invariant_now: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s = {state_init};"),
        format!("input_in_range = {};", input_range_expr("i", step)?),
        format!("state_in_range = {};", state_range_expr("s", step)?),
    ];
    if needs_i_prev {
        vars.push(input_var_decl("i_prev", step));
        equations.push(format!("i_prev = {input_zero_expr};"));
    }
    if let Some(assumption_eq) =
        emit_assumption_equation("assumption_now", assumption, "i", "s", "i_prev")
    {
        vars.push("assumption_now: bool;".to_string());
        equations.push(assumption_eq);
    }
    equations.push(
        emit_assumption_equation("invariant_now", Some(invariant), "i", "s", "i_prev")
            .expect("invariant is always present"),
    );
    let proof_domain = match assumption.is_some() {
        true => "input_in_range and state_in_range and assumption_now",
        false => "input_in_range and state_in_range",
    };
    equations.push(format!(
        "{property_name} = (not ({proof_domain})) or invariant_now;"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}({}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_input_decl("i", step),
    );
    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_invariant_step_obligation_lustre(
    step: &ExportedStepLustre,
    assumption: Option<&ExportedAssumptionLustre>,
    invariant: &ExportedAssumptionLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    validate_assumption("env", step, assumption)?;
    validate_assumption("invariant", step, Some(invariant))?;
    let top_node = "main".to_string();
    let property_name = "step_preserves_invariant".to_string();
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, step);
    let mut supporting_nodes = Vec::new();
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, step, false);
    if let Some(assumption) = assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &assumption.lustre_text,
            &assumption.type_defs,
        );
    }
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &invariant.lustre_text,
        &invariant.type_defs,
    );
    let needs_i_prev = assumption
        .and_then(|assumption| assumption.prev_input_name.as_ref())
        .is_some()
        || invariant.prev_input_name.is_some();
    let mut vars = vec![
        state_var_decl("s_next", step),
        "input_in_range: bool;".to_string(),
        "state_in_range: bool;".to_string(),
        "next_input_in_range: bool;".to_string(),
        "next_state_in_range: bool;".to_string(),
        "invariant_now: bool;".to_string(),
        "invariant_next: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s_next = {}(i, s);", step.state_only_node_name),
        format!("input_in_range = {};", input_range_expr("i", step)?),
        format!("state_in_range = {};", state_range_expr("s", step)?),
        format!("next_input_in_range = {};", input_range_expr("i_next", step)?),
        format!("next_state_in_range = {};", state_range_expr("s_next", step)?),
        emit_assumption_equation("invariant_now", Some(invariant), "i", "s", "i_prev")
            .expect("invariant is always present"),
        emit_assumption_equation("invariant_next", Some(invariant), "i_next", "s_next", "i")
            .expect("invariant is always present"),
    ];
    if let Some(assumption_eq) =
        emit_assumption_equation("assumption_now", assumption, "i", "s", "i_prev")
    {
        vars.push("assumption_now: bool;".to_string());
        equations.push(assumption_eq);
    }
    if let Some(assumption_eq) =
        emit_assumption_equation("assumption_next", assumption, "i_next", "s_next", "i")
    {
        vars.push("assumption_next: bool;".to_string());
        equations.push(assumption_eq);
    }
    let current_domain = match assumption.is_some() {
        true => "input_in_range and state_in_range and assumption_now",
        false => "input_in_range and state_in_range",
    };
    let next_domain = match assumption.is_some() {
        true => "next_input_in_range and next_state_in_range and assumption_next",
        false => "next_input_in_range and next_state_in_range",
    };
    equations.push(format!(
        "{property_name} = (not (({current_domain}) and invariant_now)) or ((not ({next_domain})) or invariant_next);"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));
    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let mut params = vec![input_input_decl("i", step), state_input_decl("s", step)];
    if needs_i_prev {
        params.push(input_input_decl("i_prev", step));
    }
    params.push(input_input_decl("i_next", step));
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}({}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        params.join("; "),
    );
    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_relation_init_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: &ExportedRelationLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    ensure_init_fits("lhs", lhs_init, lhs.state_width)?;
    ensure_init_fits("rhs", rhs_init, rhs.state_width)?;
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_relation(lhs, rhs, Some(relation))?;
    ensure_relation_has_current_input_if_prev_input(relation)?;

    let top_node = "main".to_string();
    let property_name = "init_relation_holds".to_string();
    let input_zero_expr = zero_value_expr(lhs.input_aggregate.as_ref(), &lhs.input_ty, lhs.input_width)?;
    let lhs_init_expr = state_init_expr(lhs, lhs_init)?;
    let rhs_init_expr = state_init_expr(rhs, rhs_init)?;
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &relation.lustre_text,
        &relation.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }

    let mut vars = vec![
        "input_in_range: bool;".to_string(),
        "relation_now: bool;".to_string(),
    ];
    let mut equations = vec![format!("input_in_range = {};", input_range_expr("i", lhs)?)];

    let lhs_assumption_now = emit_assumption_equation_for_state(
        "lhs_assumption_now",
        lhs_assumption,
        "i",
        &lhs_init_expr,
        &input_zero_expr,
    );
    let rhs_assumption_now = emit_assumption_equation_for_state(
        "rhs_assumption_now",
        rhs_assumption,
        "i",
        &rhs_init_expr,
        &input_zero_expr,
    );
    if lhs_assumption_now.is_some() {
        vars.push("lhs_assumption_now: bool;".to_string());
    }
    if rhs_assumption_now.is_some() {
        vars.push("rhs_assumption_now: bool;".to_string());
    }
    if let Some(lhs_assumption_now) = lhs_assumption_now {
        equations.push(lhs_assumption_now);
    }
    if let Some(rhs_assumption_now) = rhs_assumption_now {
        equations.push(rhs_assumption_now);
    }
    equations.push(emit_relation_equation_for_state(
        "relation_now",
        relation,
        "i",
        &lhs_init_expr,
        &rhs_init_expr,
        &input_zero_expr,
    ));

    let proof_domain = init_proof_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
    equations.push(format!(
        "{property_name} = (not ({proof_domain})) or relation_now;"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}(i: {}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_type_name(lhs),
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_relation_step_obligation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: &ExportedRelationLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_relation(lhs, rhs, Some(relation))?;
    ensure_relation_has_current_input_if_prev_input(relation)?;

    let top_node = "main".to_string();
    let property_name = "step_preserves_relation".to_string();
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &relation.lustre_text,
        &relation.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, lhs, false);
    push_step_supporting_node(&mut supporting_nodes, &mut type_defs, rhs, false);

    let needs_i_prev = lhs_assumption
        .and_then(|assumption| assumption.prev_input_name.as_ref())
        .is_some()
        || rhs_assumption
            .and_then(|assumption| assumption.prev_input_name.as_ref())
            .is_some()
        || relation.prev_input_name.is_some();
    let needs_i_next = relation.input_name.is_some();

    let mut vars = vec![
        state_var_decl("s_a_next", lhs),
        state_var_decl("s_b_next", rhs),
        "input_in_range: bool;".to_string(),
        "lhs_state_in_range: bool;".to_string(),
        "rhs_state_in_range: bool;".to_string(),
        "relation_now: bool;".to_string(),
        "relation_next: bool;".to_string(),
    ];
    let mut equations = vec![
        format!("s_b_next = {}(i, s_b);", rhs.state_only_node_name),
        format!("s_a_next = {}(i, s_a);", lhs.state_only_node_name),
        format!("input_in_range = {};", input_range_expr("i", lhs)?),
        format!("lhs_state_in_range = {};", state_range_expr("s_a", lhs)?),
        format!("rhs_state_in_range = {};", state_range_expr("s_b", rhs)?),
    ];
    if needs_i_next {
        vars.push("next_input_in_range: bool;".to_string());
        equations.push(format!(
            "next_input_in_range = {};",
            input_range_expr("i_next", lhs)?
        ));
    }

    let lhs_assumption_now =
        emit_assumption_equation("lhs_assumption_now", lhs_assumption, "i", "s_a", "i_prev");
    let rhs_assumption_now =
        emit_assumption_equation("rhs_assumption_now", rhs_assumption, "i", "s_b", "i_prev");
    if lhs_assumption_now.is_some() {
        vars.push("lhs_assumption_now: bool;".to_string());
    }
    if rhs_assumption_now.is_some() {
        vars.push("rhs_assumption_now: bool;".to_string());
    }
    if let Some(lhs_assumption_now) = lhs_assumption_now {
        equations.push(lhs_assumption_now);
    }
    if let Some(rhs_assumption_now) = rhs_assumption_now {
        equations.push(rhs_assumption_now);
    }
    if needs_i_next {
        let lhs_assumption_next = emit_assumption_equation(
            "lhs_assumption_next",
            lhs_assumption,
            "i_next",
            "s_a_next",
            "i",
        );
        let rhs_assumption_next = emit_assumption_equation(
            "rhs_assumption_next",
            rhs_assumption,
            "i_next",
            "s_b_next",
            "i",
        );
        if lhs_assumption_next.is_some() {
            vars.push("lhs_assumption_next: bool;".to_string());
        }
        if rhs_assumption_next.is_some() {
            vars.push("rhs_assumption_next: bool;".to_string());
        }
        if let Some(lhs_assumption_next) = lhs_assumption_next {
            equations.push(lhs_assumption_next);
        }
        if let Some(rhs_assumption_next) = rhs_assumption_next {
            equations.push(rhs_assumption_next);
        }
    }

    equations.push(emit_relation_equation(
        "relation_now",
        relation,
        "i",
        "s_a",
        "s_b",
        "i_prev",
    ));
    equations.push(emit_relation_equation(
        "relation_next",
        relation,
        "i_next",
        "s_a_next",
        "s_b_next",
        "i",
    ));

    let current_domain =
        step_current_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
    let antecedent = format!("({current_domain}) and relation_now");
    let consequence = if needs_i_next {
        let next_domain = step_next_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
        format!("((not ({next_domain})) or relation_next)")
    } else {
        "relation_next".to_string()
    };
    equations.push(format!(
        "{property_name} = (not ({antecedent})) or {consequence};"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let mut params = vec![
        input_input_decl("i", lhs),
        state_input_decl("s_a", lhs),
        state_input_decl("s_b", rhs),
    ];
    if needs_i_prev {
        params.push(input_input_decl("i_prev", lhs));
    }
    if needs_i_next {
        params.push(input_input_decl("i_next", lhs));
    }
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}({}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        params.join("; "),
    );
    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_relation_step_obligation_lustre_with_observations(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    relation: &ExportedRelationLustre,
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    if lhs.output_width != rhs.output_width {
        return Err(ObservationalEquivalenceError::OutputTypeMismatch {
            lhs_width: lhs.output_width,
            rhs_width: rhs.output_width,
        });
    }
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_observation_pair(lhs, rhs, lhs_observation, rhs_observation)?;
    validate_relation(lhs, rhs, Some(relation))?;
    ensure_relation_has_current_input_if_prev_input(relation)?;

    let top_node = "main".to_string();
    let property_name = "step_preserves_relation_and_outputs".to_string();
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &relation.lustre_text,
        &relation.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }
    if let Some(lhs_observation) = lhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_observation.lustre_text,
            &lhs_observation.type_defs,
        );
    }
    if let Some(rhs_observation) = rhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_observation.lustre_text,
            &rhs_observation.type_defs,
        );
    }

    let needs_i_prev = lhs_assumption
        .and_then(|assumption| assumption.prev_input_name.as_ref())
        .is_some()
        || rhs_assumption
            .and_then(|assumption| assumption.prev_input_name.as_ref())
            .is_some()
        || relation.prev_input_name.is_some();
    let needs_i_next = relation.input_name.is_some();

    let needs_step_output = observations_require_step_output(lhs_observation, rhs_observation);
    push_step_supporting_node(
        &mut supporting_nodes,
        &mut type_defs,
        lhs,
        needs_step_output,
    );
    push_step_supporting_node(
        &mut supporting_nodes,
        &mut type_defs,
        rhs,
        needs_step_output,
    );
    let mut vars = vec![
        state_var_decl("s_a_next", lhs),
        state_var_decl("s_b_next", rhs),
        "input_in_range: bool;".to_string(),
        "lhs_state_in_range: bool;".to_string(),
        "rhs_state_in_range: bool;".to_string(),
        "relation_now: bool;".to_string(),
        "relation_next: bool;".to_string(),
    ];
    if needs_step_output {
        vars.push(output_var_decl("o_a", lhs));
        vars.push(output_var_decl("o_b", rhs));
        vars.push("lhs_keepalive: bool;".to_string());
        vars.push("rhs_keepalive: bool;".to_string());
    }
    let mut equations = vec![
        format!("input_in_range = {};", input_range_expr("i", lhs)?),
        format!("lhs_state_in_range = {};", state_range_expr("s_a", lhs)?),
        format!("rhs_state_in_range = {};", state_range_expr("s_b", rhs)?),
    ];
    if needs_step_output {
        equations.insert(
            0,
            format!(
                "(o_b, s_b_next, rhs_keepalive) = {}(i, s_b);",
                rhs.node_name
            ),
        );
        equations.insert(
            0,
            format!(
                "(o_a, s_a_next, lhs_keepalive) = {}(i, s_a);",
                lhs.node_name
            ),
        );
    } else {
        equations.insert(
            0,
            format!("s_b_next = {}(i, s_b);", rhs.state_only_node_name),
        );
        equations.insert(
            0,
            format!("s_a_next = {}(i, s_a);", lhs.state_only_node_name),
        );
    }
    let outputs_match_expr = if let (Some(lhs_observation), Some(rhs_observation)) =
        (lhs_observation, rhs_observation)
    {
        vars.push(observation_var_decl("obs_a", lhs_observation));
        vars.push(observation_var_decl("obs_b", rhs_observation));
        equations.push(emit_observation_equation(
            "obs_a",
            lhs_observation,
            "i",
            "s_a",
            "o_a",
        ));
        equations.push(emit_observation_equation(
            "obs_b",
            rhs_observation,
            "i",
            "s_b",
            "o_b",
        ));
        emit_observation_equality("obs_a", "obs_b", lhs_observation, rhs_observation)?
    } else {
        emit_output_equality("o_a", "o_b", lhs, rhs)?
    };
    if needs_i_next {
        vars.push("next_input_in_range: bool;".to_string());
        equations.push(format!(
            "next_input_in_range = {};",
            input_range_expr("i_next", lhs)?
        ));
    }

    let lhs_assumption_now =
        emit_assumption_equation("lhs_assumption_now", lhs_assumption, "i", "s_a", "i_prev");
    let rhs_assumption_now =
        emit_assumption_equation("rhs_assumption_now", rhs_assumption, "i", "s_b", "i_prev");
    if lhs_assumption_now.is_some() {
        vars.push("lhs_assumption_now: bool;".to_string());
    }
    if rhs_assumption_now.is_some() {
        vars.push("rhs_assumption_now: bool;".to_string());
    }
    if let Some(lhs_assumption_now) = lhs_assumption_now {
        equations.push(lhs_assumption_now);
    }
    if let Some(rhs_assumption_now) = rhs_assumption_now {
        equations.push(rhs_assumption_now);
    }
    if needs_i_next {
        let lhs_assumption_next = emit_assumption_equation(
            "lhs_assumption_next",
            lhs_assumption,
            "i_next",
            "s_a_next",
            "i",
        );
        let rhs_assumption_next = emit_assumption_equation(
            "rhs_assumption_next",
            rhs_assumption,
            "i_next",
            "s_b_next",
            "i",
        );
        if lhs_assumption_next.is_some() {
            vars.push("lhs_assumption_next: bool;".to_string());
        }
        if rhs_assumption_next.is_some() {
            vars.push("rhs_assumption_next: bool;".to_string());
        }
        if let Some(lhs_assumption_next) = lhs_assumption_next {
            equations.push(lhs_assumption_next);
        }
        if let Some(rhs_assumption_next) = rhs_assumption_next {
            equations.push(rhs_assumption_next);
        }
    }

    equations.push(emit_relation_equation(
        "relation_now",
        relation,
        "i",
        "s_a",
        "s_b",
        "i_prev",
    ));
    equations.push(emit_relation_equation(
        "relation_next",
        relation,
        "i_next",
        "s_a_next",
        "s_b_next",
        "i",
    ));

    let current_domain =
        step_current_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
    let antecedent = format!("({current_domain}) and relation_now");
    let consequence = if needs_i_next {
        let next_domain = step_next_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
        format!("(({outputs_match_expr}) and ((not ({next_domain})) or relation_next))")
    } else {
        format!("(({outputs_match_expr}) and relation_next)")
    };
    equations.push(format!(
        "{property_name} = (not ({antecedent})) or {consequence};"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let input_decl = step_obligation_input_decl(lhs, rhs, needs_i_prev, needs_i_next);
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}({input_decl}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_state_projection_init_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_init: u128,
    rhs_init: u128,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    lhs_state_projection: &ExportedStateProjectionLustre,
    rhs_state_projection: &ExportedStateProjectionLustre,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    ensure_init_fits("lhs", lhs_init, lhs.state_width)?;
    ensure_init_fits("rhs", rhs_init, rhs.state_width)?;
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_state_projection_pair(
        lhs,
        rhs,
        Some(lhs_state_projection),
        Some(rhs_state_projection),
    )?;

    let top_node = "main".to_string();
    let property_name = "init_state_projection_holds".to_string();
    let input_zero_expr = zero_value_expr(lhs.input_aggregate.as_ref(), &lhs.input_ty, lhs.input_width)?;
    let lhs_init_expr = state_init_expr(lhs, lhs_init)?;
    let rhs_init_expr = state_init_expr(rhs, rhs_init)?;
    let abstraction_equal_expr = emit_state_projection_equality(
        "abs_a",
        "abs_b",
        lhs_state_projection,
        rhs_state_projection,
    )?;

    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &lhs_state_projection.lustre_text,
        &lhs_state_projection.type_defs,
    );
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &rhs_state_projection.lustre_text,
        &rhs_state_projection.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }

    let mut vars = vec![
        "input_in_range: bool;".to_string(),
        state_projection_var_decl("abs_a", lhs_state_projection),
        state_projection_var_decl("abs_b", rhs_state_projection),
    ];
    let mut equations = vec![format!("input_in_range = {};", input_range_expr("i", lhs)?)];

    let lhs_assumption_now = emit_assumption_equation_for_state(
        "lhs_assumption_now",
        lhs_assumption,
        "i",
        &lhs_init_expr,
        &input_zero_expr,
    );
    let rhs_assumption_now = emit_assumption_equation_for_state(
        "rhs_assumption_now",
        rhs_assumption,
        "i",
        &rhs_init_expr,
        &input_zero_expr,
    );
    if lhs_assumption_now.is_some() {
        vars.push("lhs_assumption_now: bool;".to_string());
    }
    if rhs_assumption_now.is_some() {
        vars.push("rhs_assumption_now: bool;".to_string());
    }
    if let Some(lhs_assumption_now) = lhs_assumption_now {
        equations.push(lhs_assumption_now);
    }
    if let Some(rhs_assumption_now) = rhs_assumption_now {
        equations.push(rhs_assumption_now);
    }

    equations.push(emit_state_projection_equation(
        "abs_a",
        lhs_state_projection,
        "i",
        &lhs_init_expr,
        &input_zero_expr,
    ));
    equations.push(emit_state_projection_equation(
        "abs_b",
        rhs_state_projection,
        "i",
        &rhs_init_expr,
        &input_zero_expr,
    ));

    let proof_domain = init_proof_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
    equations.push(format!(
        "{property_name} = (not ({proof_domain})) or ({abstraction_equal_expr});"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}(i: {}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
        input_type_name(lhs),
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

pub fn emit_state_projection_step_obligation_lustre(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_assumption: Option<&ExportedAssumptionLustre>,
    rhs_assumption: Option<&ExportedAssumptionLustre>,
    current_relation: Option<&ExportedRelationLustre>,
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
    lhs_state_projection: &ExportedStateProjectionLustre,
    rhs_state_projection: &ExportedStateProjectionLustre,
    lhs_next_assumption: Option<&ExportedNextStateAssumptionLustre>,
    rhs_next_assumption: Option<&ExportedNextStateAssumptionLustre>,
    lhs_next_state_projection: Option<&ExportedNextStateProjectionLustre>,
    rhs_next_state_projection: Option<&ExportedNextStateProjectionLustre>,
) -> Result<ObservationalEquivalenceModel, ObservationalEquivalenceError> {
    if lhs.input_width != rhs.input_width {
        return Err(ObservationalEquivalenceError::InputTypeMismatch {
            lhs_width: lhs.input_width,
            rhs_width: rhs.input_width,
        });
    }
    if lhs.output_width != rhs.output_width {
        return Err(ObservationalEquivalenceError::OutputTypeMismatch {
            lhs_width: lhs.output_width,
            rhs_width: rhs.output_width,
        });
    }
    validate_assumption("lhs", lhs, lhs_assumption)?;
    validate_assumption("rhs", rhs, rhs_assumption)?;
    validate_observation_pair(lhs, rhs, lhs_observation, rhs_observation)?;
    validate_state_projection_pair(
        lhs,
        rhs,
        Some(lhs_state_projection),
        Some(rhs_state_projection),
    )?;
    validate_relation(lhs, rhs, current_relation)?;

    let top_node = "main".to_string();
    let property_name = "step_preserves_state_projection_and_outputs".to_string();
    let mut type_defs = Vec::new();
    push_step_type_defs(&mut type_defs, lhs);
    push_step_type_defs(&mut type_defs, rhs);
    let mut supporting_nodes = Vec::new();
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &lhs_state_projection.lustre_text,
        &lhs_state_projection.type_defs,
    );
    push_supporting_export(
        &mut supporting_nodes,
        &mut type_defs,
        &rhs_state_projection.lustre_text,
        &rhs_state_projection.type_defs,
    );
    if let Some(lhs_assumption) = lhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_assumption.lustre_text,
            &lhs_assumption.type_defs,
        );
    }
    if let Some(rhs_assumption) = rhs_assumption {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_assumption.lustre_text,
            &rhs_assumption.type_defs,
        );
    }
    if let Some(lhs_observation) = lhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_observation.lustre_text,
            &lhs_observation.type_defs,
        );
    }
    if let Some(rhs_observation) = rhs_observation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_observation.lustre_text,
            &rhs_observation.type_defs,
        );
    }
    if let Some(current_relation) = current_relation {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &current_relation.lustre_text,
            &current_relation.type_defs,
        );
    }

    let needs_i_prev = lhs_assumption
        .and_then(|assumption| assumption.prev_input_name.as_ref())
        .is_some()
        || rhs_assumption
            .and_then(|assumption| assumption.prev_input_name.as_ref())
            .is_some()
        || lhs_state_projection.prev_input_name.is_some()
        || rhs_state_projection.prev_input_name.is_some();
    let needs_i_next =
        lhs_state_projection.input_width.is_some() || rhs_state_projection.input_width.is_some();

    let needs_step_output = observations_require_step_output(lhs_observation, rhs_observation);
    let can_slice_next_state = !needs_step_output
        && lhs_next_state_projection.is_some()
        && rhs_next_state_projection.is_some()
        && (!needs_i_next
            || (!lhs_assumption.is_some() || lhs_next_assumption.is_some())
                && (!rhs_assumption.is_some() || rhs_next_assumption.is_some()));
    if can_slice_next_state {
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &lhs_next_state_projection
                .expect("checked above")
                .lustre_text,
            &lhs_next_state_projection.expect("checked above").type_defs,
        );
        push_supporting_export(
            &mut supporting_nodes,
            &mut type_defs,
            &rhs_next_state_projection
                .expect("checked above")
                .lustre_text,
            &rhs_next_state_projection.expect("checked above").type_defs,
        );
        if needs_i_next {
            if let Some(lhs_next_assumption) = lhs_next_assumption {
                push_supporting_export(
                    &mut supporting_nodes,
                    &mut type_defs,
                    &lhs_next_assumption.lustre_text,
                    &lhs_next_assumption.type_defs,
                );
            }
            if let Some(rhs_next_assumption) = rhs_next_assumption {
                push_supporting_export(
                    &mut supporting_nodes,
                    &mut type_defs,
                    &rhs_next_assumption.lustre_text,
                    &rhs_next_assumption.type_defs,
                );
            }
        }
    } else {
        push_step_supporting_node(
            &mut supporting_nodes,
            &mut type_defs,
            lhs,
            needs_step_output,
        );
        push_step_supporting_node(
            &mut supporting_nodes,
            &mut type_defs,
            rhs,
            needs_step_output,
        );
    }
    let mut vars = vec![
        "input_in_range: bool;".to_string(),
        "lhs_state_in_range: bool;".to_string(),
        "rhs_state_in_range: bool;".to_string(),
        state_projection_var_decl("abs_a_now", lhs_state_projection),
        state_projection_var_decl("abs_b_now", rhs_state_projection),
    ];
    if can_slice_next_state {
        vars.push(next_state_projection_var_decl(
            "abs_a_next",
            lhs_next_state_projection.expect("checked above"),
        ));
        vars.push(next_state_projection_var_decl(
            "abs_b_next",
            rhs_next_state_projection.expect("checked above"),
        ));
    } else {
        vars.push(state_projection_var_decl(
            "abs_a_next",
            lhs_state_projection,
        ));
        vars.push(state_projection_var_decl(
            "abs_b_next",
            rhs_state_projection,
        ));
    }
    if !can_slice_next_state {
        vars.insert(0, state_var_decl("s_b_next", rhs));
        vars.insert(0, state_var_decl("s_a_next", lhs));
    }
    if !can_slice_next_state && needs_step_output {
        vars.push(output_var_decl("o_a", lhs));
        vars.push(output_var_decl("o_b", rhs));
        vars.push("lhs_keepalive: bool;".to_string());
        vars.push("rhs_keepalive: bool;".to_string());
    }
    let mut equations = vec![
        format!("input_in_range = {};", input_range_expr("i", lhs)?),
        format!("lhs_state_in_range = {};", state_range_expr("s_a", lhs)?),
        format!("rhs_state_in_range = {};", state_range_expr("s_b", rhs)?),
    ];
    if !can_slice_next_state {
        if needs_step_output {
            equations.insert(
                0,
                format!(
                    "(o_b, s_b_next, rhs_keepalive) = {}(i, s_b);",
                    rhs.node_name
                ),
            );
            equations.insert(
                0,
                format!(
                    "(o_a, s_a_next, lhs_keepalive) = {}(i, s_a);",
                    lhs.node_name
                ),
            );
        } else {
            equations.insert(
                0,
                format!("s_b_next = {}(i, s_b);", rhs.state_only_node_name),
            );
            equations.insert(
                0,
                format!("s_a_next = {}(i, s_a);", lhs.state_only_node_name),
            );
        }
    }

    let outputs_match_expr = if let (Some(lhs_observation), Some(rhs_observation)) =
        (lhs_observation, rhs_observation)
    {
        vars.push(observation_var_decl("obs_a", lhs_observation));
        vars.push(observation_var_decl("obs_b", rhs_observation));
        equations.push(emit_observation_equation(
            "obs_a",
            lhs_observation,
            "i",
            "s_a",
            "o_a",
        ));
        equations.push(emit_observation_equation(
            "obs_b",
            rhs_observation,
            "i",
            "s_b",
            "o_b",
        ));
        emit_observation_equality("obs_a", "obs_b", lhs_observation, rhs_observation)?
    } else {
        emit_output_equality("o_a", "o_b", lhs, rhs)?
    };

    if needs_i_next {
        vars.push("next_input_in_range: bool;".to_string());
        equations.push(format!(
            "next_input_in_range = {};",
            input_range_expr("i_next", lhs)?
        ));
    }
    if current_relation.is_some() {
        vars.push("relation_now: bool;".to_string());
        equations.push(emit_relation_equation(
            "relation_now",
            current_relation.expect("checked above"),
            "i",
            "s_a",
            "s_b",
            "i_prev",
        ));
    }

    let lhs_assumption_now =
        emit_assumption_equation("lhs_assumption_now", lhs_assumption, "i", "s_a", "i_prev");
    let rhs_assumption_now =
        emit_assumption_equation("rhs_assumption_now", rhs_assumption, "i", "s_b", "i_prev");
    if lhs_assumption_now.is_some() {
        vars.push("lhs_assumption_now: bool;".to_string());
    }
    if rhs_assumption_now.is_some() {
        vars.push("rhs_assumption_now: bool;".to_string());
    }
    if let Some(lhs_assumption_now) = lhs_assumption_now {
        equations.push(lhs_assumption_now);
    }
    if let Some(rhs_assumption_now) = rhs_assumption_now {
        equations.push(rhs_assumption_now);
    }

    if needs_i_next {
        if can_slice_next_state {
            if let Some(lhs_next_assumption) = lhs_next_assumption {
                vars.push("lhs_assumption_next: bool;".to_string());
                equations.push(emit_next_state_assumption_equation(
                    "lhs_assumption_next",
                    lhs_next_assumption,
                    "i",
                    "s_a",
                    "i_next",
                ));
            }
            if let Some(rhs_next_assumption) = rhs_next_assumption {
                vars.push("rhs_assumption_next: bool;".to_string());
                equations.push(emit_next_state_assumption_equation(
                    "rhs_assumption_next",
                    rhs_next_assumption,
                    "i",
                    "s_b",
                    "i_next",
                ));
            }
        } else {
            let lhs_assumption_next = emit_assumption_equation(
                "lhs_assumption_next",
                lhs_assumption,
                "i_next",
                "s_a_next",
                "i",
            );
            let rhs_assumption_next = emit_assumption_equation(
                "rhs_assumption_next",
                rhs_assumption,
                "i_next",
                "s_b_next",
                "i",
            );
            if lhs_assumption_next.is_some() {
                vars.push("lhs_assumption_next: bool;".to_string());
            }
            if rhs_assumption_next.is_some() {
                vars.push("rhs_assumption_next: bool;".to_string());
            }
            if let Some(lhs_assumption_next) = lhs_assumption_next {
                equations.push(lhs_assumption_next);
            }
            if let Some(rhs_assumption_next) = rhs_assumption_next {
                equations.push(rhs_assumption_next);
            }
        }
    }

    equations.push(emit_state_projection_equation(
        "abs_a_now",
        lhs_state_projection,
        "i",
        "s_a",
        "i_prev",
    ));
    equations.push(emit_state_projection_equation(
        "abs_b_now",
        rhs_state_projection,
        "i",
        "s_b",
        "i_prev",
    ));
    if can_slice_next_state {
        equations.push(emit_next_state_projection_equation(
            "abs_a_next",
            lhs_next_state_projection.expect("checked above"),
            "i",
            "s_a",
            "i_next",
        ));
        equations.push(emit_next_state_projection_equation(
            "abs_b_next",
            rhs_next_state_projection.expect("checked above"),
            "i",
            "s_b",
            "i_next",
        ));
    } else {
        equations.push(emit_state_projection_equation(
            "abs_a_next",
            lhs_state_projection,
            "i_next",
            "s_a_next",
            "i",
        ));
        equations.push(emit_state_projection_equation(
            "abs_b_next",
            rhs_state_projection,
            "i_next",
            "s_b_next",
            "i",
        ));
    }

    let abstraction_now_expr = emit_state_projection_equality(
        "abs_a_now",
        "abs_b_now",
        lhs_state_projection,
        rhs_state_projection,
    )?;
    let abstraction_next_expr = if can_slice_next_state {
        let lhs_next_state_projection = lhs_next_state_projection.expect("checked above");
        let rhs_next_state_projection = rhs_next_state_projection.expect("checked above");
        match (
            lhs_next_state_projection.abstraction_aggregate.as_ref(),
            rhs_next_state_projection.abstraction_aggregate.as_ref(),
        ) {
            (Some(lhs_aggregate), Some(rhs_aggregate))
                if aggregates_have_matching_widths(lhs_aggregate, rhs_aggregate) =>
            {
                emit_aggregate_pair_equality(
                    "abs_a_next",
                    "abs_b_next",
                    lhs_aggregate,
                    rhs_aggregate,
                )
            }
            (lhs_aggregate, rhs_aggregate) => {
                let lhs_expr = if let Some(aggregate) = lhs_aggregate {
                    aggregate_pack_var_expr("abs_a_next", aggregate)?
                } else {
                    "abs_a_next".to_string()
                };
                let rhs_expr = if let Some(aggregate) = rhs_aggregate {
                    aggregate_pack_var_expr("abs_b_next", aggregate)?
                } else {
                    "abs_b_next".to_string()
                };
                emit_segmented_int_equality(
                    &lhs_expr,
                    &rhs_expr,
                    lhs_next_state_projection
                        .abstraction_segment_widths
                        .as_deref(),
                )?
            }
        }
    } else {
        emit_state_projection_equality(
            "abs_a_next",
            "abs_b_next",
            lhs_state_projection,
            rhs_state_projection,
        )?
    };

    let current_domain =
        step_current_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
    let antecedent = if current_relation.is_some() {
        format!("({current_domain}) and relation_now and ({abstraction_now_expr})")
    } else {
        format!("({current_domain}) and ({abstraction_now_expr})")
    };
    let consequence = if needs_i_next {
        let next_domain = step_next_domain_expr(lhs_assumption.is_some(), rhs_assumption.is_some());
        format!("(({outputs_match_expr}) and ((not ({next_domain})) or ({abstraction_next_expr})))")
    } else {
        format!("(({outputs_match_expr}) and ({abstraction_next_expr}))")
    };
    equations.push(format!(
        "{property_name} = (not ({antecedent})) or {consequence};"
    ));
    equations.push(format!("--%PROPERTY {property_name};"));

    let supporting_nodes_text = if type_defs.is_empty() {
        supporting_nodes.join("\n")
    } else {
        format!("{}\n{}", type_defs.join("\n"), supporting_nodes.join("\n"))
    };
    let input_decl = step_obligation_input_decl(lhs, rhs, needs_i_prev, needs_i_next);
    let vars_text = vars
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let equations_text = equations
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    let lustre_text = format!(
        "{supporting_nodes_text}\n\
         node {top_node}({input_decl}) returns ({property_name}: bool);\n\
         var\n\
         {vars_text}\
         let\n\
         {equations_text}\
         tel\n",
    );

    Ok(ObservationalEquivalenceModel {
        lustre_text,
        top_node,
        property_name,
    })
}

impl From<XlsynthError> for ObservationalEquivalenceError {
    fn from(value: XlsynthError) -> Self {
        Self::Xlsynth(value)
    }
}

fn validate_assumption(
    which: &'static str,
    step: &ExportedStepLustre,
    assumption: Option<&ExportedAssumptionLustre>,
) -> Result<(), ObservationalEquivalenceError> {
    let Some(assumption) = assumption else {
        return Ok(());
    };
    if assumption.input_width != step.input_width {
        return Err(ObservationalEquivalenceError::AssumptionInputTypeMismatch {
            which,
            step_width: step.input_width,
            assumption_width: assumption.input_width,
        });
    }
    if assumption.state_width != step.state_width {
        return Err(ObservationalEquivalenceError::AssumptionStateTypeMismatch {
            which,
            step_width: step.state_width,
            assumption_width: assumption.state_width,
        });
    }
    Ok(())
}

fn validate_observation(
    which: &'static str,
    step: &ExportedStepLustre,
    observation: &ExportedObservationLustre,
) -> Result<(), ObservationalEquivalenceError> {
    if let Some(observation_input_width) = observation.input_width {
        if observation_input_width != step.input_width {
            return Err(
                ObservationalEquivalenceError::ObservationInputTypeMismatch {
                    which,
                    step_width: step.input_width,
                    observation_width: observation_input_width,
                },
            );
        }
    }
    if let Some(observation_state_width) = observation.state_width {
        if observation_state_width != step.state_width {
            return Err(
                ObservationalEquivalenceError::ObservationStateTypeMismatch {
                    which,
                    step_width: step.state_width,
                    observation_width: observation_state_width,
                },
            );
        }
    }
    if let Some(observation_output_width) = observation.output_param_width {
        if observation_output_width != step.output_width {
            return Err(
                ObservationalEquivalenceError::ObservationOutputTypeMismatch {
                    which,
                    step_width: step.output_width,
                    observation_width: observation_output_width,
                },
            );
        }
    }
    Ok(())
}

fn validate_observation_pair(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
) -> Result<(), ObservationalEquivalenceError> {
    match (lhs_observation, rhs_observation) {
        (None, None) => Ok(()),
        (Some(_), None) => Err(ObservationalEquivalenceError::ObservationMissing { which: "rhs" }),
        (None, Some(_)) => Err(ObservationalEquivalenceError::ObservationMissing { which: "lhs" }),
        (Some(lhs_observation), Some(rhs_observation)) => {
            validate_observation("lhs", lhs, lhs_observation)?;
            validate_observation("rhs", rhs, rhs_observation)?;
            if lhs_observation.observation_width != rhs_observation.observation_width {
                return Err(ObservationalEquivalenceError::ObservationTypeMismatch {
                    lhs_width: lhs_observation.observation_width,
                    rhs_width: rhs_observation.observation_width,
                });
            }
            Ok(())
        }
    }
}

fn validate_state_projection(
    which: &'static str,
    step: &ExportedStepLustre,
    state_projection: &ExportedStateProjectionLustre,
) -> Result<(), ObservationalEquivalenceError> {
    if let Some(projection_input_width) = state_projection.input_width {
        if projection_input_width != step.input_width {
            return Err(
                ObservationalEquivalenceError::StateProjectionInputTypeMismatch {
                    which,
                    step_width: step.input_width,
                    projection_width: projection_input_width,
                },
            );
        }
    }
    if state_projection.state_width != step.state_width {
        return Err(
            ObservationalEquivalenceError::StateProjectionStateTypeMismatch {
                which,
                step_width: step.state_width,
                projection_width: state_projection.state_width,
            },
        );
    }
    Ok(())
}

fn validate_state_projection_pair(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    lhs_state_projection: Option<&ExportedStateProjectionLustre>,
    rhs_state_projection: Option<&ExportedStateProjectionLustre>,
) -> Result<(), ObservationalEquivalenceError> {
    match (lhs_state_projection, rhs_state_projection) {
        (None, None) => Ok(()),
        (Some(_), None) => {
            Err(ObservationalEquivalenceError::StateProjectionMissing { which: "rhs" })
        }
        (None, Some(_)) => {
            Err(ObservationalEquivalenceError::StateProjectionMissing { which: "lhs" })
        }
        (Some(lhs_state_projection), Some(rhs_state_projection)) => {
            validate_state_projection("lhs", lhs, lhs_state_projection)?;
            validate_state_projection("rhs", rhs, rhs_state_projection)?;
            if lhs_state_projection.abstraction_width != rhs_state_projection.abstraction_width {
                return Err(ObservationalEquivalenceError::StateProjectionTypeMismatch {
                    lhs_width: lhs_state_projection.abstraction_width,
                    rhs_width: rhs_state_projection.abstraction_width,
                });
            }
            Ok(())
        }
    }
}

fn observations_require_step_output(
    lhs_observation: Option<&ExportedObservationLustre>,
    rhs_observation: Option<&ExportedObservationLustre>,
) -> bool {
    let (Some(lhs_observation), Some(rhs_observation)) = (lhs_observation, rhs_observation) else {
        return true;
    };
    matches!(
        lhs_observation.param_layout,
        ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
    ) || matches!(
        rhs_observation.param_layout,
        ObservationParamLayout::OutputOnly | ObservationParamLayout::InputStateOutput
    )
}

fn emit_segmented_int_equality(
    lhs: &str,
    rhs: &str,
    widths: Option<&[usize]>,
) -> Result<String, ObservationalEquivalenceError> {
    let Some(widths) = widths else {
        return Ok(format!("({lhs} = {rhs})"));
    };
    if widths.len() <= 1 {
        return Ok(format!("({lhs} = {rhs})"));
    }

    let mut start = 0usize;
    let mut terms = Vec::with_capacity(widths.len());
    for width in widths.iter().rev() {
        let lhs_slice = emit_int_bit_slice_expr(lhs, start, *width)?;
        let rhs_slice = emit_int_bit_slice_expr(rhs, start, *width)?;
        terms.push(format!("({lhs_slice} = {rhs_slice})"));
        start += *width;
    }
    Ok(format!("({})", terms.join(" and ")))
}

fn emit_int_bit_slice_expr(
    expr: &str,
    start: usize,
    width: usize,
) -> Result<String, ObservationalEquivalenceError> {
    if width == 0 {
        return Ok("0".to_string());
    }
    let modulus = power_of_two_decimal(width)?;
    if start == 0 {
        return Ok(format!("(({expr}) mod {modulus})"));
    }
    let start_divisor = power_of_two_decimal(start)?;
    Ok(format!("((({expr} / {start_divisor}) mod {modulus}))"))
}

fn validate_relation(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    relation: Option<&ExportedRelationLustre>,
) -> Result<(), ObservationalEquivalenceError> {
    let Some(relation) = relation else {
        return Ok(());
    };
    if let Some(relation_input_width) = relation.input_width {
        if relation_input_width != lhs.input_width {
            return Err(ObservationalEquivalenceError::RelationInputTypeMismatch {
                step_width: lhs.input_width,
                relation_width: relation_input_width,
            });
        }
    }
    if relation.lhs_state_width != lhs.state_width {
        return Err(
            ObservationalEquivalenceError::RelationLhsStateTypeMismatch {
                step_width: lhs.state_width,
                relation_width: relation.lhs_state_width,
            },
        );
    }
    if relation.rhs_state_width != rhs.state_width {
        return Err(
            ObservationalEquivalenceError::RelationRhsStateTypeMismatch {
                step_width: rhs.state_width,
                relation_width: relation.rhs_state_width,
            },
        );
    }
    Ok(())
}

fn ensure_relation_has_current_input_if_prev_input(
    relation: &ExportedRelationLustre,
) -> Result<(), ObservationalEquivalenceError> {
    if relation.prev_input_name.is_some() && relation.input_name.is_none() {
        return Err(
            ObservationalEquivalenceError::DecomposedRelationRequiresCurrentInput {
                relation_name: relation.node_name.clone(),
            },
        );
    }
    Ok(())
}

fn ensure_init_fits(
    which: &'static str,
    value: u128,
    state_width: usize,
) -> Result<(), ObservationalEquivalenceError> {
    if state_width == 0 {
        if value == 0 {
            return Ok(());
        }
        return Err(ObservationalEquivalenceError::InitOutOfRange {
            which,
            value,
            state_width,
        });
    }
    if state_width < 128 && value >= (1u128 << state_width) {
        return Err(ObservationalEquivalenceError::InitOutOfRange {
            which,
            value,
            state_width,
        });
    }
    Ok(())
}

fn power_of_two_decimal(width: usize) -> Result<String, ObservationalEquivalenceError> {
    let mut bits = vec![false; width + 1];
    bits[width] = true;
    let value = IrValue::from_bits(&IrBits::from_lsb_is_0(&bits));
    Ok(value
        .to_string_fmt(IrFormatPreference::UnsignedDecimal)?
        .trim_start_matches(&format!("bits[{}]:", width + 1))
        .to_string())
}

fn emit_assumption_equation(
    equation_name: &str,
    assumption: Option<&ExportedAssumptionLustre>,
    input_var: &str,
    state_var: &str,
    prev_input_var: &str,
) -> Option<String> {
    assumption.map(|assumption| {
        let call = if assumption.prev_input_name.is_some() {
            format!(
                "{}({}, {}, {})",
                assumption.node_name, input_var, state_var, prev_input_var
            )
        } else {
            format!("{}({}, {})", assumption.node_name, input_var, state_var)
        };
        format!("{equation_name} = {call};")
    })
}

fn emit_observation_equation(
    equation_name: &str,
    observation: &ExportedObservationLustre,
    input_var: &str,
    state_var: &str,
    output_var: &str,
) -> String {
    let call = match observation.param_layout {
        ObservationParamLayout::OutputOnly => {
            format!("{}({output_var})", observation.node_name)
        }
        ObservationParamLayout::InputState => {
            format!("{}({input_var}, {state_var})", observation.node_name)
        }
        ObservationParamLayout::InputStateOutput => format!(
            "{}({input_var}, {state_var}, {output_var})",
            observation.node_name
        ),
    };
    format!("{equation_name} = {call};")
}

fn emit_state_projection_equation(
    equation_name: &str,
    state_projection: &ExportedStateProjectionLustre,
    input_var: &str,
    state_var: &str,
    prev_input_var: &str,
) -> String {
    let call = match state_projection.param_layout {
        StateProjectionParamLayout::StateOnly => {
            format!("{}({state_var})", state_projection.node_name)
        }
        StateProjectionParamLayout::InputState => {
            format!("{}({input_var}, {state_var})", state_projection.node_name)
        }
        StateProjectionParamLayout::InputStatePrevInput => format!(
            "{}({input_var}, {state_var}, {prev_input_var})",
            state_projection.node_name
        ),
    };
    format!("{equation_name} = {call};")
}

fn emit_next_state_assumption_equation(
    equation_name: &str,
    assumption: &ExportedNextStateAssumptionLustre,
    input_var: &str,
    state_var: &str,
    next_input_var: &str,
) -> String {
    format!(
        "{equation_name} = {}({input_var}, {state_var}, {next_input_var});",
        assumption.node_name
    )
}

fn emit_next_state_projection_equation(
    equation_name: &str,
    state_projection: &ExportedNextStateProjectionLustre,
    input_var: &str,
    state_var: &str,
    next_input_var: &str,
) -> String {
    let call = if state_projection.next_input_name.is_some() {
        format!(
            "{}({input_var}, {state_var}, {next_input_var})",
            state_projection.node_name
        )
    } else {
        format!("{}({input_var}, {state_var})", state_projection.node_name)
    };
    format!("{equation_name} = {call};")
}

fn emit_assumption_equation_for_state(
    equation_name: &str,
    assumption: Option<&ExportedAssumptionLustre>,
    input_var: &str,
    state_expr: &str,
    prev_input_expr: &str,
) -> Option<String> {
    assumption.map(|assumption| {
        let call = if assumption.prev_input_name.is_some() {
            format!(
                "{}({}, {}, {})",
                assumption.node_name, input_var, state_expr, prev_input_expr
            )
        } else {
            format!("{}({}, {})", assumption.node_name, input_var, state_expr)
        };
        format!("{equation_name} = {call};")
    })
}

fn emit_relation_equation(
    equation_name: &str,
    relation: &ExportedRelationLustre,
    input_var: &str,
    lhs_state_var: &str,
    rhs_state_var: &str,
    prev_input_var: &str,
) -> String {
    let call = match (
        relation.input_name.is_some(),
        relation.prev_input_name.is_some(),
    ) {
        (false, false) => format!(
            "{}({}, {})",
            relation.node_name, lhs_state_var, rhs_state_var
        ),
        (true, false) => {
            format!(
                "{}({}, {}, {})",
                relation.node_name, input_var, lhs_state_var, rhs_state_var
            )
        }
        (true, true) => format!(
            "{}({}, {}, {}, {})",
            relation.node_name, input_var, lhs_state_var, rhs_state_var, prev_input_var
        ),
        (false, true) => unreachable!(),
    };
    format!("{equation_name} = {call};")
}

fn emit_relation_equation_for_state(
    equation_name: &str,
    relation: &ExportedRelationLustre,
    input_expr: &str,
    lhs_state_expr: &str,
    rhs_state_expr: &str,
    prev_input_expr: &str,
) -> String {
    let call = match (
        relation.input_name.is_some(),
        relation.prev_input_name.is_some(),
    ) {
        (false, false) => format!(
            "{}({}, {})",
            relation.node_name, lhs_state_expr, rhs_state_expr
        ),
        (true, false) => format!(
            "{}({}, {}, {})",
            relation.node_name, input_expr, lhs_state_expr, rhs_state_expr
        ),
        (true, true) => format!(
            "{}({}, {}, {}, {})",
            relation.node_name, input_expr, lhs_state_expr, rhs_state_expr, prev_input_expr
        ),
        (false, true) => unreachable!(),
    };
    format!("{equation_name} = {call};")
}

fn init_proof_domain_expr(lhs_assumption: bool, rhs_assumption: bool) -> String {
    match (lhs_assumption, rhs_assumption) {
        (true, true) => "input_in_range and lhs_assumption_now and rhs_assumption_now".to_string(),
        (true, false) => "input_in_range and lhs_assumption_now".to_string(),
        (false, true) => "input_in_range and rhs_assumption_now".to_string(),
        (false, false) => "input_in_range".to_string(),
    }
}

fn step_current_domain_expr(lhs_assumption: bool, rhs_assumption: bool) -> String {
    let assumptions = match (lhs_assumption, rhs_assumption) {
        (true, true) => " and lhs_assumption_now and rhs_assumption_now",
        (true, false) => " and lhs_assumption_now",
        (false, true) => " and rhs_assumption_now",
        (false, false) => "",
    };
    format!("input_in_range and lhs_state_in_range and rhs_state_in_range{assumptions}")
}

fn step_next_domain_expr(lhs_assumption: bool, rhs_assumption: bool) -> String {
    match (lhs_assumption, rhs_assumption) {
        (true, true) => {
            "next_input_in_range and lhs_assumption_next and rhs_assumption_next".to_string()
        }
        (true, false) => "next_input_in_range and lhs_assumption_next".to_string(),
        (false, true) => "next_input_in_range and rhs_assumption_next".to_string(),
        (false, false) => "next_input_in_range".to_string(),
    }
}

fn step_obligation_input_decl(
    lhs: &ExportedStepLustre,
    rhs: &ExportedStepLustre,
    needs_i_prev: bool,
    needs_i_next: bool,
) -> String {
    let mut decls = vec![
        input_input_decl("i", lhs),
        state_input_decl("s_a", lhs),
        state_input_decl("s_b", rhs),
    ];
    if needs_i_prev {
        decls.push(input_input_decl("i_prev", lhs));
    }
    if needs_i_next {
        decls.push(input_input_decl("i_next", lhs));
    }
    decls.join("; ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::kind2_smt::{Kind2CheckOutcome, Kind2Options, check_kind2_model};
    use crate::parse_ir_text;
    use crate::pir_lustre::{
        AggregateBitsSpec, AggregateFieldSpec, ExportedAssumptionLustre,
        ExportedNextStateAssumptionLustre, ExportedNextStateProjectionLustre,
        ExportedObservationLustre, ExportedRelationLustre, ExportedStateProjectionLustre,
        ObservationParamLayout, StateProjectionParamLayout, export_pir_fn_to_lustre_relation,
        export_pir_fn_to_named_lustre_assumption, export_pir_fn_to_named_lustre_observation,
        export_pir_fn_to_named_lustre_step,
    };
    use xlsynth_pir::ir;

    use super::{
        ObservationalEquivalenceError, emit_combined_assumption_lustre,
        emit_combined_relation_lustre, emit_invariant_init_lustre,
        emit_invariant_step_obligation_lustre, emit_observation_equivalence_relation_lustre,
        emit_observational_equivalence_lustre,
        emit_observational_equivalence_lustre_with_assumptions,
        emit_observational_equivalence_lustre_with_assumptions_and_relation,
        emit_observational_equivalence_lustre_with_assumptions_relation_and_observations,
        emit_state_equality_relation_lustre,
        emit_relation_init_lustre, emit_relation_invariant_lustre, emit_type_equality,
        emit_relation_step_obligation_lustre,
        emit_relation_step_obligation_lustre_with_observations, emit_segmented_int_equality,
        emit_state_projection_init_lustre, emit_state_projection_relation_lustre,
        emit_state_projection_step_obligation_lustre,
        power_of_two_decimal,
    };

    fn tools_available() -> bool {
        Kind2Options::default().runner.is_available()
    }

    fn test_step(
        node_name: &str,
        input_width: usize,
        state_width: usize,
        output_width: usize,
    ) -> crate::ExportedStepLustre {
        let state_only_lustre_text = format!(
            "node {node_name}_state_only(i: int; s: int) returns (s_next: int);\nlet\n  s_next = s;\ntel\n"
        );
        let full_step_lustre_text = format!(
            "node {node_name}(i: int; s: int) returns (o: int; s_next: int; keepalive: bool);\nlet\n  o = i;\n  s_next = s;\n  keepalive = true;\n  --%PROPERTY keepalive;\ntel\n"
        );
        crate::ExportedStepLustre {
            lustre_text: format!("{state_only_lustre_text}{full_step_lustre_text}"),
            type_defs: Vec::new(),
            state_only_lustre_text,
            full_step_lustre_text,
            node_name: node_name.to_string(),
            state_only_node_name: format!("{node_name}_state_only"),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            input_ty: xlsynth_pir::ir::Type::Bits(input_width),
            input_width,
            input_aggregate: None,
            state_ty: xlsynth_pir::ir::Type::Bits(state_width),
            state_width,
            state_aggregate: None,
            output_ty: xlsynth_pir::ir::Type::Bits(output_width),
            output_width,
            output_aggregate: None,
            output_segment_widths: None,
        }
    }

    fn test_structured_step(node_name: &str, state_type_name: &str) -> crate::ExportedStepLustre {
        let type_def = format!("type {state_type_name} = struct {{ f0: int; f1: int }};");
        let state_only_lustre_text = format!(
            "node {node_name}_state_only(i: int; s: {state_type_name}) returns (s_next: {state_type_name});\nlet\n  s_next = {state_type_name} {{ f0 = s.f0; f1 = s.f1 }};\ntel\n"
        );
        let full_step_lustre_text = format!(
            "node {node_name}(i: int; s: {state_type_name}) returns (o: int; s_next: {state_type_name}; keepalive: bool);\nlet\n  o = i;\n  s_next = {state_type_name} {{ f0 = s.f0; f1 = s.f1 }};\n  keepalive = true;\n  --%PROPERTY keepalive;\ntel\n"
        );
        crate::ExportedStepLustre {
            lustre_text: format!("{type_def}\n{state_only_lustre_text}{full_step_lustre_text}"),
            type_defs: vec![type_def],
            state_only_lustre_text,
            full_step_lustre_text,
            node_name: node_name.to_string(),
            state_only_node_name: format!("{node_name}_state_only"),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            input_ty: xlsynth_pir::ir::Type::Bits(8),
            input_width: 8,
            input_aggregate: None,
            state_ty: xlsynth_pir::ir::Type::Tuple(vec![
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
            ]),
            state_width: 8,
            state_aggregate: Some(AggregateBitsSpec {
                type_name: state_type_name.to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 4,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 4,
                    },
                ],
            }),
            output_ty: xlsynth_pir::ir::Type::Bits(8),
            output_width: 8,
            output_aggregate: None,
            output_segment_widths: None,
        }
    }

    fn test_structured_io_step(
        node_name: &str,
        input_type_name: &str,
        output_type_name: &str,
    ) -> crate::ExportedStepLustre {
        let input_type_def = format!("type {input_type_name} = struct {{ f0: int; f1: int }};");
        let output_type_def = format!("type {output_type_name} = struct {{ f0: int; f1: int }};");
        let state_only_lustre_text = format!(
            "node {node_name}_state_only(i: {input_type_name}; s: int) returns (s_next: int);\nlet\n  s_next = s;\ntel\n"
        );
        let full_step_lustre_text = format!(
            "node {node_name}(i: {input_type_name}; s: int) returns (o: {output_type_name}; s_next: int; keepalive: bool);\nlet\n  o = {output_type_name} {{ f0 = i.f1; f1 = i.f0 }};\n  s_next = s;\n  keepalive = true;\n  --%PROPERTY keepalive;\ntel\n"
        );
        crate::ExportedStepLustre {
            lustre_text: format!(
                "{input_type_def}\n{output_type_def}\n{state_only_lustre_text}{full_step_lustre_text}"
            ),
            type_defs: vec![input_type_def, output_type_def],
            state_only_lustre_text,
            full_step_lustre_text,
            node_name: node_name.to_string(),
            state_only_node_name: format!("{node_name}_state_only"),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            input_ty: xlsynth_pir::ir::Type::Tuple(vec![
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
            ]),
            input_width: 8,
            input_aggregate: Some(AggregateBitsSpec {
                type_name: input_type_name.to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 4,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 4,
                    },
                ],
            }),
            state_ty: xlsynth_pir::ir::Type::Bits(8),
            state_width: 8,
            state_aggregate: None,
            output_ty: xlsynth_pir::ir::Type::Tuple(vec![
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
                Box::new(xlsynth_pir::ir::Type::Bits(4)),
            ]),
            output_width: 8,
            output_aggregate: Some(AggregateBitsSpec {
                type_name: output_type_name.to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 4,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 4,
                    },
                ],
            }),
            output_segment_widths: None,
        }
    }

    fn test_output_observation(node_name: &str, output_width: usize) -> ExportedObservationLustre {
        ExportedObservationLustre {
            lustre_text: format!(
                "node {node_name}(o: int) returns (obs: int);\nlet\n  obs = o;\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            param_layout: ObservationParamLayout::OutputOnly,
            input_name: None,
            state_name: None,
            output_name: Some("o".to_string()),
            input_width: None,
            state_width: None,
            output_param_width: Some(output_width),
            observation_width: output_width,
            observation_aggregate: None,
            observation_segment_widths: None,
        }
    }

    fn test_input_state_observation(
        node_name: &str,
        input_width: usize,
        state_width: usize,
        observation_width: usize,
    ) -> ExportedObservationLustre {
        ExportedObservationLustre {
            lustre_text: format!(
                "node {node_name}(i: int; s: int) returns (obs: int);\nlet\n  obs = s;\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            param_layout: ObservationParamLayout::InputState,
            input_name: Some("i".to_string()),
            state_name: Some("s".to_string()),
            output_name: None,
            input_width: Some(input_width),
            state_width: Some(state_width),
            output_param_width: None,
            observation_width,
            observation_aggregate: None,
            observation_segment_widths: None,
        }
    }

    fn test_state_projection(
        node_name: &str,
        input_width: Option<usize>,
        state_width: usize,
        abstraction_width: usize,
        param_layout: StateProjectionParamLayout,
    ) -> ExportedStateProjectionLustre {
        let (input_name, prev_input_name, state_name, lustre_text) = match param_layout {
            StateProjectionParamLayout::StateOnly => (
                None,
                None,
                "s".to_string(),
                format!("node {node_name}(s: int) returns (abs: int);\nlet\n  abs = s;\ntel\n"),
            ),
            StateProjectionParamLayout::InputState => (
                Some("i".to_string()),
                None,
                "s".to_string(),
                format!(
                    "node {node_name}(i: int; s: int) returns (abs: int);\nlet\n  abs = i;\ntel\n"
                ),
            ),
            StateProjectionParamLayout::InputStatePrevInput => (
                Some("i".to_string()),
                Some("i_prev".to_string()),
                "s".to_string(),
                format!(
                    "node {node_name}(i: int; s: int; i_prev: int) returns (abs: int);\nlet\n  abs = i_prev;\ntel\n"
                ),
            ),
        };
        ExportedStateProjectionLustre {
            lustre_text,
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            param_layout,
            input_name,
            state_name,
            prev_input_name,
            input_width,
            state_width,
            abstraction_width,
            abstraction_aggregate: None,
            abstraction_segment_widths: None,
        }
    }

    fn test_next_state_projection(
        node_name: &str,
        needs_next_input: bool,
        input_width: usize,
        state_width: usize,
        abstraction_width: usize,
    ) -> ExportedNextStateProjectionLustre {
        let params = if needs_next_input {
            "i: int; s: int; i_next: int"
        } else {
            "i: int; s: int"
        };
        let abs_expr = if needs_next_input { "i_next" } else { "i" };
        ExportedNextStateProjectionLustre {
            lustre_text: format!(
                "node {node_name}({params}) returns (abs: int);\nlet\n  abs = {abs_expr};\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            next_input_name: needs_next_input.then(|| "i_next".to_string()),
            input_width,
            state_width,
            abstraction_width,
            abstraction_aggregate: None,
            abstraction_segment_widths: None,
        }
    }

    fn test_next_state_assumption(
        node_name: &str,
        input_width: usize,
        state_width: usize,
    ) -> ExportedNextStateAssumptionLustre {
        ExportedNextStateAssumptionLustre {
            lustre_text: format!(
                "node {node_name}(i: int; s: int; i_next: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            next_input_name: "i_next".to_string(),
            input_width,
            state_width,
        }
    }

    fn test_assumption(
        node_name: &str,
        input_width: usize,
        state_width: usize,
        needs_prev_input: bool,
    ) -> ExportedAssumptionLustre {
        let params = if needs_prev_input {
            "i: int; s: int; i_prev: int"
        } else {
            "i: int; s: int"
        };
        ExportedAssumptionLustre {
            lustre_text: format!(
                "node {node_name}({params}) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: needs_prev_input.then_some("i_prev".to_string()),
            input_width,
            state_width,
        }
    }

    fn test_relation(
        node_name: &str,
        needs_input: bool,
        input_width: usize,
        lhs_state_width: usize,
        rhs_state_width: usize,
    ) -> ExportedRelationLustre {
        let params = if needs_input {
            "i: int; s_a: int; s_b: int"
        } else {
            "s_a: int; s_b: int"
        };
        ExportedRelationLustre {
            lustre_text: format!(
                "node {node_name}({params}) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
            ),
            type_defs: Vec::new(),
            node_name: node_name.to_string(),
            input_name: needs_input.then_some("i".to_string()),
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: needs_input.then_some(input_width),
            lhs_state_width,
            rhs_state_width,
        }
    }

    #[test]
    fn emits_paired_harness_for_observational_equivalence() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        assert!(
            model
                .lustre_text
                .contains("(o_a, s_a_next, lhs_keepalive) = step_a(i, s_a);")
        );
        assert!(
            model
                .lustre_text
                .contains("(o_b, s_b_next, rhs_keepalive) = step_b(i, s_b);")
        );
        assert!(model.lustre_text.contains("input_in_range = true;"));
        assert!(model.lustre_text.contains(
            "input_history_in_range = input_in_range and (true -> pre(input_history_in_range));"
        ));
        let outputs_match = emit_segmented_int_equality("o_a", "o_b", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "outputs_match = (not (input_history_in_range)) or ({outputs_match});"
        )));
    }

    #[test]
    fn dedupes_identical_step_supporting_nodes_in_self_proofs() {
        let step = test_step("step_shared", 8, 8, 8);

        let model = emit_observational_equivalence_lustre(&step, &step, 0, 0).unwrap();

        assert_eq!(model.lustre_text.matches("node step_shared(").count(), 1);
        assert_eq!(
            model
                .lustre_text
                .matches("node step_shared_state_only(")
                .count(),
            0
        );
    }

    #[test]
    fn uses_single_step_call_for_reflexive_self_proofs() {
        let step = test_step("step_shared", 8, 8, 8);

        let model = emit_observational_equivalence_lustre(&step, &step, 0, 0).unwrap();

        assert_eq!(
            model
                .lustre_text
                .matches("(o_a, s_next, keepalive) = step_shared(i, s);")
                .count(),
            1
        );
        assert_eq!(model.lustre_text.matches("s_a_next").count(), 0);
        assert_eq!(model.lustre_text.matches("s_b_next").count(), 0);
        assert!(model.lustre_text.contains("o_b = o_a;"));
    }

    #[test]
    fn keeps_paired_harness_when_reflexive_inits_differ() {
        let step = test_step("step_shared", 8, 8, 8);

        let model = emit_observational_equivalence_lustre(&step, &step, 0, 1).unwrap();

        assert!(
            model
                .lustre_text
                .contains("(o_a, s_a_next, lhs_keepalive) = step_shared(i, s_a);")
        );
        assert!(
            model
                .lustre_text
                .contains("(o_b, s_b_next, rhs_keepalive) = step_shared(i, s_b);")
        );
    }

    #[test]
    fn strips_nested_supporting_properties_from_observational_models() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();

        assert_eq!(model.lustre_text.matches("--%PROPERTY ").count(), 1);
        assert!(!model.lustre_text.contains("--%PROPERTY keepalive;"));
        assert!(model.lustre_text.contains("--%PROPERTY outputs_match;"));
    }

    #[test]
    fn emits_structured_input_output_observational_harness() {
        let lhs = test_structured_io_step("step_a", "step_a_input_t", "step_a_output_t");
        let rhs = test_structured_io_step("step_b", "step_b_input_t", "step_b_output_t");

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        assert!(model
            .lustre_text
            .contains("node main(i: step_a_input_t) returns (o_a: step_a_output_t; o_b: step_b_output_t; outputs_match: bool);"));
        assert!(
            model
                .lustre_text
                .contains("(o_a.f0 = o_b.f0) and (o_a.f1 = o_b.f1)")
        );
        assert!(
            model
                .lustre_text
                .contains("input_in_range = (i.f0 <= (uint8 15)) and (i.f1 <= (uint8 15));")
        );
    }

    #[test]
    fn supports_wide_input_ranges() {
        assert_eq!(
            power_of_two_decimal(202).unwrap(),
            "6427752177035961102167848369364650410088811975131171341205504"
        );

        let lhs = test_step("step_a", 202, 166, 212);
        let rhs = crate::ExportedStepLustre {
            lustre_text: lhs.lustre_text.clone(),
            type_defs: Vec::new(),
            state_only_lustre_text: lhs.state_only_lustre_text.clone(),
            full_step_lustre_text: lhs.full_step_lustre_text.clone(),
            node_name: "step_b".to_string(),
            state_only_node_name: "step_b_state_only".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            input_ty: lhs.input_ty.clone(),
            input_width: 202,
            input_aggregate: None,
            state_ty: lhs.state_ty.clone(),
            state_width: 166,
            state_aggregate: None,
            output_ty: lhs.output_ty.clone(),
            output_width: 212,
            output_aggregate: None,
            output_segment_widths: None,
        };

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        assert!(model.lustre_text.contains(
            "input_in_range = (i.f3 <= (uint16 1023));"
        ));
    }

    #[test]
    fn rejects_mismatched_output_widths() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 16);

        let err = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::OutputTypeMismatch { .. }
        ));
    }

    #[test]
    fn emits_assumption_gated_harness_for_observational_equivalence() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_assumption = ExportedAssumptionLustre {
            lustre_text: "node lhs_assumption(i: int; s: int; i_prev: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n".to_string(),
            type_defs: Vec::new(),
            node_name: "lhs_assumption".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: Some("i_prev".to_string()),
            input_width: 8,
            state_width: 8,
        };
        let rhs_assumption = ExportedAssumptionLustre {
            lustre_text: "node rhs_assumption(i: int; s: int; i_prev: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n".to_string(),
            type_defs: Vec::new(),
            node_name: "rhs_assumption".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: Some("i_prev".to_string()),
            input_width: 8,
            state_width: 16,
        };

        let model = emit_observational_equivalence_lustre_with_assumptions(
            &lhs,
            &rhs,
            0,
            0,
            Some(&lhs_assumption),
            Some(&rhs_assumption),
        )
        .unwrap();
        assert!(model.lustre_text.contains("i_prev = (uint8 0) -> pre(i);"));
        assert!(
            model
                .lustre_text
                .contains("lhs_assumption_now = lhs_assumption(i, s_a, i_prev);")
        );
        assert!(
            model
                .lustre_text
                .contains("rhs_assumption_now = rhs_assumption(i, s_b, i_prev);")
        );
        assert!(model.lustre_text.contains(
            "assumption_history = (lhs_assumption_now and rhs_assumption_now) and (true -> pre(assumption_history));"
        ));
        let outputs_match = emit_segmented_int_equality("o_a", "o_b", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "outputs_match = (not (input_history_in_range and assumption_history)) or ({outputs_match});"
        )));
    }

    #[test]
    fn rejects_assumption_width_mismatches() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let bad_assumption = ExportedAssumptionLustre {
            lustre_text: String::new(),
            type_defs: Vec::new(),
            node_name: "lhs_assumption".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: None,
            input_width: 7,
            state_width: 8,
        };
        let err = emit_observational_equivalence_lustre_with_assumptions(
            &lhs,
            &rhs,
            0,
            0,
            Some(&bad_assumption),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::AssumptionInputTypeMismatch { .. }
        ));
    }

    #[test]
    fn emits_relation_gated_harness_for_observational_equivalence() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let relation = ExportedRelationLustre {
            lustre_text:
                "node relation(s_a: int; s_b: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
                    .to_string(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 8,
            rhs_state_width: 16,
        };
        let model = emit_observational_equivalence_lustre_with_assumptions_and_relation(
            &lhs,
            &rhs,
            0,
            0,
            None,
            None,
            Some(&relation),
        )
        .unwrap();
        assert!(
            model
                .lustre_text
                .contains("relation_now = relation(s_a, s_b);")
        );
        let outputs_match = emit_segmented_int_equality("o_a", "o_b", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "outputs_match = (not ((input_history_in_range) and relation_now)) or ({outputs_match});"
        )));
    }

    #[test]
    fn emits_observation_gated_harness_for_observational_equivalence() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_observation = test_output_observation("lhs_observation", 8);
        let rhs_observation = test_output_observation("rhs_observation", 8);

        let model =
            emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
                &lhs,
                &rhs,
                0,
                0,
                None,
                None,
                None,
                Some(&lhs_observation),
                Some(&rhs_observation),
            )
            .unwrap();
        assert!(model.lustre_text.contains("obs_a = lhs_observation(o_a);"));
        assert!(model.lustre_text.contains("obs_b = rhs_observation(o_b);"));
        let outputs_match = emit_segmented_int_equality("obs_a", "obs_b", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "outputs_match = (not (input_history_in_range)) or ({outputs_match});"
        )));
    }

    #[test]
    fn emits_structured_observation_equality_fieldwise() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let lhs_observation = ExportedObservationLustre {
            lustre_text:
                "type lhs_obs_t = struct { f0: int; f1: bool };\nnode lhs_observation(o: int) returns (obs: lhs_obs_t);\nlet\n  obs = lhs_obs_t { f0 = o; f1 = false };\ntel\n"
                    .to_string(),
            type_defs: vec!["type lhs_obs_t = struct { f0: int; f1: bool };".to_string()],
            node_name: "lhs_observation".to_string(),
            param_layout: ObservationParamLayout::OutputOnly,
            input_name: None,
            state_name: None,
            output_name: Some("o".to_string()),
            input_width: None,
            state_width: None,
            output_param_width: Some(8),
            observation_width: 9,
            observation_aggregate: Some(AggregateBitsSpec {
                type_name: "lhs_obs_t".to_string(),
                fields: vec![
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 8,
                    },
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            }),
            observation_segment_widths: None,
        };
        let rhs_observation = ExportedObservationLustre {
            lustre_text:
                "type rhs_obs_t = struct { f0: int; f1: bool };\nnode rhs_observation(o: int) returns (obs: rhs_obs_t);\nlet\n  obs = rhs_obs_t { f0 = o; f1 = false };\ntel\n"
                    .to_string(),
            type_defs: vec!["type rhs_obs_t = struct { f0: int; f1: bool };".to_string()],
            node_name: "rhs_observation".to_string(),
            param_layout: ObservationParamLayout::OutputOnly,
            input_name: None,
            state_name: None,
            output_name: Some("o".to_string()),
            input_width: None,
            state_width: None,
            output_param_width: Some(8),
            observation_width: 9,
            observation_aggregate: Some(AggregateBitsSpec {
                type_name: "rhs_obs_t".to_string(),
                fields: vec![
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 8,
                    },
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            }),
            observation_segment_widths: None,
        };

        let model =
            emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
                &lhs,
                &rhs,
                0,
                0,
                None,
                None,
                None,
                Some(&lhs_observation),
                Some(&rhs_observation),
            )
            .unwrap();
        assert!(model.lustre_text.contains("obs_a: lhs_obs_t;"));
        assert!(model.lustre_text.contains("obs_b: rhs_obs_t;"));
        assert!(
            model
                .lustre_text
                .contains("((obs_a.f0 = obs_b.f0) and (obs_a.f1 = obs_b.f1))")
        );
    }

    #[test]
    fn uses_native_record_equality_for_tuple_types_without_arrays() {
        let ty = ir::Type::Tuple(vec![
            Box::new(ir::Type::Bits(8)),
            Box::new(ir::Type::Tuple(vec![Box::new(ir::Type::Bits(1))])),
        ]);
        assert_eq!(emit_type_equality("lhs", "rhs", &ty), "(lhs = rhs)");
    }

    #[test]
    fn uses_native_record_element_equality_inside_arrays() {
        let ty = ir::Type::Array(ir::ArrayTypeData {
            element_type: Box::new(ir::Type::Tuple(vec![
                Box::new(ir::Type::Bits(1)),
                Box::new(ir::Type::Bits(8)),
            ])),
            element_count: 3,
        });
        let expr = emit_type_equality("lhs", "rhs", &ty);
        assert_eq!(
            expr,
            "((lhs[0] = rhs[0]) and (lhs[1] = rhs[1]) and (lhs[2] = rhs[2]))"
        );
    }

    #[test]
    fn keeps_recursive_equality_for_array_elements_with_nested_arrays() {
        let ty = ir::Type::Array(ir::ArrayTypeData {
            element_type: Box::new(ir::Type::Tuple(vec![Box::new(ir::Type::Array(
                ir::ArrayTypeData {
                    element_type: Box::new(ir::Type::Bits(1)),
                    element_count: 2,
                },
            ))])),
            element_count: 2,
        });
        let expr = emit_type_equality("lhs", "rhs", &ty);
        assert!(expr.contains("lhs[0].f0[0]"));
        assert!(!expr.contains("(lhs[0] = rhs[0])"));
    }

    #[test]
    fn emits_relation_invariant_harness() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let relation = ExportedRelationLustre {
            lustre_text:
                "node relation(s_a: int; s_b: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
                    .to_string(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 8,
            rhs_state_width: 16,
        };
        let model =
            emit_relation_invariant_lustre(&lhs, &rhs, 0, 0, None, None, &relation).unwrap();
        assert!(
            model
                .lustre_text
                .contains("relation_now = relation(s_a, s_b);")
        );
        assert!(
            model
                .lustre_text
                .contains("s_a_next = step_a_state_only(i, s_a);")
        );
        assert!(
            model
                .lustre_text
                .contains("s_b_next = step_b_state_only(i, s_b);")
        );
        assert!(
            model
                .lustre_text
                .contains("relation_holds = (not (input_history_in_range)) or relation_now;")
        );
    }

    #[test]
    fn emits_relation_init_harness() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let relation = ExportedRelationLustre {
            lustre_text:
                "node relation(s_a: int; s_b: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
                    .to_string(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 8,
            rhs_state_width: 16,
        };
        let model = emit_relation_init_lustre(&lhs, &rhs, 0, 0, None, None, &relation).unwrap();
        assert!(
            model
                .lustre_text
                .contains("relation_now = relation((uint8 0), (uint16 0));")
        );
        assert!(
            model
                .lustre_text
                .contains("init_relation_holds = (not (input_in_range)) or relation_now;")
        );
    }

    #[test]
    fn emits_relation_step_obligation_harness() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let relation = ExportedRelationLustre {
            lustre_text:
                "node relation(s_a: int; s_b: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
                    .to_string(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 8,
            rhs_state_width: 16,
        };
        let model =
            emit_relation_step_obligation_lustre(&lhs, &rhs, None, None, &relation).unwrap();
        assert!(
            model
                .lustre_text
                .contains("node main(i: uint8; s_a: uint8; s_b: uint16)")
        );
        assert!(model.lustre_text.contains("s_a_next = step_a_state_only(i, s_a);"));
        assert!(model.lustre_text.contains("s_b_next = step_b_state_only(i, s_b);"));
        assert!(!model.lustre_text.contains("o_a"));
        assert!(!model.lustre_text.contains("o_b"));
        assert!(model.lustre_text.contains("lhs_state_in_range = true;"));
        assert!(model.lustre_text.contains("rhs_state_in_range = true;"));
        assert!(
            model
                .lustre_text
                .contains("relation_now = relation(s_a, s_b);")
        );
        assert!(
            model
                .lustre_text
                .contains("relation_next = relation(s_a_next, s_b_next);")
        );
        assert!(model.lustre_text.contains(&format!(
            "step_preserves_relation = (not ((input_in_range and lhs_state_in_range and rhs_state_in_range) and relation_now)) or relation_next;"
        )));
    }

    #[test]
    fn emits_relation_step_obligation_with_observations() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_observation = test_output_observation("lhs_observation", 8);
        let rhs_observation = test_output_observation("rhs_observation", 8);
        let relation = ExportedRelationLustre {
            lustre_text:
                "node relation(s_a: int; s_b: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n"
                    .to_string(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 8,
            rhs_state_width: 16,
        };
        let model = emit_relation_step_obligation_lustre_with_observations(
            &lhs,
            &rhs,
            None,
            None,
            &relation,
            Some(&lhs_observation),
            Some(&rhs_observation),
        )
        .unwrap();
        assert!(model.lustre_text.contains("obs_a = lhs_observation(o_a);"));
        assert!(model.lustre_text.contains("obs_b = rhs_observation(o_b);"));
        let outputs_match = emit_segmented_int_equality("obs_a", "obs_b", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "step_preserves_relation_and_outputs = (not ((input_in_range and lhs_state_in_range and rhs_state_in_range) and relation_now)) or (({outputs_match}) and relation_next);"
        )));
    }

    #[test]
    fn emits_state_projection_init_harness() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);
        let rhs_projection = test_state_projection(
            "rhs_abs",
            None,
            16,
            8,
            StateProjectionParamLayout::StateOnly,
        );

        let model = emit_state_projection_init_lustre(
            &lhs,
            &rhs,
            0,
            0,
            None,
            None,
            &lhs_projection,
            &rhs_projection,
        )
        .unwrap();
        assert!(model.lustre_text.contains("abs_a = lhs_abs((uint8 0));"));
        assert!(model.lustre_text.contains("abs_b = rhs_abs((uint16 0));"));
        assert!(
            model
                .lustre_text
                .contains("init_state_projection_holds = (not (input_in_range)) or")
        );
        assert!(model.lustre_text.contains("abs_a = abs_b"));
    }

    #[test]
    fn emits_state_projection_step_obligation_harness() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_projection = test_state_projection(
            "lhs_abs",
            Some(8),
            8,
            8,
            StateProjectionParamLayout::InputState,
        );
        let rhs_projection = test_state_projection(
            "rhs_abs",
            Some(8),
            16,
            8,
            StateProjectionParamLayout::InputState,
        );

        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            None,
            None,
            None,
            None,
            None,
            &lhs_projection,
            &rhs_projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            model
                .lustre_text
                .contains("node main(i: uint8; s_a: uint8; s_b: uint16; i_next: uint8)")
        );
        assert!(model.lustre_text.contains("abs_a_now = lhs_abs(i, s_a);"));
        assert!(model.lustre_text.contains("abs_b_now = rhs_abs(i, s_b);"));
        assert!(
            model
                .lustre_text
                .contains("abs_a_next = lhs_abs(i_next, s_a_next);")
        );
        assert!(
            model
                .lustre_text
                .contains("abs_b_next = rhs_abs(i_next, s_b_next);")
        );
        let outputs_match = emit_segmented_int_equality("o_a", "o_b", None).unwrap();
        let abs_now = emit_segmented_int_equality("abs_a_now", "abs_b_now", None).unwrap();
        let abs_next = emit_segmented_int_equality("abs_a_next", "abs_b_next", None).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "step_preserves_state_projection_and_outputs = (not ((input_in_range and lhs_state_in_range and rhs_state_in_range) and ({abs_now}))) or (({outputs_match}) and ((not (next_input_in_range)) or ({abs_next})));"
        )));
    }

    #[test]
    fn dedupes_identical_state_projection_self_proof_supporting_nodes() {
        let step = test_step("step_shared", 8, 8, 8);
        let projection = test_state_projection(
            "shared_abs",
            Some(8),
            8,
            8,
            StateProjectionParamLayout::InputState,
        );

        let model = emit_state_projection_step_obligation_lustre(
            &step,
            &step,
            None,
            None,
            None,
            None,
            None,
            &projection,
            &projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(model.lustre_text.matches("node step_shared(").count(), 1);
        assert_eq!(model.lustre_text.matches("node shared_abs(").count(), 1);
    }

    #[test]
    fn strips_nested_supporting_properties_from_state_projection_models() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_projection = test_state_projection(
            "lhs_abs",
            Some(8),
            8,
            8,
            StateProjectionParamLayout::InputState,
        );
        let rhs_projection = test_state_projection(
            "rhs_abs",
            Some(8),
            16,
            8,
            StateProjectionParamLayout::InputState,
        );

        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            None,
            None,
            None,
            None,
            None,
            &lhs_projection,
            &rhs_projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(model.lustre_text.matches("--%PROPERTY ").count(), 1);
        assert!(!model.lustre_text.contains("--%PROPERTY keepalive;"));
        assert!(model
            .lustre_text
            .contains("--%PROPERTY step_preserves_state_projection_and_outputs;"));
    }

    #[test]
    fn uses_state_only_step_when_observation_does_not_need_outputs() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_observation = test_input_state_observation("lhs_observation", 8, 8, 8);
        let rhs_observation = test_input_state_observation("rhs_observation", 8, 16, 8);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);
        let rhs_projection = test_state_projection(
            "rhs_abs",
            None,
            16,
            8,
            StateProjectionParamLayout::StateOnly,
        );

        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            None,
            None,
            None,
            Some(&lhs_observation),
            Some(&rhs_observation),
            &lhs_projection,
            &rhs_projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            model
                .lustre_text
                .contains("s_a_next = step_a_state_only(i, s_a);")
        );
        assert!(
            model
                .lustre_text
                .contains("s_b_next = step_b_state_only(i, s_b);")
        );
        assert!(
            !model
                .lustre_text
                .contains("(o_a, s_a_next, lhs_keepalive) = step_a(i, s_a);")
        );
        assert!(
            !model
                .lustre_text
                .contains("(o_b, s_b_next, rhs_keepalive) = step_b(i, s_b);")
        );
        assert!(
            model
                .lustre_text
                .contains("obs_a = lhs_observation(i, s_a);")
        );
        assert!(
            model
                .lustre_text
                .contains("obs_b = rhs_observation(i, s_b);")
        );
    }

    #[test]
    fn uses_composed_next_helpers_when_available() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_assumption = ExportedAssumptionLustre {
            lustre_text: "node lhs_assumption(i: int; s: int; i_prev: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n".to_string(),
            type_defs: Vec::new(),
            node_name: "lhs_assumption".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: Some("i_prev".to_string()),
            input_width: 8,
            state_width: 8,
        };
        let rhs_assumption = ExportedAssumptionLustre {
            lustre_text: "node rhs_assumption(i: int; s: int; i_prev: int) returns (ok: bool);\nlet\n  ok = true;\ntel\n".to_string(),
            type_defs: Vec::new(),
            node_name: "rhs_assumption".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            prev_input_name: Some("i_prev".to_string()),
            input_width: 8,
            state_width: 16,
        };
        let lhs_observation = test_input_state_observation("lhs_observation", 8, 8, 8);
        let rhs_observation = test_input_state_observation("rhs_observation", 8, 16, 8);
        let lhs_projection = test_state_projection(
            "lhs_abs",
            Some(8),
            8,
            8,
            StateProjectionParamLayout::InputState,
        );
        let rhs_projection = test_state_projection(
            "rhs_abs",
            Some(8),
            16,
            8,
            StateProjectionParamLayout::InputState,
        );
        let lhs_next_projection = test_next_state_projection("lhs_next_abs", true, 8, 8, 8);
        let rhs_next_projection = test_next_state_projection("rhs_next_abs", true, 8, 16, 8);
        let lhs_next_assumption = test_next_state_assumption("lhs_next_assumption", 8, 8);
        let rhs_next_assumption = test_next_state_assumption("rhs_next_assumption", 8, 16);

        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            Some(&lhs_assumption),
            Some(&rhs_assumption),
            None,
            Some(&lhs_observation),
            Some(&rhs_observation),
            &lhs_projection,
            &rhs_projection,
            Some(&lhs_next_assumption),
            Some(&rhs_next_assumption),
            Some(&lhs_next_projection),
            Some(&rhs_next_projection),
        )
        .unwrap();

        assert!(!model.lustre_text.contains("step_a_state_only(i, s_a)"));
        assert!(!model.lustre_text.contains("step_b_state_only(i, s_b)"));
        assert!(
            model
                .lustre_text
                .contains("lhs_assumption_next = lhs_next_assumption(i, s_a, i_next);")
        );
        assert!(
            model
                .lustre_text
                .contains("rhs_assumption_next = rhs_next_assumption(i, s_b, i_next);")
        );
        assert!(
            model
                .lustre_text
                .contains("abs_a_next = lhs_next_abs(i, s_a, i_next);")
        );
        assert!(
            model
                .lustre_text
                .contains("abs_b_next = rhs_next_abs(i, s_b, i_next);")
        );
    }

    #[test]
    fn emits_combined_assumption_helper() {
        let step = test_step("step_a", 8, 8, 8);
        let env = test_assumption("env", 8, 8, true);
        let inv = test_assumption("inv", 8, 8, false);
        let combined =
            emit_combined_assumption_lustre(&step, "combined", &[&env, &inv]).unwrap().unwrap();
        assert!(combined.prev_input_name.is_some());
        assert!(combined.lustre_text.contains("a_0 = env(i, s, i_prev);"));
        assert!(combined.lustre_text.contains("a_1 = inv(i, s);"));
        assert!(combined.lustre_text.contains("ok = a_0 and a_1;"));
    }

    #[test]
    fn emits_observation_relation_for_input_state_helpers() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_observation = test_input_state_observation("lhs_observation", 8, 8, 8);
        let rhs_observation = test_input_state_observation("rhs_observation", 8, 16, 8);
        let relation = emit_observation_equivalence_relation_lustre(
            &lhs,
            &rhs,
            &lhs_observation,
            &rhs_observation,
            "aux_relation",
        )
        .unwrap();
        assert!(relation.input_width.is_some());
        assert!(relation.lustre_text.contains("obs_a = lhs_observation(i, s_a);"));
        assert!(relation.lustre_text.contains("obs_b = rhs_observation(i, s_b);"));
        assert!(relation.lustre_text.contains("ok = "));
    }

    #[test]
    fn emits_combined_relation_helper() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let rel0 = test_relation("rel0", true, 8, 8, 16);
        let rel1 = test_relation("rel1", false, 8, 8, 16);
        let combined =
            emit_combined_relation_lustre(&lhs, &rhs, "combined_rel", &[&rel0, &rel1])
                .unwrap()
                .unwrap();
        assert!(combined.lustre_text.contains("r_0 = rel0(i, s_a, s_b);"));
        assert!(combined.lustre_text.contains("r_1 = rel1(s_a, s_b);"));
        assert!(combined.lustre_text.contains("ok = r_0 and r_1;"));
    }

    #[test]
    fn emits_state_projection_relation_helper() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);
        let rhs_projection = test_state_projection(
            "rhs_abs",
            None,
            16,
            8,
            StateProjectionParamLayout::StateOnly,
        );
        let relation = emit_state_projection_relation_lustre(
            &lhs,
            &rhs,
            &lhs_projection,
            &rhs_projection,
            "abs_rel",
        )
        .unwrap();
        assert!(relation.lustre_text.contains("abs_a = lhs_abs(s_a);"));
        assert!(relation.lustre_text.contains("abs_b = rhs_abs(s_b);"));
        assert!(relation.lustre_text.contains("ok = "));
    }

    #[test]
    fn emits_direct_state_relation_helper() {
        let lhs = test_structured_step("step_a", "state_t");
        let rhs = test_structured_step("step_b", "state_t");
        let relation = emit_state_equality_relation_lustre(&lhs, &rhs, "state_rel").unwrap();
        assert!(
            relation
                .lustre_text
                .contains("node state_rel(s_a: state_t; s_b: state_t) returns (ok: bool);")
        );
        assert!(relation.lustre_text.contains("ok = ((s_a.f0 = s_b.f0) and (s_a.f1 = s_b.f1));"));
        assert!(!relation.lustre_text.contains("abs_a = "));
        assert!(!relation.lustre_text.contains("abs_b = "));
    }

    #[test]
    fn state_projection_step_supports_current_relation() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 16, 8);
        let relation = test_relation("rel", true, 8, 8, 16);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);
        let rhs_projection = test_state_projection(
            "rhs_abs",
            None,
            16,
            8,
            StateProjectionParamLayout::StateOnly,
        );
        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            None,
            None,
            Some(&relation),
            None,
            None,
            &lhs_projection,
            &rhs_projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(model.lustre_text.contains("relation_now = rel(i, s_a, s_b);"));
        assert!(model.lustre_text.contains("and relation_now and"));
    }

    #[test]
    fn emits_invariant_models() {
        let step = test_step("step_a", 8, 8, 8);
        let env = test_assumption("env", 8, 8, true);
        let invariant = test_assumption("inv", 8, 8, true);
        let init_model = emit_invariant_init_lustre(&step, 0, Some(&env), &invariant).unwrap();
        assert!(init_model.lustre_text.contains("s = (uint8 0);"));
        assert!(init_model.lustre_text.contains("assumption_now = env(i, s, i_prev);"));
        assert!(init_model.lustre_text.contains("invariant_now = inv(i, s, i_prev);"));
        let step_model =
            emit_invariant_step_obligation_lustre(&step, Some(&env), &invariant).unwrap();
        assert!(step_model.lustre_text.contains("s_next = step_a_state_only(i, s);"));
        assert!(step_model.lustre_text.contains("invariant_next = inv(i_next, s_next, i);"));
        assert!(step_model.lustre_text.contains("assumption_next = env(i_next, s_next, i);"));
    }

    #[test]
    fn emits_structured_state_projection_step_obligation_harness() {
        let lhs = test_structured_step("step_a", "step_a_state_t");
        let rhs = test_structured_step("step_b", "step_b_state_t");
        let lhs_projection = ExportedStateProjectionLustre {
            lustre_text:
                "type step_a_state_t = struct { f0: int; f1: int };\ntype lhs_abs_t = struct { f0: int; f1: bool };\nnode lhs_abs(s: step_a_state_t) returns (abs: lhs_abs_t);\nlet\n  abs = lhs_abs_t { f0 = s.f0; f1 = false };\ntel\n"
                    .to_string(),
            type_defs: vec![
                "type step_a_state_t = struct { f0: int; f1: int };".to_string(),
                "type lhs_abs_t = struct { f0: int; f1: bool };".to_string(),
            ],
            node_name: "lhs_abs".to_string(),
            param_layout: StateProjectionParamLayout::StateOnly,
            input_name: None,
            state_name: "s".to_string(),
            prev_input_name: None,
            input_width: None,
            state_width: 8,
            abstraction_width: 4,
            abstraction_aggregate: Some(AggregateBitsSpec {
                type_name: "lhs_abs_t".to_string(),
                fields: vec![
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 3,
                    },
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            }),
            abstraction_segment_widths: None,
        };
        let rhs_projection = ExportedStateProjectionLustre {
            lustre_text:
                "type step_b_state_t = struct { f0: int; f1: int };\ntype rhs_abs_t = struct { f0: int; f1: bool };\nnode rhs_abs(s: step_b_state_t) returns (abs: rhs_abs_t);\nlet\n  abs = rhs_abs_t { f0 = s.f0; f1 = false };\ntel\n"
                    .to_string(),
            type_defs: vec![
                "type step_b_state_t = struct { f0: int; f1: int };".to_string(),
                "type rhs_abs_t = struct { f0: int; f1: bool };".to_string(),
            ],
            node_name: "rhs_abs".to_string(),
            param_layout: StateProjectionParamLayout::StateOnly,
            input_name: None,
            state_name: "s".to_string(),
            prev_input_name: None,
            input_width: None,
            state_width: 8,
            abstraction_width: 4,
            abstraction_aggregate: Some(AggregateBitsSpec {
                type_name: "rhs_abs_t".to_string(),
                fields: vec![
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 3,
                    },
                    crate::pir_lustre::AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            }),
            abstraction_segment_widths: None,
        };

        let model = emit_state_projection_step_obligation_lustre(
            &lhs,
            &rhs,
            None,
            None,
            None,
            None,
            None,
            &lhs_projection,
            &rhs_projection,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            model
                .lustre_text
                .contains("node main(i: uint8; s_a: step_a_state_t; s_b: step_b_state_t)")
        );
        assert!(
            model
                .lustre_text
                .contains("type step_a_state_t = struct { f0: int; f1: int };")
        );
        assert!(
            model
                .lustre_text
                .contains("type step_b_state_t = struct { f0: int; f1: int };")
        );
        assert!(model.lustre_text.contains("abs_a_now: lhs_abs_t;"));
        assert!(model.lustre_text.contains("abs_b_next: rhs_abs_t;"));
        assert!(
            model
                .lustre_text
                .contains("(o_a, s_a_next, lhs_keepalive) = step_a(i, s_a);")
        );
        assert!(model.lustre_text.contains("abs_a_now = lhs_abs(s_a);"));
        assert!(
            model
                .lustre_text
                .contains("abs_b_next = rhs_abs(s_b_next);")
        );
        assert!(
            model
                .lustre_text
                .contains("lhs_state_in_range = (s_a.f0 <= (uint8 15))")
        );
        assert!(
            model
                .lustre_text
                .contains("rhs_state_in_range = (s_b.f0 <= (uint8 15))")
        );
        assert!(
            model
                .lustre_text
                .contains("(abs_a_now.f0 = abs_b_now.f0) and (abs_a_now.f1 = abs_b_now.f1)")
        );
    }

    #[test]
    fn rejects_state_projection_state_width_mismatches() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);

        let err = emit_state_projection_init_lustre(
            &lhs,
            &rhs,
            0,
            0,
            None,
            None,
            &lhs_projection,
            // The missing rhs helper is modeled through the pair validator on the step API.
            &ExportedStateProjectionLustre {
                lustre_text: String::new(),
                type_defs: Vec::new(),
                node_name: "rhs_abs".to_string(),
                param_layout: StateProjectionParamLayout::StateOnly,
                input_name: None,
                state_name: "s".to_string(),
                prev_input_name: None,
                input_width: None,
                state_width: 7,
                abstraction_width: 8,
                abstraction_aggregate: None,
                abstraction_segment_widths: None,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::StateProjectionStateTypeMismatch { which: "rhs", .. }
        ));
    }

    #[test]
    fn rejects_state_projection_width_mismatches() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let lhs_projection =
            test_state_projection("lhs_abs", None, 8, 8, StateProjectionParamLayout::StateOnly);
        let rhs_projection = test_state_projection(
            "rhs_abs",
            None,
            8,
            16,
            StateProjectionParamLayout::StateOnly,
        );

        let err = emit_state_projection_init_lustre(
            &lhs,
            &rhs,
            0,
            0,
            None,
            None,
            &lhs_projection,
            &rhs_projection,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::StateProjectionTypeMismatch { .. }
        ));
    }

    #[test]
    fn rejects_missing_rhs_observation() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let lhs_observation = test_output_observation("lhs_observation", 8);

        let err = emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
            &lhs,
            &rhs,
            0,
            0,
            None,
            None,
            None,
            Some(&lhs_observation),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::ObservationMissing { which: "rhs" }
        ));
    }

    #[test]
    fn rejects_relation_width_mismatches() {
        let lhs = test_step("step_a", 8, 8, 8);
        let rhs = test_step("step_b", 8, 8, 8);
        let relation = ExportedRelationLustre {
            lustre_text: String::new(),
            type_defs: Vec::new(),
            node_name: "relation".to_string(),
            input_name: None,
            lhs_state_name: "s_a".to_string(),
            rhs_state_name: "s_b".to_string(),
            prev_input_name: None,
            input_width: None,
            lhs_state_width: 7,
            rhs_state_width: 8,
        };
        let err =
            emit_relation_invariant_lustre(&lhs, &rhs, 0, 0, None, None, &relation).unwrap_err();
        assert!(matches!(
            err,
            ObservationalEquivalenceError::RelationLhsStateTypeMismatch { .. }
        ));
    }

    #[test]
    fn uses_segmented_output_equality_when_available() {
        let mut lhs = test_step("step_a", 8, 8, 8);
        lhs.output_segment_widths = Some(vec![4, 4]);
        let mut rhs = test_step("step_b", 8, 8, 8);
        rhs.output_segment_widths = Some(vec![4, 4]);

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        let outputs_match = emit_segmented_int_equality("o_a", "o_b", Some(&[4, 4])).unwrap();
        assert!(model.lustre_text.contains(&format!(
            "outputs_match = (not (input_history_in_range)) or ({outputs_match});"
        )));
    }

    #[test]
    fn proves_equivalent_outputs_for_different_state_widths() {
        if !tools_available() {
            return;
        }

        let lhs_ir = r#"
package lhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  o: bits[8] = add(i, s, id=3)
  one: bits[8] = literal(value=1, id=4)
  next_s: bits[8] = add(s, one, id=5)
  ret tuple.6: (bits[8], bits[8]) = tuple(o, next_s, id=6)
}
"#;
        let rhs_ir = r#"
package rhs_machine

top fn step(i: bits[8] id=1, s: bits[16] id=2) -> (bits[8], bits[16]) {
  i_wide: bits[16] = zero_ext(i, new_bit_count=16, id=3)
  sum: bits[16] = add(i_wide, s, id=4)
  o: bits[8] = bit_slice(sum, start=0, width=8, id=5)
  one: bits[16] = literal(value=1, id=6)
  next_s: bits[16] = add(s, one, id=7)
  ret tuple.8: (bits[8], bits[16]) = tuple(o, next_s, id=8)
}
"#;
        let lhs_artifacts = parse_ir_text(Path::new("lhs.ir"), lhs_ir, Some("step")).unwrap();
        let rhs_artifacts = parse_ir_text(Path::new("rhs.ir"), rhs_ir, Some("step")).unwrap();
        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Kind2CheckOutcome::Valid(_)));
    }

    #[test]
    fn detects_non_equivalent_outputs() {
        if !tools_available() {
            return;
        }

        let lhs_ir = r#"
package lhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  ret tuple.3: (bits[8], bits[8]) = tuple(i, s, id=3)
}
"#;
        let rhs_ir = r#"
package rhs_machine

top fn step(i: bits[8] id=1, s: bits[16] id=2) -> (bits[8], bits[16]) {
  one: bits[8] = literal(value=1, id=3)
  o: bits[8] = add(i, one, id=4)
  ret tuple.5: (bits[8], bits[16]) = tuple(o, s, id=5)
}
"#;
        let lhs_artifacts = parse_ir_text(Path::new("lhs_bad.ir"), lhs_ir, Some("step")).unwrap();
        let rhs_artifacts = parse_ir_text(Path::new("rhs_bad.ir"), rhs_ir, Some("step")).unwrap();
        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();

        let model = emit_observational_equivalence_lustre(&lhs, &rhs, 0, 0).unwrap();
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Kind2CheckOutcome::Invalid(_)));
    }

    #[test]
    fn proves_equivalent_outputs_with_assumptions() {
        if !tools_available() {
            return;
        }

        let lhs_ir = r#"
package lhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  ret tuple.3: (bits[8], bits[8]) = tuple(i, s, id=3)
}

fn env(i: bits[8] id=4, s: bits[8] id=5, i_prev: bits[8] id=6) -> bits[1] {
  zero: bits[8] = literal(value=0, id=7)
  busy: bits[1] = ne(s, zero, id=8)
  stable: bits[1] = eq(i, i_prev, id=9)
  one: bits[1] = literal(value=1, id=10)
  ret sel.11: bits[1] = sel(busy, cases=[one, stable], id=11)
}
"#;
        let rhs_ir = r#"
package rhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  ret tuple.3: (bits[8], bits[8]) = tuple(i, s, id=3)
}

fn env(i: bits[8] id=4, s: bits[8] id=5, i_prev: bits[8] id=6) -> bits[1] {
  zero: bits[8] = literal(value=0, id=7)
  busy: bits[1] = ne(s, zero, id=8)
  stable: bits[1] = eq(i, i_prev, id=9)
  one: bits[1] = literal(value=1, id=10)
  ret sel.11: bits[1] = sel(busy, cases=[one, stable], id=11)
}
"#;
        let lhs_artifacts =
            parse_ir_text(Path::new("lhs_assume.ir"), lhs_ir, Some("step")).unwrap();
        let rhs_artifacts =
            parse_ir_text(Path::new("rhs_assume.ir"), rhs_ir, Some("step")).unwrap();
        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();
        let lhs_assumption_artifacts =
            parse_ir_text(Path::new("lhs_assume.ir"), lhs_ir, Some("env")).unwrap();
        let rhs_assumption_artifacts =
            parse_ir_text(Path::new("rhs_assume.ir"), rhs_ir, Some("env")).unwrap();
        let lhs_assumption = export_pir_fn_to_named_lustre_assumption(
            lhs_assumption_artifacts.top_fn().unwrap(),
            "lhs_assumption",
        )
        .unwrap();
        let rhs_assumption = export_pir_fn_to_named_lustre_assumption(
            rhs_assumption_artifacts.top_fn().unwrap(),
            "rhs_assumption",
        )
        .unwrap();

        let model = emit_observational_equivalence_lustre_with_assumptions(
            &lhs,
            &rhs,
            0,
            0,
            Some(&lhs_assumption),
            Some(&rhs_assumption),
        )
        .unwrap();
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Kind2CheckOutcome::Valid(_)));
    }

    #[test]
    fn proves_relation_then_outputs_with_relation() {
        if !tools_available() {
            return;
        }

        let lhs_ir = r#"
package lhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  next_s: bits[8] = add(s, i, id=3)
  ret tuple.4: (bits[8], bits[8]) = tuple(s, next_s, id=4)
}
"#;
        let rhs_ir = lhs_ir;
        let rel_ir = r#"
package rel_machine

top fn rel(s_a: bits[8] id=1, s_b: bits[8] id=2) -> bits[1] {
  ret eq.3: bits[1] = eq(s_a, s_b, id=3)
}
"#;
        let obs_ir = r#"
package obs_machine

top fn obs(o: bits[8] id=1) -> bits[8] {
  ret identity.2: bits[8] = identity(o, id=2)
}
"#;
        let lhs_artifacts = parse_ir_text(Path::new("lhs_rel.ir"), lhs_ir, Some("step")).unwrap();
        let rhs_artifacts = parse_ir_text(Path::new("rhs_rel.ir"), rhs_ir, Some("step")).unwrap();
        let rel_artifacts = parse_ir_text(Path::new("rel.ir"), rel_ir, Some("rel")).unwrap();
        let lhs_obs_artifacts =
            parse_ir_text(Path::new("lhs_obs.ir"), obs_ir, Some("obs")).unwrap();
        let rhs_obs_artifacts =
            parse_ir_text(Path::new("rhs_obs.ir"), obs_ir, Some("obs")).unwrap();
        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();
        let relation = export_pir_fn_to_lustre_relation(rel_artifacts.top_fn().unwrap()).unwrap();
        let lhs_observation = export_pir_fn_to_named_lustre_observation(
            lhs_obs_artifacts.top_fn().unwrap(),
            "lhs_observation",
        )
        .unwrap();
        let rhs_observation = export_pir_fn_to_named_lustre_observation(
            rhs_obs_artifacts.top_fn().unwrap(),
            "rhs_observation",
        )
        .unwrap();

        let relation_model =
            emit_relation_invariant_lustre(&lhs, &rhs, 0, 0, None, None, &relation).unwrap();
        let relation_outcome = check_kind2_model(
            &relation_model.lustre_text,
            relation_model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(relation_outcome, Kind2CheckOutcome::Valid(_)));

        let model =
            emit_observational_equivalence_lustre_with_assumptions_relation_and_observations(
                &lhs,
                &rhs,
                0,
                0,
                None,
                None,
                Some(&relation),
                Some(&lhs_observation),
                Some(&rhs_observation),
            )
            .unwrap();
        let outcome = check_kind2_model(
            &model.lustre_text,
            model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Kind2CheckOutcome::Valid(_)));
    }

    #[test]
    fn proves_decomposed_relation_obligations() {
        if !tools_available() {
            return;
        }

        let lhs_ir = r#"
package lhs_machine

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  next_s: bits[8] = add(s, i, id=3)
  ret tuple.4: (bits[8], bits[8]) = tuple(s, next_s, id=4)
}
"#;
        let rhs_ir = lhs_ir;
        let rel_ir = r#"
package rel_machine

top fn rel(s_a: bits[8] id=1, s_b: bits[8] id=2) -> bits[1] {
  ret eq.3: bits[1] = eq(s_a, s_b, id=3)
}
"#;
        let obs_ir = r#"
package obs_machine

top fn obs(o: bits[8] id=1) -> bits[8] {
  ret identity.2: bits[8] = identity(o, id=2)
}
"#;
        let lhs_artifacts =
            parse_ir_text(Path::new("lhs_rel_step.ir"), lhs_ir, Some("step")).unwrap();
        let rhs_artifacts =
            parse_ir_text(Path::new("rhs_rel_step.ir"), rhs_ir, Some("step")).unwrap();
        let rel_artifacts = parse_ir_text(Path::new("rel_step.ir"), rel_ir, Some("rel")).unwrap();
        let lhs_obs_artifacts =
            parse_ir_text(Path::new("lhs_rel_step_obs.ir"), obs_ir, Some("obs")).unwrap();
        let rhs_obs_artifacts =
            parse_ir_text(Path::new("rhs_rel_step_obs.ir"), obs_ir, Some("obs")).unwrap();
        let lhs =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();
        let relation = export_pir_fn_to_lustre_relation(rel_artifacts.top_fn().unwrap()).unwrap();
        let lhs_observation = export_pir_fn_to_named_lustre_observation(
            lhs_obs_artifacts.top_fn().unwrap(),
            "lhs_observation",
        )
        .unwrap();
        let rhs_observation = export_pir_fn_to_named_lustre_observation(
            rhs_obs_artifacts.top_fn().unwrap(),
            "rhs_observation",
        )
        .unwrap();

        let init_model =
            emit_relation_init_lustre(&lhs, &rhs, 0, 0, None, None, &relation).unwrap();
        let init_outcome = check_kind2_model(
            &init_model.lustre_text,
            init_model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(init_outcome, Kind2CheckOutcome::Valid(_)));

        let step_model = emit_relation_step_obligation_lustre_with_observations(
            &lhs,
            &rhs,
            None,
            None,
            &relation,
            Some(&lhs_observation),
            Some(&rhs_observation),
        )
        .unwrap();
        let step_outcome = check_kind2_model(
            &step_model.lustre_text,
            step_model.top_node.as_str(),
            &Kind2Options::default(),
        )
        .unwrap();
        assert!(matches!(step_outcome, Kind2CheckOutcome::Valid(_)));
    }
}
