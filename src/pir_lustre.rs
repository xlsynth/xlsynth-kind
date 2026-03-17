use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};

use xlsynth::{IrBits, XlsynthError, ir_value::IrFormatPreference};
use xlsynth_pir::ir::{self, Binop, NaryOp, NodePayload, NodeRef, Unop};

use crate::pipeline::{PipelineError, infer_recurrence_signature};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedStepLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub state_only_lustre_text: String,
    pub full_step_lustre_text: String,
    pub node_name: String,
    pub state_only_node_name: String,
    pub input_name: String,
    pub state_name: String,
    pub input_ty: ir::Type,
    pub input_width: usize,
    pub input_aggregate: Option<AggregateBitsSpec>,
    pub state_ty: ir::Type,
    pub state_width: usize,
    pub state_aggregate: Option<AggregateBitsSpec>,
    pub output_ty: ir::Type,
    pub output_width: usize,
    pub output_aggregate: Option<AggregateBitsSpec>,
    pub output_segment_widths: Option<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateFieldSpec {
    pub name: String,
    pub width: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarBitsType {
    BigInt,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
}

impl ScalarBitsType {
    fn for_width(bit_count: usize) -> Self {
        match bit_count {
            2..=8 => Self::UInt8,
            9..=16 => Self::UInt16,
            17..=32 => Self::UInt32,
            33..=64 => Self::UInt64,
            _ => Self::BigInt,
        }
    }

    fn lustre_type_name(self) -> &'static str {
        match self {
            Self::BigInt => "int",
            Self::UInt8 => "uint8",
            Self::UInt16 => "uint16",
            Self::UInt32 => "uint32",
            Self::UInt64 => "uint64",
        }
    }

    fn carrier_width(self) -> Option<usize> {
        match self {
            Self::BigInt => None,
            Self::UInt8 => Some(8),
            Self::UInt16 => Some(16),
            Self::UInt32 => Some(32),
            Self::UInt64 => Some(64),
        }
    }

    fn is_machine(self) -> bool {
        self != Self::BigInt
    }
}

pub(crate) fn lustre_bits_type_name(bit_count: usize) -> &'static str {
    ScalarBitsType::for_width(bit_count).lustre_type_name()
}

pub(crate) fn bits_use_native_chunks(bit_count: usize) -> bool {
    bit_count > 64
}

pub(crate) fn wide_bits_chunk_widths(bit_count: usize) -> Vec<usize> {
    let mut remaining = bit_count;
    let mut widths = Vec::new();
    while remaining > 64 {
        widths.push(64);
        remaining -= 64;
    }
    if remaining > 0 {
        widths.push(remaining);
    }
    widths
}

fn wide_bits_chunk_field_name(index: usize) -> String {
    format!("f{index}")
}

fn wide_bits_chunk_access_expr(expr: &str, index: usize) -> String {
    format!("({expr}).{}", wide_bits_chunk_field_name(index))
}

fn wide_bits_type_ref(bit_count: usize) -> Result<String, PirLustreError> {
    native_named_type_name(&ir::Type::Bits(bit_count))
}

pub(crate) fn native_bits_type_ref(bit_count: usize) -> Result<String, PirLustreError> {
    Ok(if bit_count == 1 {
        "bool".to_string()
    } else if bits_use_native_chunks(bit_count) {
        wide_bits_type_ref(bit_count)?
    } else {
        lustre_bits_type_name(bit_count).to_string()
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateBitsSpec {
    pub type_name: String,
    pub fields: Vec<AggregateFieldSpec>,
}

impl AggregateBitsSpec {
    fn total_width(&self) -> usize {
        self.fields.iter().map(|field| field.width).sum()
    }

    fn type_def_text(&self) -> String {
        let fields = self
            .fields
            .iter()
            .map(|field| {
                let ty = if field.width == 1 {
                    "bool"
                } else {
                    lustre_bits_type_name(field.width)
                };
                format!("{}: {}", field.name, ty)
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("type {} = struct {{ {} }};", self.type_name, fields)
    }
}

fn stable_type_descriptor(ty: &ir::Type) -> Result<String, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => Ok(format!("b{width}")),
        ir::Type::Tuple(members) => {
            let parts = members
                .iter()
                .map(|member| stable_type_descriptor(member))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("t({})", parts.join(",")))
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => Ok(format!(
            "a{element_count}({})",
            stable_type_descriptor(element_type)?
        )),
        other => Err(PirLustreError::Unsupported(format!(
            "expected native composite type to be bits/tuple/array, got {other}"
        ))),
    }
}

fn stable_type_hash(ty: &ir::Type) -> Result<u64, PirLustreError> {
    let descriptor = stable_type_descriptor(ty)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    descriptor.hash(&mut hasher);
    Ok(hasher.finish())
}

fn native_named_type_name(ty: &ir::Type) -> Result<String, PirLustreError> {
    Ok(format!("xk2_t_{:016x}", stable_type_hash(ty)?))
}

pub(crate) fn native_lustre_type_ref(ty: &ir::Type) -> Result<String, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => native_bits_type_ref(*width),
        ir::Type::Tuple(_) | ir::Type::Array(_) => native_named_type_name(ty),
        other => Err(PirLustreError::Unsupported(format!(
            "expected native composite type to be bits/tuple/array, got {other}"
        ))),
    }
}

fn exported_lustre_type_ref(
    aggregate: Option<&AggregateBitsSpec>,
    ty: &ir::Type,
) -> Result<String, PirLustreError> {
    if let Some(aggregate) = aggregate {
        Ok(aggregate.type_name.clone())
    } else {
        native_lustre_type_ref(ty)
    }
}

fn register_native_type_if_needed(
    aggregate: Option<&AggregateBitsSpec>,
    ty: &ir::Type,
    native_types: &mut NativeTypeRegistry,
) -> Result<(), PirLustreError> {
    if aggregate.is_none()
        && (matches!(ty, ir::Type::Tuple(_) | ir::Type::Array(_))
            || matches!(ty, ir::Type::Bits(width) if bits_use_native_chunks(*width)))
    {
        native_types.register(ty)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalBinding {
    name: String,
    ty: String,
    expr: String,
}

#[derive(Default)]
struct LoweringContext {
    cache: BTreeMap<usize, LoweredValue>,
    bindings: Vec<LocalBinding>,
    structured_params: BTreeMap<String, LoweredValue>,
    native_types: NativeTypeRegistry,
    hoist_non_leaf: bool,
    binding_prefix: Option<String>,
}

impl LoweringContext {
    fn inline() -> Self {
        Self::default()
    }

    fn hoisted() -> Self {
        Self {
            hoist_non_leaf: true,
            ..Self::default()
        }
    }

    fn with_structured_params(
        mut self,
        params: impl IntoIterator<Item = StructuredParamSpec>,
    ) -> Self {
        self.structured_params = params
            .into_iter()
            .map(|param| (param.param_name, param.value))
            .collect();
        self
    }

    fn with_native_types(mut self, native_types: NativeTypeRegistry) -> Self {
        self.native_types = native_types;
        self
    }

    fn with_binding_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.binding_prefix = Some(prefix.into());
        self
    }

    fn binding_name(&self, kind: &str, index: usize) -> String {
        match self.binding_prefix.as_deref() {
            Some(prefix) => format!("{prefix}_{kind}_{index}"),
            None => format!("{kind}_{index}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NativeTypeRegistry {
    defs: Vec<String>,
    seen: BTreeSet<String>,
}

impl NativeTypeRegistry {
    fn register(&mut self, ty: &ir::Type) -> Result<(), PirLustreError> {
        match ty {
            ir::Type::Bits(width) => {
                if !bits_use_native_chunks(*width) {
                    return Ok(());
                }
                let type_name = native_named_type_name(ty)?;
                if self.seen.insert(type_name.clone()) {
                    let chunk_fields = wide_bits_chunk_widths(*width)
                        .into_iter()
                        .enumerate()
                        .map(|(index, chunk_width)| {
                            Ok::<_, PirLustreError>(format!(
                                "{}: {}",
                                wide_bits_chunk_field_name(index),
                                native_bits_type_ref(chunk_width)?
                            ))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .join("; ");
                    self.defs
                        .push(format!("type {type_name} = struct {{ {chunk_fields} }};"));
                }
                Ok(())
            }
            ir::Type::Tuple(members) => {
                for member in members {
                    self.register(member)?;
                }
                let type_name = native_named_type_name(ty)?;
                if self.seen.insert(type_name.clone()) {
                    let fields = members
                        .iter()
                        .enumerate()
                        .map(|(index, member)| {
                            Ok(format!("f{index}: {}", native_lustre_type_ref(member)?))
                        })
                        .collect::<Result<Vec<_>, PirLustreError>>()?
                        .join("; ");
                    self.defs
                        .push(format!("type {type_name} = struct {{ {fields} }};"));
                }
                Ok(())
            }
            ir::Type::Array(ir::ArrayTypeData {
                element_type,
                element_count,
            }) => {
                self.register(element_type)?;
                let type_name = native_named_type_name(ty)?;
                if self.seen.insert(type_name.clone()) {
                    self.defs.push(format!(
                        "type {type_name} = {} ^ {};",
                        native_lustre_type_ref(element_type)?,
                        element_count
                    ));
                }
                Ok(())
            }
            other => Err(PirLustreError::Unsupported(format!(
                "expected native composite type to be bits/tuple/array, got {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StructuredParamSpec {
    param_name: String,
    value: LoweredValue,
}

fn render_lustre_node(
    node_name: &str,
    params_text: &str,
    returns_text: &str,
    bindings: &[LocalBinding],
    assignments: &[String],
    property_name: Option<&str>,
) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "node {node_name}({params_text}) returns ({returns_text});\n"
    ));
    if !bindings.is_empty() {
        text.push_str("var\n");
        for binding in bindings {
            text.push_str(&format!("  {}: {};\n", binding.name, binding.ty));
        }
    }
    text.push_str("let\n");
    for binding in bindings {
        text.push_str(&format!("  {} = {};\n", binding.name, binding.expr));
    }
    for assignment in assignments {
        text.push_str("  ");
        text.push_str(assignment);
        text.push('\n');
    }
    if let Some(property_name) = property_name {
        text.push_str(&format!("  --%PROPERTY {property_name};\n"));
    }
    text.push_str("tel\n");
    text
}

fn should_hoist_node(payload: &NodePayload) -> bool {
    !matches!(
        payload,
        NodePayload::GetParam(_)
            | NodePayload::Literal(_)
            | NodePayload::TupleIndex { .. }
    )
}

fn maybe_hoist_value(
    ctx: &mut LoweringContext,
    node_ref: &NodeRef,
    node: &ir::Node,
    lowered: LoweredValue,
) -> Result<LoweredValue, PirLustreError> {
    if !ctx.hoist_non_leaf || !should_hoist_node(&node.payload) {
        return Ok(lowered);
    }
    let hoisted = match lowered {
        LoweredValue::Bits { expr, width } => {
            let name = ctx.binding_name("n", node_ref.index);
            ctx.bindings.push(LocalBinding {
                name: name.clone(),
                ty: lustre_bits_type_name(width).to_string(),
                expr,
            });
            LoweredValue::Bits { expr: name, width }
        }
        LoweredValue::Bool(expr) => {
            let name = ctx.binding_name("b", node_ref.index);
            ctx.bindings.push(LocalBinding {
                name: name.clone(),
                ty: "bool".to_string(),
                expr,
            });
            LoweredValue::Bool(name)
        }
        LoweredValue::Composite { expr, ty } => {
            let name = ctx.binding_name("c", node_ref.index);
            ctx.native_types.register(&ty)?;
            ctx.bindings.push(LocalBinding {
                name: name.clone(),
                ty: native_lustre_type_ref(&ty)?,
                expr,
            });
            LoweredValue::Composite { expr: name, ty }
        }
        LoweredValue::Aggregate { spec, fields } => LoweredValue::Aggregate { spec, fields },
        LoweredValue::Tuple(values) => LoweredValue::Tuple(values),
    };
    Ok(hoisted)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedAssumptionLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub input_name: String,
    pub state_name: String,
    pub prev_input_name: Option<String>,
    pub input_width: usize,
    pub state_width: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationParamLayout {
    OutputOnly,
    InputState,
    InputStateOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateProjectionParamLayout {
    StateOnly,
    InputState,
    InputStatePrevInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedObservationLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub param_layout: ObservationParamLayout,
    pub input_name: Option<String>,
    pub state_name: Option<String>,
    pub output_name: Option<String>,
    pub input_width: Option<usize>,
    pub state_width: Option<usize>,
    pub output_param_width: Option<usize>,
    pub observation_width: usize,
    pub observation_aggregate: Option<AggregateBitsSpec>,
    pub observation_segment_widths: Option<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedStateProjectionLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub param_layout: StateProjectionParamLayout,
    pub input_name: Option<String>,
    pub state_name: String,
    pub prev_input_name: Option<String>,
    pub input_width: Option<usize>,
    pub state_width: usize,
    pub abstraction_width: usize,
    pub abstraction_aggregate: Option<AggregateBitsSpec>,
    pub abstraction_segment_widths: Option<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedNextStateProjectionLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub input_name: String,
    pub state_name: String,
    pub next_input_name: Option<String>,
    pub input_width: usize,
    pub state_width: usize,
    pub abstraction_width: usize,
    pub abstraction_aggregate: Option<AggregateBitsSpec>,
    pub abstraction_segment_widths: Option<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedNextStateAssumptionLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub input_name: String,
    pub state_name: String,
    pub next_input_name: String,
    pub input_width: usize,
    pub state_width: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedRelationLustre {
    pub lustre_text: String,
    pub type_defs: Vec<String>,
    pub node_name: String,
    pub input_name: Option<String>,
    pub lhs_state_name: String,
    pub rhs_state_name: String,
    pub prev_input_name: Option<String>,
    pub input_width: Option<usize>,
    pub lhs_state_width: usize,
    pub rhs_state_width: usize,
}

#[derive(Debug)]
pub enum PirLustreError {
    Pipeline(PipelineError),
    Xlsynth(XlsynthError),
    Unsupported(String),
}

impl Display for PirLustreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(err) => write!(f, "{err}"),
            Self::Xlsynth(err) => write!(f, "{err}"),
            Self::Unsupported(err) => write!(f, "{err}"),
        }
    }
}

impl Error for PirLustreError {}

impl From<PipelineError> for PirLustreError {
    fn from(value: PipelineError) -> Self {
        Self::Pipeline(value)
    }
}

impl From<XlsynthError> for PirLustreError {
    fn from(value: XlsynthError) -> Self {
        Self::Xlsynth(value)
    }
}

pub fn export_pir_fn_to_lustre_step(func: &ir::Fn) -> Result<ExportedStepLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_step(func, "step")
}

pub fn export_pir_fn_to_lustre_assumption(
    func: &ir::Fn,
) -> Result<ExportedAssumptionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_assumption(func, "assumption")
}

pub fn export_pir_fn_to_lustre_observation(
    func: &ir::Fn,
) -> Result<ExportedObservationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_observation(func, "observation")
}

pub fn export_pir_fn_to_lustre_state_projection(
    func: &ir::Fn,
) -> Result<ExportedStateProjectionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_state_projection(func, "state_projection")
}

pub fn export_pir_fn_to_lustre_relation(
    func: &ir::Fn,
) -> Result<ExportedRelationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_relation(func, "relation")
}

pub fn export_pir_fn_to_named_lustre_assumption_with_aggregates(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedAssumptionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_assumption_impl(func, node_name, input_aggregate, state_aggregate)
}

pub fn export_pir_fn_to_named_lustre_assumption_with_state_aggregate(
    func: &ir::Fn,
    node_name: &str,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedAssumptionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_assumption_with_aggregates(func, node_name, None, state_aggregate)
}

pub fn export_pir_fn_to_named_lustre_observation_with_aggregates(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
    output_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedObservationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_observation_impl(
        func,
        node_name,
        input_aggregate,
        state_aggregate,
        output_aggregate,
    )
}

pub fn export_pir_fn_to_named_lustre_observation_with_state_aggregate(
    func: &ir::Fn,
    node_name: &str,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedObservationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_observation_with_aggregates(
        func,
        node_name,
        None,
        state_aggregate,
        None,
    )
}

pub fn export_pir_fn_to_named_lustre_state_projection_with_aggregates(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedStateProjectionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_state_projection_impl(
        func,
        node_name,
        input_aggregate,
        state_aggregate,
    )
}

pub fn export_pir_fn_to_named_lustre_state_projection_with_state_aggregate(
    func: &ir::Fn,
    node_name: &str,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedStateProjectionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_state_projection_with_aggregates(
        func,
        node_name,
        None,
        state_aggregate,
    )
}

fn get_step_next_state_node_ref(
    func: &ir::Fn,
) -> Result<(crate::pipeline::RecurrenceSignature, NodeRef), PirLustreError> {
    let signature = infer_recurrence_signature(func)?;
    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "top function `{}` does not have a return node",
            func.name
        ))
    })?;
    let ret_node = func.get_node(ret_node_ref);
    let NodePayload::Tuple(return_members) = &ret_node.payload else {
        return Err(PirLustreError::Unsupported(format!(
            "top function `{}` return node is not a tuple",
            func.name
        )));
    };
    if return_members.len() != 2 {
        return Err(PirLustreError::Unsupported(format!(
            "top function `{}` must return a 2-tuple, got {} elements",
            func.name,
            return_members.len()
        )));
    }
    Ok((
        signature.clone(),
        return_members[signature.next_state_index],
    ))
}

fn lower_step_next_state_as_helper_value(
    step_func: &ir::Fn,
    step_signature: &crate::pipeline::RecurrenceSignature,
    next_state_ref: &NodeRef,
    helper_state_ty: &ir::Type,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
    helper_input_expr_name: &str,
    helper_state_expr_name: &str,
    binding_prefix: &str,
) -> Result<(Vec<LocalBinding>, NativeTypeRegistry, LoweredValue), PirLustreError> {
    let mut native_types = NativeTypeRegistry::default();
    register_native_type_if_needed(input_aggregate, &step_signature.input_ty, &mut native_types)?;
    register_native_type_if_needed(state_aggregate, &step_signature.state_ty, &mut native_types)?;
    register_native_type_if_needed(None, helper_state_ty, &mut native_types)?;
    let mut structured_params = Vec::new();
    if let Some(spec) = make_boundary_param_spec_from_expr_name(
        &sanitize_name(&step_signature.input_name),
        helper_input_expr_name,
        &step_signature.input_ty,
        input_aggregate,
        &mut native_types,
    )? {
        structured_params.push(spec);
    }
    if let Some(spec) = make_boundary_param_spec_from_expr_name(
        &sanitize_name(&step_signature.state_name),
        helper_state_expr_name,
        &step_signature.state_ty,
        state_aggregate,
        &mut native_types,
    )? {
        structured_params.push(spec);
    }
    let mut step_ctx = LoweringContext::hoisted()
        .with_native_types(native_types)
        .with_binding_prefix(binding_prefix)
        .with_structured_params(structured_params.into_iter());
    let next_state_value = if let Some(aggregate) = state_aggregate {
        let lowered_fields =
            if let Some(leaf_refs) = flattened_concat_leaf_refs(step_func, next_state_ref)? {
                leaf_refs
                    .iter()
                    .map(|leaf_ref| lower_value(step_func, leaf_ref, &mut step_ctx))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                let lowered_next_state = lower_value(step_func, next_state_ref, &mut step_ctx)?;
                let mut leaves = Vec::new();
                flatten_lowered_value_leaves(&lowered_next_state, &mut leaves)?;
                leaves
            };
        make_structured_value_from_aggregate_fields(
            helper_state_ty,
            aggregate,
            lowered_fields,
            &mut step_ctx.native_types,
        )?
    } else {
        lower_value(step_func, next_state_ref, &mut step_ctx)?
    };
    Ok((step_ctx.bindings, step_ctx.native_types, next_state_value))
}

pub fn export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
    step_func: &ir::Fn,
    projection_func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedNextStateProjectionLustre, PirLustreError> {
    let (step_signature, next_state_ref) = get_step_next_state_node_ref(step_func)?;
    let input_width = flattened_type_width(&step_signature.input_ty, "step input parameter")?;
    let state_width = flattened_type_width(&step_signature.state_ty, "step state parameter")?;

    let (
        next_input_name,
        helper_state_param,
        helper_prev_input_name,
        helper_input_width,
        helper_state_width,
    ) = match projection_func.params.as_slice() {
        [state_param] => (
            None,
            state_param,
            None,
            None,
            flattened_type_width(&state_param.ty, "next state projection state parameter")?,
        ),
        [input_param, state_param] => (
            Some(sanitize_name(&input_param.name)),
            state_param,
            None,
            Some(flattened_type_width(
                &input_param.ty,
                "next state projection input parameter",
            )?),
            flattened_type_width(&state_param.ty, "next state projection state parameter")?,
        ),
        [input_param, state_param, prev_input_param] => {
            let helper_input_width =
                flattened_type_width(&input_param.ty, "next state projection input parameter")?;
            let prev_input_width = flattened_type_width(
                &prev_input_param.ty,
                "next state projection previous input parameter",
            )?;
            if prev_input_width != input_width {
                return Err(PirLustreError::Unsupported(format!(
                    "next state projection function `{}` previous input width {} did not match step input width {}",
                    projection_func.name, prev_input_width, input_width
                )));
            }
            (
                Some(sanitize_name(&input_param.name)),
                state_param,
                Some(sanitize_name(&prev_input_param.name)),
                Some(helper_input_width),
                flattened_type_width(&state_param.ty, "next state projection state parameter")?,
            )
        }
        _ => {
            return Err(PirLustreError::Unsupported(format!(
                "next state projection function `{}` must have 1, 2, or 3 parameters, got {}",
                projection_func.name,
                projection_func.params.len()
            )));
        }
    };
    if helper_state_width != state_width {
        return Err(PirLustreError::Unsupported(format!(
            "next state projection function `{}` state width {} did not match step state width {}",
            projection_func.name, helper_state_width, state_width
        )));
    }
    if let Some(helper_input_width) = helper_input_width
        && helper_input_width != input_width
    {
        return Err(PirLustreError::Unsupported(format!(
            "next state projection function `{}` input width {} did not match step input width {}",
            projection_func.name, helper_input_width, input_width
        )));
    }

    let abstraction_width =
        flattened_type_width(&projection_func.ret_ty, "next state projection return type")?;
    let abstraction_aggregate =
        make_aggregate_bits_spec_from_type(format!("{node_name}_abs_t"), &projection_func.ret_ty)?;
    let ret_node_ref = projection_func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "next state projection function `{}` does not have a return node",
            projection_func.name
        ))
    })?;
    let abstraction_segment_widths = normalize_concat_segment_widths(
        flattened_concat_segment_widths(projection_func, &ret_node_ref)?,
    );

    let input_name = "i".to_string();
    let state_name = "s".to_string();
    let exported_next_input_name = next_input_name.as_ref().map(|_| "i_next".to_string());
    let node_name = sanitize_name(node_name);
    let mut type_defs = Vec::new();
    for aggregate in [
        input_aggregate,
        state_aggregate,
        abstraction_aggregate.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }

    let (step_bindings, step_native_types, next_state_value) = lower_step_next_state_as_helper_value(
        step_func,
        &step_signature,
        &next_state_ref,
        &helper_state_param.ty,
        input_aggregate,
        state_aggregate,
        &input_name,
        &state_name,
        "step",
    )?;

    let mut helper_native_types = step_native_types;
    register_native_type_if_needed(input_aggregate, &step_signature.input_ty, &mut helper_native_types)?;
    register_native_type_if_needed(state_aggregate, &helper_state_param.ty, &mut helper_native_types)?;
    register_native_type_if_needed(
        abstraction_aggregate.as_ref(),
        &projection_func.ret_ty,
        &mut helper_native_types,
    )?;
    if next_input_name.is_some() {
        register_native_type_if_needed(
            input_aggregate,
            &projection_func.params[0].ty,
            &mut helper_native_types,
        )?;
    }
    if helper_prev_input_name.is_some() {
        register_native_type_if_needed(
            input_aggregate,
            &projection_func.params[projection_func.params.len() - 1].ty,
            &mut helper_native_types,
        )?;
    }
    let mut helper_structured_params = vec![StructuredParamSpec {
        param_name: sanitize_name(&helper_state_param.name),
        value: next_state_value,
    }];
    if let Some(next_input_param_name) = next_input_name.as_deref() {
        if let Some(aggregate) = input_aggregate {
            helper_structured_params.push(StructuredParamSpec {
                param_name: next_input_param_name.to_string(),
                value: make_structured_param_value(
                    "i_next",
                    &projection_func.params[0].ty,
                    aggregate,
                    &mut helper_native_types,
                )?,
            });
        }
    }
    if let Some(prev_input_name) = helper_prev_input_name.as_deref()
        && let Some(aggregate) = input_aggregate
    {
        helper_structured_params.push(StructuredParamSpec {
            param_name: prev_input_name.to_string(),
            value: make_structured_param_value(
                &input_name,
                &projection_func.params[projection_func.params.len() - 1].ty,
                aggregate,
                &mut helper_native_types,
            )?,
        });
    }
    if next_input_name.is_some() && input_aggregate.is_none() {
        helper_structured_params.push(StructuredParamSpec {
            param_name: next_input_name.as_ref().unwrap().clone(),
            value: lowered_value_from_native_expr(
                "i_next".to_string(),
                &projection_func.params[0].ty,
                &mut helper_native_types,
            )?,
        });
    }
    if helper_prev_input_name.is_some() && input_aggregate.is_none() {
        helper_structured_params.push(StructuredParamSpec {
            param_name: helper_prev_input_name.as_ref().unwrap().clone(),
            value: lowered_value_from_native_expr(
                input_name.clone(),
                &projection_func.params[projection_func.params.len() - 1].ty,
                &mut helper_native_types,
            )?,
        });
    }

    let mut helper_ctx = LoweringContext::hoisted()
        .with_native_types(helper_native_types)
        .with_binding_prefix("abs")
        .with_structured_params(helper_structured_params.into_iter());
    let abstraction_value = lower_value(projection_func, &ret_node_ref, &mut helper_ctx)?;
    let abstraction_expr = if let Some(aggregate) = &abstraction_aggregate {
        aggregate_constructor_expr_from_value(
            aggregate,
            &abstraction_value,
            ret_node_ref.index,
            &projection_func.name,
        )?
    } else {
        lowered_value_to_native_expr(
            &abstraction_value,
            &projection_func.ret_ty,
            ret_node_ref.index,
            &projection_func.name,
            &mut helper_ctx.native_types,
        )?
    };
    let abstraction_return_ty =
        exported_lustre_type_ref(abstraction_aggregate.as_ref(), &projection_func.ret_ty)?;
    let params_text = if exported_next_input_name.is_some() {
        format!(
            "{input_name}: {}; {state_name}: {}; i_next: {}",
            exported_lustre_type_ref(input_aggregate, &step_signature.input_ty)?,
            exported_lustre_type_ref(state_aggregate, &step_signature.state_ty)?,
            exported_lustre_type_ref(input_aggregate, &step_signature.input_ty)?
        )
    } else {
        format!(
            "{input_name}: {}; {state_name}: {}",
            exported_lustre_type_ref(input_aggregate, &step_signature.input_ty)?,
            exported_lustre_type_ref(state_aggregate, &step_signature.state_ty)?
        )
    };
    for type_def in &helper_ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let mut bindings = step_bindings;
    bindings.extend(helper_ctx.bindings);
    let lustre_text = format!(
        "{type_defs_text}{}",
        render_lustre_node(
            &node_name,
            &params_text,
            &format!("abs: {abstraction_return_ty}"),
            &bindings,
            &[format!("abs = {abstraction_expr};")],
            None,
        )
    );

    Ok(ExportedNextStateProjectionLustre {
        lustre_text,
        type_defs,
        node_name,
        input_name,
        state_name,
        next_input_name: exported_next_input_name,
        input_width,
        state_width,
        abstraction_width,
        abstraction_aggregate,
        abstraction_segment_widths,
    })
}

pub fn export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates(
    step_func: &ir::Fn,
    assumption_func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedNextStateAssumptionLustre, PirLustreError> {
    let (step_signature, next_state_ref) = get_step_next_state_node_ref(step_func)?;
    let input_width = flattened_type_width(&step_signature.input_ty, "step input parameter")?;
    let state_width = flattened_type_width(&step_signature.state_ty, "step state parameter")?;
    let [helper_input_param, helper_state_param, rest @ ..] = assumption_func.params.as_slice()
    else {
        return Err(PirLustreError::Unsupported(format!(
            "next state assumption function `{}` must have 2 or 3 parameters, got {}",
            assumption_func.name,
            assumption_func.params.len()
        )));
    };
    if rest.len() > 1 {
        return Err(PirLustreError::Unsupported(format!(
            "next state assumption function `{}` must have 2 or 3 parameters, got {}",
            assumption_func.name,
            assumption_func.params.len()
        )));
    }
    let helper_input_width = flattened_type_width(
        &helper_input_param.ty,
        "next state assumption input parameter",
    )?;
    let helper_state_width = flattened_type_width(
        &helper_state_param.ty,
        "next state assumption state parameter",
    )?;
    if helper_input_width != input_width {
        return Err(PirLustreError::Unsupported(format!(
            "next state assumption function `{}` input width {} did not match step input width {}",
            assumption_func.name, helper_input_width, input_width
        )));
    }
    if helper_state_width != state_width {
        return Err(PirLustreError::Unsupported(format!(
            "next state assumption function `{}` state width {} did not match step state width {}",
            assumption_func.name, helper_state_width, state_width
        )));
    }
    if let Some(prev_input_param) = rest.first() {
        let prev_input_width = flattened_type_width(
            &prev_input_param.ty,
            "next state assumption previous input parameter",
        )?;
        if prev_input_width != input_width {
            return Err(PirLustreError::Unsupported(format!(
                "next state assumption function `{}` previous input width {} did not match step input width {}",
                assumption_func.name, prev_input_width, input_width
            )));
        }
    }

    let output_width =
        expect_bits_width(&assumption_func.ret_ty, "next state assumption return type")?;
    if output_width != 1 {
        return Err(PirLustreError::Unsupported(format!(
            "next state assumption function `{}` must return bits[1], got {}",
            assumption_func.name, assumption_func.ret_ty
        )));
    }
    let ret_node_ref = assumption_func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "next state assumption function `{}` does not have a return node",
            assumption_func.name
        ))
    })?;

    let input_name = "i".to_string();
    let state_name = "s".to_string();
    let next_input_name = "i_next".to_string();
    let node_name = sanitize_name(node_name);
    let mut type_defs = Vec::new();
    for aggregate in [input_aggregate, state_aggregate].into_iter().flatten() {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }

    let (step_bindings, step_native_types, next_state_value) = lower_step_next_state_as_helper_value(
        step_func,
        &step_signature,
        &next_state_ref,
        &helper_state_param.ty,
        input_aggregate,
        state_aggregate,
        &input_name,
        &state_name,
        "step",
    )?;

    let mut helper_native_types = step_native_types;
    register_native_type_if_needed(input_aggregate, &step_signature.input_ty, &mut helper_native_types)?;
    register_native_type_if_needed(state_aggregate, &helper_state_param.ty, &mut helper_native_types)?;
    register_native_type_if_needed(input_aggregate, &helper_input_param.ty, &mut helper_native_types)?;
    if let Some(prev_input_param) = rest.first() {
        register_native_type_if_needed(
            input_aggregate,
            &prev_input_param.ty,
            &mut helper_native_types,
        )?;
    }
    let mut helper_structured_params = vec![StructuredParamSpec {
        param_name: sanitize_name(&helper_state_param.name),
        value: next_state_value,
    }];
    if let Some(aggregate) = input_aggregate {
        helper_structured_params.push(StructuredParamSpec {
            param_name: sanitize_name(&helper_input_param.name),
            value: make_structured_param_value(
                &next_input_name,
                &helper_input_param.ty,
                aggregate,
                &mut helper_native_types,
            )?,
        });
        if let Some(prev_input_param) = rest.first() {
            helper_structured_params.push(StructuredParamSpec {
                param_name: sanitize_name(&prev_input_param.name),
                value: make_structured_param_value(
                    &input_name,
                    &prev_input_param.ty,
                    aggregate,
                    &mut helper_native_types,
                )?,
            });
        }
    } else {
        helper_structured_params.push(StructuredParamSpec {
            param_name: sanitize_name(&helper_input_param.name),
            value: lowered_value_from_native_expr(
                next_input_name.clone(),
                &helper_input_param.ty,
                &mut helper_native_types,
            )?,
        });
        if let Some(prev_input_param) = rest.first() {
            helper_structured_params.push(StructuredParamSpec {
                param_name: sanitize_name(&prev_input_param.name),
                value: lowered_value_from_native_expr(
                    input_name.clone(),
                    &prev_input_param.ty,
                    &mut helper_native_types,
                )?,
            });
        }
    }

    let mut helper_ctx = LoweringContext::hoisted()
        .with_native_types(helper_native_types)
        .with_binding_prefix("assumption")
        .with_structured_params(helper_structured_params.into_iter());
    let ok_expr = lower_bool_node(assumption_func, &ret_node_ref, &mut helper_ctx)?;
    let params_text = format!(
        "{input_name}: {}; {state_name}: {}; {next_input_name}: {}",
        exported_lustre_type_ref(input_aggregate, &step_signature.input_ty)?,
        exported_lustre_type_ref(state_aggregate, &step_signature.state_ty)?,
        exported_lustre_type_ref(input_aggregate, &step_signature.input_ty)?
    );
    for type_def in &helper_ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let mut bindings = step_bindings;
    bindings.extend(helper_ctx.bindings);
    let lustre_text = format!(
        "{type_defs_text}{}",
        render_lustre_node(
            &node_name,
            &params_text,
            "ok: bool",
            &bindings,
            &[format!("ok = {ok_expr};")],
            None,
        )
    );

    Ok(ExportedNextStateAssumptionLustre {
        lustre_text,
        type_defs,
        node_name,
        input_name,
        state_name,
        next_input_name,
        input_width,
        state_width,
    })
}

pub fn export_pir_fn_to_named_lustre_relation_with_aggregates(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    lhs_state_aggregate: Option<&AggregateBitsSpec>,
    rhs_state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedRelationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_relation_impl(
        func,
        node_name,
        input_aggregate,
        lhs_state_aggregate,
        rhs_state_aggregate,
    )
}

pub fn export_pir_fn_to_named_lustre_relation_with_state_aggregates(
    func: &ir::Fn,
    node_name: &str,
    lhs_state_aggregate: Option<&AggregateBitsSpec>,
    rhs_state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedRelationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_relation_with_aggregates(
        func,
        node_name,
        None,
        lhs_state_aggregate,
        rhs_state_aggregate,
    )
}

pub fn export_pir_fn_to_named_lustre_step(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedStepLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_step_impl(func, node_name, true)
}

pub fn export_pir_fn_to_named_lustre_full_step(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedStepLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_step_impl(func, node_name, false)
}

fn export_pir_fn_to_named_lustre_step_impl(
    func: &ir::Fn,
    node_name: &str,
    include_state_only: bool,
) -> Result<ExportedStepLustre, PirLustreError> {
    let signature = infer_recurrence_signature(func)?;
    let input_width = flattened_type_width(&signature.input_ty, "input parameter")?;
    let state_width = flattened_type_width(&signature.state_ty, "state parameter")?;
    let output_width = flattened_type_width(&signature.output_ty, "output tuple element")?;

    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "top function `{}` does not have a return node",
            func.name
        ))
    })?;
    let ret_node = func.get_node(ret_node_ref);
    let NodePayload::Tuple(return_members) = &ret_node.payload else {
        return Err(PirLustreError::Unsupported(format!(
            "top function `{}` return node is not a tuple",
            func.name
        )));
    };
    if return_members.len() != 2 {
        return Err(PirLustreError::Unsupported(format!(
            "top function `{}` must return a 2-tuple, got {} elements",
            func.name,
            return_members.len()
        )));
    }

    let input_name = sanitize_name(&signature.input_name);
    let state_name = sanitize_name(&signature.state_name);
    let node_name = sanitize_name(node_name);
    let state_only_node_name = sanitize_name(&format!("{node_name}_state_only"));
    let input_aggregate = if type_contains_array(&signature.input_ty) {
        None
    } else {
        make_aggregate_bits_spec_from_type(
            sanitize_name(&format!("{node_name}_input_t")),
            &signature.input_ty,
        )?
    };
    let state_return_ref = return_members[signature.next_state_index];
    let output_return_ref = return_members[signature.output_index];
    let state_segment_widths =
        normalize_concat_segment_widths(flattened_concat_segment_widths(func, &state_return_ref)?);
    let state_aggregate = if type_contains_array(&signature.state_ty) {
        None
    } else if let Some(state_segment_widths) = state_segment_widths.as_deref() {
        make_aggregate_bits_spec(
            sanitize_name(&format!("{node_name}_state_t")),
            state_segment_widths,
        )
    } else {
        make_aggregate_bits_spec_from_type(
            sanitize_name(&format!("{node_name}_state_t")),
            &signature.state_ty,
        )?
    };
    let state_leaf_refs = if state_aggregate.is_some() {
        flattened_concat_leaf_refs(func, &state_return_ref)?
    } else {
        None
    };
    let output_segment_widths =
        normalize_concat_segment_widths(flattened_concat_segment_widths(func, &output_return_ref)?);
    let output_aggregate = if type_contains_array(&signature.output_ty) {
        None
    } else if let Some(output_segment_widths) = output_segment_widths.as_deref() {
        make_aggregate_bits_spec(
            sanitize_name(&format!("{node_name}_output_t")),
            output_segment_widths,
        )
    } else {
        make_aggregate_bits_spec_from_type(
            sanitize_name(&format!("{node_name}_output_t")),
            &signature.output_ty,
        )?
    };
    let output_leaf_refs = if output_aggregate.is_some() {
        flattened_concat_leaf_refs(func, &output_return_ref)?
    } else {
        None
    };
    let input_param_text = format!(
        "{input_name}: {}",
        input_aggregate
            .as_ref()
            .map(|spec| spec.type_name.clone())
            .unwrap_or_else(|| native_lustre_type_ref(&signature.input_ty).unwrap())
    );
    let state_param_text = format!(
        "{state_name}: {}",
        state_aggregate
            .as_ref()
            .map(|spec| spec.type_name.clone())
            .unwrap_or_else(|| native_lustre_type_ref(&signature.state_ty).unwrap())
    );
    let output_return_text = format!(
        "o: {}",
        output_aggregate
            .as_ref()
            .map(|spec| spec.type_name.clone())
            .unwrap_or_else(|| native_lustre_type_ref(&signature.output_ty).unwrap())
    );
    let state_return_text = format!(
        "s_next: {}",
        state_aggregate
            .as_ref()
            .map(|spec| spec.type_name.clone())
            .unwrap_or_else(|| native_lustre_type_ref(&signature.state_ty).unwrap())
    );
    let mut native_types = NativeTypeRegistry::default();
    let mut structured_params = Vec::new();
    if let Some(spec) = make_boundary_param_spec(
        &input_name,
        &signature.input_ty,
        input_aggregate.as_ref(),
        &mut native_types,
    )? {
        structured_params.push(spec);
    }
    if let Some(spec) = make_boundary_param_spec(
        &state_name,
        &signature.state_ty,
        state_aggregate.as_ref(),
        &mut native_types,
    )? {
        structured_params.push(spec);
    }
    let mut type_defs = Vec::new();
    for aggregate in [&input_aggregate, &state_aggregate, &output_aggregate]
        .into_iter()
        .flatten()
    {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }

    let (state_only_lustre, state_only_type_defs) = if include_state_only {
        let mut state_only_ctx = LoweringContext::hoisted()
            .with_native_types(native_types.clone())
            .with_structured_params(structured_params.clone().into_iter());
        let state_only_next_state_expr = if let (Some(spec), Some(leaf_refs)) =
            (state_aggregate.as_ref(), state_leaf_refs.as_ref())
        {
            let lowered_fields = leaf_refs
                .iter()
                .map(|leaf_ref| lower_value(func, leaf_ref, &mut state_only_ctx))
                .collect::<Result<Vec<_>, _>>()?;
            aggregate_constructor_expr(spec, &lowered_fields, state_return_ref.index, &func.name)?
        } else if let Some(spec) = &state_aggregate {
            let lowered_next_state = lower_value(func, &state_return_ref, &mut state_only_ctx)?;
            aggregate_constructor_expr_from_value(
                spec,
                &lowered_next_state,
                state_return_ref.index,
                &func.name,
            )?
        } else {
            let lowered_next_state = lower_value(func, &state_return_ref, &mut state_only_ctx)?;
            lowered_value_to_native_expr(
                &lowered_next_state,
                &signature.state_ty,
                state_return_ref.index,
                &func.name,
                &mut state_only_ctx.native_types,
            )?
        };
        (
            render_lustre_node(
                &state_only_node_name,
                &format!("{input_param_text}; {state_param_text}"),
                &state_return_text,
                &state_only_ctx.bindings,
                &[format!("s_next = {state_only_next_state_expr};")],
                None,
            ),
            state_only_ctx.native_types.defs,
        )
    } else {
        (String::new(), Vec::new())
    };

    let mut step_ctx = LoweringContext::hoisted()
        .with_native_types(native_types)
        .with_structured_params(structured_params.into_iter());
    let output_expr = if let (Some(spec), Some(leaf_refs)) =
        (output_aggregate.as_ref(), output_leaf_refs.as_ref())
    {
        let lowered_fields = leaf_refs
            .iter()
            .map(|leaf_ref| lower_value(func, leaf_ref, &mut step_ctx))
            .collect::<Result<Vec<_>, _>>()?;
        aggregate_constructor_expr(spec, &lowered_fields, output_return_ref.index, &func.name)?
    } else if let Some(spec) = &output_aggregate {
        let lowered_output = lower_value(func, &output_return_ref, &mut step_ctx)?;
        aggregate_constructor_expr_from_value(
            spec,
            &lowered_output,
            output_return_ref.index,
            &func.name,
        )?
    } else {
        let lowered_output = lower_value(func, &output_return_ref, &mut step_ctx)?;
        lowered_value_to_native_expr(
            &lowered_output,
            &signature.output_ty,
            output_return_ref.index,
            &func.name,
            &mut step_ctx.native_types,
        )?
    };
    let next_state_expr = if let (Some(spec), Some(leaf_refs)) =
        (state_aggregate.as_ref(), state_leaf_refs.as_ref())
    {
        let lowered_fields = leaf_refs
            .iter()
            .map(|leaf_ref| lower_value(func, leaf_ref, &mut step_ctx))
            .collect::<Result<Vec<_>, _>>()?;
        aggregate_constructor_expr(spec, &lowered_fields, state_return_ref.index, &func.name)?
    } else if let Some(spec) = &state_aggregate {
        let lowered_next_state = lower_value(func, &state_return_ref, &mut step_ctx)?;
        aggregate_constructor_expr_from_value(
            spec,
            &lowered_next_state,
            state_return_ref.index,
            &func.name,
        )?
    } else {
        let lowered_next_state = lower_value(func, &state_return_ref, &mut step_ctx)?;
        lowered_value_to_native_expr(
            &lowered_next_state,
            &signature.state_ty,
            state_return_ref.index,
            &func.name,
            &mut step_ctx.native_types,
        )?
    };

    let step_lustre = render_lustre_node(
        &node_name,
        &format!("{input_param_text}; {state_param_text}"),
        &format!("{output_return_text}; {state_return_text}; keepalive: bool"),
        &step_ctx.bindings,
        &[
            format!("o = {output_expr};"),
            format!("s_next = {next_state_expr};"),
            "keepalive = true;".to_string(),
        ],
        Some("keepalive"),
    );
    for type_def in state_only_type_defs.iter().chain(step_ctx.native_types.defs.iter()) {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let lustre_text = format!("{type_defs_text}{state_only_lustre}{step_lustre}");

    Ok(ExportedStepLustre {
        lustre_text,
        type_defs,
        state_only_lustre_text: state_only_lustre,
        full_step_lustre_text: step_lustre,
        node_name,
        state_only_node_name,
        input_name,
        state_name,
        input_ty: signature.input_ty.clone(),
        input_width,
        input_aggregate,
        state_ty: signature.state_ty.clone(),
        state_width,
        state_aggregate,
        output_ty: signature.output_ty.clone(),
        output_width,
        output_aggregate,
        output_segment_widths,
    })
}

pub fn export_pir_fn_to_named_lustre_assumption(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedAssumptionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_assumption_impl(func, node_name, None, None)
}

fn export_pir_fn_to_named_lustre_assumption_impl(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedAssumptionLustre, PirLustreError> {
    let [input_param, state_param, rest @ ..] = func.params.as_slice() else {
        return Err(PirLustreError::Unsupported(format!(
            "assumption function `{}` must have 2 or 3 parameters, got {}",
            func.name,
            func.params.len()
        )));
    };
    if rest.len() > 1 {
        return Err(PirLustreError::Unsupported(format!(
            "assumption function `{}` must have 2 or 3 parameters, got {}",
            func.name,
            func.params.len()
        )));
    }

    let input_width = flattened_type_width(&input_param.ty, "assumption input parameter")?;
    let state_width = flattened_type_width(&state_param.ty, "assumption state parameter")?;
    let prev_input_name = rest.first().map(|param| sanitize_name(&param.name));
    if let Some(prev_input_param) = rest.first() {
        let prev_input_width =
            flattened_type_width(&prev_input_param.ty, "assumption previous input parameter")?;
        if prev_input_width != input_width {
            return Err(PirLustreError::Unsupported(format!(
                "assumption function `{}` previous input width {} did not match input width {}",
                func.name, prev_input_width, input_width
            )));
        }
    }

    let output_width = expect_bits_width(&func.ret_ty, "assumption return type")?;
    if output_width != 1 {
        return Err(PirLustreError::Unsupported(format!(
            "assumption function `{}` must return bits[1], got {}",
            func.name, func.ret_ty
        )));
    }

    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "assumption function `{}` does not have a return node",
            func.name
        ))
    })?;

    let input_name = sanitize_name(&input_param.name);
    let state_name = sanitize_name(&state_param.name);
    let node_name = sanitize_name(node_name);
    let input_param_text = format!(
        "{input_name}: {}",
        exported_lustre_type_ref(input_aggregate, &input_param.ty)?
    );
    let state_param_text = format!(
        "{state_name}: {}",
        exported_lustre_type_ref(state_aggregate, &state_param.ty)?
    );
    let prev_input_param_text = prev_input_name.as_deref().map(|prev_input_name| {
        format!(
            "{prev_input_name}: {}",
            exported_lustre_type_ref(input_aggregate, &rest[0].ty)
                .expect("input type was already validated")
        )
    });
    let mut type_defs = Vec::new();
    for aggregate in [input_aggregate, state_aggregate].into_iter().flatten() {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }

    let mut native_types = NativeTypeRegistry::default();
    register_native_type_if_needed(input_aggregate, &input_param.ty, &mut native_types)?;
    register_native_type_if_needed(state_aggregate, &state_param.ty, &mut native_types)?;
    if let Some(prev_input_param) = rest.first() {
        register_native_type_if_needed(input_aggregate, &prev_input_param.ty, &mut native_types)?;
    }
    let mut structured_params = Vec::new();
    if let Some(aggregate) = input_aggregate {
        structured_params.push(make_structured_param_spec(
            &input_name,
            &input_param.ty,
            aggregate,
            &mut native_types,
        )?);
        if let Some(prev_input_param) = rest.first() {
            structured_params.push(make_structured_param_spec(
                prev_input_name.as_deref().unwrap(),
                &prev_input_param.ty,
                aggregate,
                &mut native_types,
            )?);
        }
    }
    if let Some(aggregate) = state_aggregate {
        structured_params.push(make_structured_param_spec(
            &state_name,
            &state_param.ty,
            aggregate,
            &mut native_types,
        )?);
    }
    let mut ctx = LoweringContext::inline()
        .with_native_types(native_types)
        .with_structured_params(structured_params.into_iter());
    let ok_expr = lower_bool_node(func, &ret_node_ref, &mut ctx)?;
    let params_text = match prev_input_name.as_deref() {
        Some(prev_input_name) => {
            format!(
                "{input_param_text}; {state_param_text}; {}",
                prev_input_param_text
                    .unwrap_or_else(|| format!("{prev_input_name}: {}", native_bits_type_ref(input_width).expect("validated input width")))
            )
        }
        None => format!("{input_param_text}; {state_param_text}"),
    };
    for type_def in &ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let lustre_text = format!(
        "{type_defs_text}node {node_name}({params_text}) returns (ok: bool);\nlet\n  ok = {ok_expr};\ntel\n"
    );

    Ok(ExportedAssumptionLustre {
        lustre_text,
        type_defs,
        node_name,
        input_name,
        state_name,
        prev_input_name,
        input_width,
        state_width,
    })
}

pub fn export_pir_fn_to_named_lustre_observation(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedObservationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_observation_impl(func, node_name, None, None, None)
}

fn export_pir_fn_to_named_lustre_observation_impl(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
    output_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedObservationLustre, PirLustreError> {
    let (
        param_layout,
        input_name,
        state_name,
        output_name,
        input_width,
        state_width,
        output_param_width,
    ) = match func.params.as_slice() {
        [output_param] => (
            ObservationParamLayout::OutputOnly,
            None,
            None,
            Some(sanitize_name(&output_param.name)),
            None,
            None,
            Some(flattened_type_width(
                &output_param.ty,
                "observation output parameter",
            )?),
        ),
        [input_param, state_param] => (
            ObservationParamLayout::InputState,
            Some(sanitize_name(&input_param.name)),
            Some(sanitize_name(&state_param.name)),
            None,
            Some(flattened_type_width(
                &input_param.ty,
                "observation input parameter",
            )?),
            Some(flattened_type_width(
                &state_param.ty,
                "observation state parameter",
            )?),
            None,
        ),
        [input_param, state_param, output_param] => (
            ObservationParamLayout::InputStateOutput,
            Some(sanitize_name(&input_param.name)),
            Some(sanitize_name(&state_param.name)),
            Some(sanitize_name(&output_param.name)),
            Some(flattened_type_width(
                &input_param.ty,
                "observation input parameter",
            )?),
            Some(flattened_type_width(
                &state_param.ty,
                "observation state parameter",
            )?),
            Some(flattened_type_width(
                &output_param.ty,
                "observation output parameter",
            )?),
        ),
        _ => {
            return Err(PirLustreError::Unsupported(format!(
                "observation function `{}` must have 1, 2, or 3 parameters, got {}",
                func.name,
                func.params.len()
            )));
        }
    };

    let observation_width = flattened_type_width(&func.ret_ty, "observation return type")?;
    let observation_aggregate =
        make_aggregate_bits_spec_from_type(format!("{node_name}_obs_t"), &func.ret_ty)?;
    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "observation function `{}` does not have a return node",
            func.name
        ))
    })?;

    let node_name = sanitize_name(node_name);
    let mut type_defs = Vec::new();
    for aggregate in [
        input_aggregate,
        state_aggregate,
        output_aggregate,
        observation_aggregate.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }
    let mut native_types = NativeTypeRegistry::default();
    if let Some(input_index) = input_name.as_ref().map(|_| 0usize) {
        register_native_type_if_needed(
            input_aggregate,
            &func.params[input_index].ty,
            &mut native_types,
        )?;
    }
    if let Some(state_index) = state_name.as_ref().map(|_| match param_layout {
        ObservationParamLayout::OutputOnly => unreachable!("no state parameter"),
        ObservationParamLayout::InputState => 1usize,
        ObservationParamLayout::InputStateOutput => 1usize,
    }) {
        register_native_type_if_needed(
            state_aggregate,
            &func.params[state_index].ty,
            &mut native_types,
        )?;
    }
    if let Some(output_index) = output_name.as_ref().map(|_| match param_layout {
        ObservationParamLayout::OutputOnly => 0usize,
        ObservationParamLayout::InputState => unreachable!("no output parameter"),
        ObservationParamLayout::InputStateOutput => 2usize,
    }) {
        register_native_type_if_needed(
            output_aggregate,
            &func.params[output_index].ty,
            &mut native_types,
        )?;
    }
    register_native_type_if_needed(observation_aggregate.as_ref(), &func.ret_ty, &mut native_types)?;
    let mut structured_params = Vec::new();
    match param_layout {
        ObservationParamLayout::OutputOnly => {
            if let Some(aggregate) = output_aggregate {
                structured_params.push(make_structured_param_spec(
                    output_name.as_deref().unwrap(),
                    &func.params[0].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
        }
        ObservationParamLayout::InputState => {
            if let Some(aggregate) = input_aggregate {
                structured_params.push(make_structured_param_spec(
                    input_name.as_deref().unwrap(),
                    &func.params[0].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
            if let Some(aggregate) = state_aggregate {
                structured_params.push(make_structured_param_spec(
                    state_name.as_deref().unwrap(),
                    &func.params[1].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
        }
        ObservationParamLayout::InputStateOutput => {
            if let Some(aggregate) = input_aggregate {
                structured_params.push(make_structured_param_spec(
                    input_name.as_deref().unwrap(),
                    &func.params[0].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
            if let Some(aggregate) = state_aggregate {
                structured_params.push(make_structured_param_spec(
                    state_name.as_deref().unwrap(),
                    &func.params[1].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
            if let Some(aggregate) = output_aggregate {
                structured_params.push(make_structured_param_spec(
                    output_name.as_deref().unwrap(),
                    &func.params[2].ty,
                    aggregate,
                    &mut native_types,
                )?);
            }
        }
    }
    let mut ctx = LoweringContext::inline()
        .with_native_types(native_types)
        .with_structured_params(structured_params.into_iter());
    let observation_value = lower_value(func, &ret_node_ref, &mut ctx)?;
    let observation_expr = if let Some(aggregate) = &observation_aggregate {
        aggregate_constructor_expr_from_value(
            aggregate,
            &observation_value,
            ret_node_ref.index,
            &func.name,
        )?
    } else {
        lowered_value_to_native_expr(
            &observation_value,
            &func.ret_ty,
            ret_node_ref.index,
            &func.name,
            &mut ctx.native_types,
        )?
    };
    let observation_segment_widths =
        normalize_concat_segment_widths(flattened_concat_segment_widths(func, &ret_node_ref)?);
    let observation_return_ty =
        exported_lustre_type_ref(observation_aggregate.as_ref(), &func.ret_ty)?;

    let params_text = match param_layout {
        ObservationParamLayout::OutputOnly => {
            format!(
                "{}: {}",
                output_name.as_deref().unwrap(),
                exported_lustre_type_ref(output_aggregate, &func.params[0].ty)?
            )
        }
        ObservationParamLayout::InputState => format!(
            "{}: {}; {}: {}",
            input_name.as_deref().unwrap(),
            exported_lustre_type_ref(input_aggregate, &func.params[0].ty)?,
            state_name.as_deref().unwrap(),
            exported_lustre_type_ref(state_aggregate, &func.params[1].ty)?
        ),
        ObservationParamLayout::InputStateOutput => format!(
            "{}: {}; {}: {}; {}: {}",
            input_name.as_deref().unwrap(),
            exported_lustre_type_ref(input_aggregate, &func.params[0].ty)?,
            state_name.as_deref().unwrap(),
            exported_lustre_type_ref(state_aggregate, &func.params[1].ty)?,
            output_name.as_deref().unwrap(),
            exported_lustre_type_ref(output_aggregate, &func.params[2].ty)?
        ),
    };
    for type_def in &ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let lustre_text = format!(
        "{type_defs_text}node {node_name}({params_text}) returns (obs: {observation_return_ty});\nlet\n  obs = {observation_expr};\ntel\n"
    );

    Ok(ExportedObservationLustre {
        lustre_text,
        type_defs,
        node_name,
        param_layout,
        input_name,
        state_name,
        output_name,
        input_width,
        state_width,
        output_param_width,
        observation_width,
        observation_aggregate,
        observation_segment_widths,
    })
}

pub fn export_pir_fn_to_named_lustre_state_projection(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedStateProjectionLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_state_projection_impl(func, node_name, None, None)
}

fn export_pir_fn_to_named_lustre_state_projection_impl(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedStateProjectionLustre, PirLustreError> {
    let (param_layout, input_name, state_name, prev_input_name, input_width, state_width) =
        match func.params.as_slice() {
            [state_param] => (
                StateProjectionParamLayout::StateOnly,
                None,
                sanitize_name(&state_param.name),
                None,
                None,
                flattened_type_width(&state_param.ty, "state projection state parameter")?,
            ),
            [input_param, state_param] => (
                StateProjectionParamLayout::InputState,
                Some(sanitize_name(&input_param.name)),
                sanitize_name(&state_param.name),
                None,
                Some(flattened_type_width(
                    &input_param.ty,
                    "state projection input parameter",
                )?),
                flattened_type_width(&state_param.ty, "state projection state parameter")?,
            ),
            [input_param, state_param, prev_input_param] => {
                let input_width =
                    flattened_type_width(&input_param.ty, "state projection input parameter")?;
                let prev_input_width = flattened_type_width(
                    &prev_input_param.ty,
                    "state projection previous input parameter",
                )?;
                if prev_input_width != input_width {
                    return Err(PirLustreError::Unsupported(format!(
                        "state projection function `{}` previous input width {} did not match input width {}",
                        func.name, prev_input_width, input_width
                    )));
                }
                (
                    StateProjectionParamLayout::InputStatePrevInput,
                    Some(sanitize_name(&input_param.name)),
                    sanitize_name(&state_param.name),
                    Some(sanitize_name(&prev_input_param.name)),
                    Some(input_width),
                    flattened_type_width(&state_param.ty, "state projection state parameter")?,
                )
            }
            _ => {
                return Err(PirLustreError::Unsupported(format!(
                    "state projection function `{}` must have 1, 2, or 3 parameters, got {}",
                    func.name,
                    func.params.len()
                )));
            }
        };

    let abstraction_width = flattened_type_width(&func.ret_ty, "state projection return type")?;
    let abstraction_aggregate =
        make_aggregate_bits_spec_from_type(format!("{node_name}_abs_t"), &func.ret_ty)?;
    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "state projection function `{}` does not have a return node",
            func.name
        ))
    })?;

    let node_name = sanitize_name(node_name);
    let mut type_defs = Vec::new();
    for aggregate in [
        input_aggregate,
        state_aggregate,
        abstraction_aggregate.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }
    let mut native_types = NativeTypeRegistry::default();
    if input_name.is_some() {
        let input_param_index = match param_layout {
            StateProjectionParamLayout::StateOnly => unreachable!("no input parameter"),
            StateProjectionParamLayout::InputState => 0usize,
            StateProjectionParamLayout::InputStatePrevInput => 0usize,
        };
        register_native_type_if_needed(
            input_aggregate,
            &func.params[input_param_index].ty,
            &mut native_types,
        )?;
    }
    let state_param_index = match param_layout {
        StateProjectionParamLayout::StateOnly => 0usize,
        StateProjectionParamLayout::InputState => 1usize,
        StateProjectionParamLayout::InputStatePrevInput => 1usize,
    };
    register_native_type_if_needed(
        state_aggregate,
        &func.params[state_param_index].ty,
        &mut native_types,
    )?;
    if prev_input_name.is_some() {
        register_native_type_if_needed(
            input_aggregate,
            &func.params[2].ty,
            &mut native_types,
        )?;
    }
    register_native_type_if_needed(
        abstraction_aggregate.as_ref(),
        &func.ret_ty,
        &mut native_types,
    )?;
    let mut structured_params = Vec::new();
    if let Some(aggregate) = state_aggregate {
        let state_param_index = match param_layout {
            StateProjectionParamLayout::StateOnly => 0,
            StateProjectionParamLayout::InputState => 1,
            StateProjectionParamLayout::InputStatePrevInput => 1,
        };
        structured_params.push(make_structured_param_spec(
            &state_name,
            &func.params[state_param_index].ty,
            aggregate,
            &mut native_types,
        )?);
    }
    if let Some(aggregate) = input_aggregate {
        if let Some(input_name) = input_name.as_deref() {
            structured_params.push(make_structured_param_spec(
                input_name,
                &func.params[0].ty,
                aggregate,
                &mut native_types,
            )?);
        }
        if let Some(prev_input_name) = prev_input_name.as_deref() {
            structured_params.push(make_structured_param_spec(
                prev_input_name,
                &func.params[2].ty,
                aggregate,
                &mut native_types,
            )?);
        }
    }
    let mut ctx = LoweringContext::inline()
        .with_native_types(native_types)
        .with_structured_params(structured_params.into_iter());
    let abstraction_value = lower_value(func, &ret_node_ref, &mut ctx)?;
    let abstraction_expr = if let Some(aggregate) = &abstraction_aggregate {
        aggregate_constructor_expr_from_value(
            aggregate,
            &abstraction_value,
            ret_node_ref.index,
            &func.name,
        )?
    } else {
        lowered_value_to_native_expr(
            &abstraction_value,
            &func.ret_ty,
            ret_node_ref.index,
            &func.name,
            &mut ctx.native_types,
        )?
    };
    let abstraction_segment_widths =
        normalize_concat_segment_widths(flattened_concat_segment_widths(func, &ret_node_ref)?);
    let abstraction_return_ty =
        exported_lustre_type_ref(abstraction_aggregate.as_ref(), &func.ret_ty)?;

    let params_text = match param_layout {
        StateProjectionParamLayout::StateOnly => format!(
            "{state_name}: {}",
            exported_lustre_type_ref(state_aggregate, &func.params[state_param_index].ty)?
        ),
        StateProjectionParamLayout::InputState => {
            format!(
                "{}: {}; {state_name}: {}",
                input_name.as_deref().unwrap(),
                exported_lustre_type_ref(input_aggregate, &func.params[0].ty)?,
                exported_lustre_type_ref(state_aggregate, &func.params[state_param_index].ty)?
            )
        }
        StateProjectionParamLayout::InputStatePrevInput => format!(
            "{}: {}; {state_name}: {}; {}: {}",
            input_name.as_deref().unwrap(),
            exported_lustre_type_ref(input_aggregate, &func.params[0].ty)?,
            exported_lustre_type_ref(state_aggregate, &func.params[state_param_index].ty)?,
            prev_input_name.as_deref().unwrap(),
            exported_lustre_type_ref(input_aggregate, &func.params[2].ty)?
        ),
    };
    for type_def in &ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let lustre_text = format!(
        "{type_defs_text}node {node_name}({params_text}) returns (abs: {abstraction_return_ty});\nlet\n  abs = {abstraction_expr};\ntel\n"
    );

    Ok(ExportedStateProjectionLustre {
        lustre_text,
        type_defs,
        node_name,
        param_layout,
        input_name,
        state_name,
        prev_input_name,
        input_width,
        state_width,
        abstraction_width,
        abstraction_aggregate,
        abstraction_segment_widths,
    })
}

pub fn export_pir_fn_to_named_lustre_relation(
    func: &ir::Fn,
    node_name: &str,
) -> Result<ExportedRelationLustre, PirLustreError> {
    export_pir_fn_to_named_lustre_relation_impl(func, node_name, None, None, None)
}

fn export_pir_fn_to_named_lustre_relation_impl(
    func: &ir::Fn,
    node_name: &str,
    input_aggregate: Option<&AggregateBitsSpec>,
    lhs_state_aggregate: Option<&AggregateBitsSpec>,
    rhs_state_aggregate: Option<&AggregateBitsSpec>,
) -> Result<ExportedRelationLustre, PirLustreError> {
    let (input_name, input_width, lhs_state_param, rhs_state_param, prev_input_name) = match func
        .params
        .as_slice()
    {
        [lhs_state_param, rhs_state_param] => (None, None, lhs_state_param, rhs_state_param, None),
        [input_param, lhs_state_param, rhs_state_param] => (
            Some(sanitize_name(&input_param.name)),
            Some(flattened_type_width(
                &input_param.ty,
                "relation input parameter",
            )?),
            lhs_state_param,
            rhs_state_param,
            None,
        ),
        [
            input_param,
            lhs_state_param,
            rhs_state_param,
            prev_input_param,
        ] => {
            let input_width = flattened_type_width(&input_param.ty, "relation input parameter")?;
            let prev_input_width =
                flattened_type_width(&prev_input_param.ty, "relation previous input parameter")?;
            if prev_input_width != input_width {
                return Err(PirLustreError::Unsupported(format!(
                    "relation function `{}` previous input width {} did not match input width {}",
                    func.name, prev_input_width, input_width
                )));
            }
            (
                Some(sanitize_name(&input_param.name)),
                Some(input_width),
                lhs_state_param,
                rhs_state_param,
                Some(sanitize_name(&prev_input_param.name)),
            )
        }
        _ => {
            return Err(PirLustreError::Unsupported(format!(
                "relation function `{}` must have 2, 3, or 4 parameters, got {}",
                func.name,
                func.params.len()
            )));
        }
    };

    let lhs_state_width =
        flattened_type_width(&lhs_state_param.ty, "relation lhs state parameter")?;
    let rhs_state_width =
        flattened_type_width(&rhs_state_param.ty, "relation rhs state parameter")?;
    let output_width = expect_bits_width(&func.ret_ty, "relation return type")?;
    if output_width != 1 {
        return Err(PirLustreError::Unsupported(format!(
            "relation function `{}` must return bits[1], got {}",
            func.name, func.ret_ty
        )));
    }

    let ret_node_ref = func.ret_node_ref.ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "relation function `{}` does not have a return node",
            func.name
        ))
    })?;

    let lhs_state_name = sanitize_name(&lhs_state_param.name);
    let rhs_state_name = sanitize_name(&rhs_state_param.name);
    let node_name = sanitize_name(node_name);
    let lhs_state_index = if input_name.is_some() { 1usize } else { 0usize };
    let rhs_state_index = lhs_state_index + 1;
    let mut type_defs = Vec::new();
    if let Some(aggregate) = input_aggregate {
        type_defs.push(aggregate.type_def_text());
    }
    if let Some(aggregate) = lhs_state_aggregate {
        let type_def = aggregate.type_def_text();
        if !type_defs.iter().any(|existing| existing == &type_def) {
            type_defs.push(type_def);
        }
    }
    if let Some(aggregate) = rhs_state_aggregate
        && !type_defs
            .iter()
            .any(|existing| existing == &aggregate.type_def_text())
    {
        type_defs.push(aggregate.type_def_text());
    }

    let mut native_types = NativeTypeRegistry::default();
    if input_name.is_some() {
        register_native_type_if_needed(input_aggregate, &func.params[0].ty, &mut native_types)?;
    }
    register_native_type_if_needed(
        lhs_state_aggregate,
        &func.params[lhs_state_index].ty,
        &mut native_types,
    )?;
    register_native_type_if_needed(
        rhs_state_aggregate,
        &func.params[rhs_state_index].ty,
        &mut native_types,
    )?;
    if prev_input_name.is_some() {
        register_native_type_if_needed(
            input_aggregate,
            &func.params[func.params.len() - 1].ty,
            &mut native_types,
        )?;
    }
    let mut structured_params = Vec::new();
    if let Some(aggregate) = input_aggregate
        && let Some(input_name) = input_name.as_deref()
    {
        structured_params.push(make_structured_param_spec(
            input_name,
            &func.params[0].ty,
            aggregate,
            &mut native_types,
        )?);
        if let Some(prev_input_name) = prev_input_name.as_deref() {
            let prev_index = func.params.len() - 1;
            structured_params.push(make_structured_param_spec(
                prev_input_name,
                &func.params[prev_index].ty,
                aggregate,
                &mut native_types,
            )?);
        }
    }
    if let Some(aggregate) = lhs_state_aggregate {
        let lhs_param_index = if input_name.is_some() { 1 } else { 0 };
        structured_params.push(make_structured_param_spec(
            &lhs_state_name,
            &func.params[lhs_param_index].ty,
            aggregate,
            &mut native_types,
        )?);
    }
    if let Some(aggregate) = rhs_state_aggregate {
        let rhs_param_index = if input_name.is_some() { 2 } else { 1 };
        structured_params.push(make_structured_param_spec(
            &rhs_state_name,
            &func.params[rhs_param_index].ty,
            aggregate,
            &mut native_types,
        )?);
    }
    let mut ctx = LoweringContext::inline()
        .with_native_types(native_types)
        .with_structured_params(structured_params.into_iter());
    let ok_expr = lower_bool_node(func, &ret_node_ref, &mut ctx)?;
    let mut params = Vec::new();
    if let Some(input_name) = input_name.as_deref() {
        params.push(format!(
            "{input_name}: {}",
            exported_lustre_type_ref(input_aggregate, &func.params[0].ty)?
        ));
    }
    params.push(format!(
        "{lhs_state_name}: {}",
        exported_lustre_type_ref(lhs_state_aggregate, &func.params[lhs_state_index].ty)?
    ));
    params.push(format!(
        "{rhs_state_name}: {}",
        exported_lustre_type_ref(rhs_state_aggregate, &func.params[rhs_state_index].ty)?
    ));
    if let Some(prev_input_name) = prev_input_name.as_deref() {
        params.push(format!(
            "{prev_input_name}: {}",
            exported_lustre_type_ref(input_aggregate, &func.params[func.params.len() - 1].ty)?
        ));
    }
    let params_text = params.join("; ");
    for type_def in &ctx.native_types.defs {
        if !type_defs.iter().any(|existing| existing == type_def) {
            type_defs.push(type_def.clone());
        }
    }
    let type_defs_text = if type_defs.is_empty() {
        String::new()
    } else {
        format!("{}\n", type_defs.join("\n"))
    };
    let lustre_text = format!(
        "{type_defs_text}node {node_name}({params_text}) returns (ok: bool);\nlet\n  ok = {ok_expr};\ntel\n"
    );

    Ok(ExportedRelationLustre {
        lustre_text,
        type_defs,
        node_name,
        input_name,
        lhs_state_name,
        rhs_state_name,
        prev_input_name,
        input_width,
        lhs_state_width,
        rhs_state_width,
    })
}

fn aggregate_field_access_value(
    param_name: &str,
    spec: &AggregateBitsSpec,
    index: usize,
) -> LoweredValue {
    let field = &spec.fields[index];
    let expr = format!("{param_name}.{}", field.name);
    if field.width == 1 {
        LoweredValue::Bool(expr)
    } else {
        LoweredValue::Bits {
            expr,
            width: field.width,
        }
    }
}

fn aggregate_constructor_expr(
    spec: &AggregateBitsSpec,
    fields: &[LoweredValue],
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    if fields.len() != spec.fields.len() {
        return Err(PirLustreError::Unsupported(format!(
            "aggregate constructor for `{}` expected {} fields, got {}",
            spec.type_name,
            spec.fields.len(),
            fields.len()
        )));
    }
    let assignments = spec
        .fields
        .iter()
        .zip(fields.iter())
        .enumerate()
        .map(|(index, (field, value))| {
            let expr = if field.width == 1 {
                value.clone().into_bool(node_index + index, fn_name)?
            } else {
                value.clone().into_bits(node_index + index, fn_name)?
            };
            Ok(format!("{} = {}", field.name, expr))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    Ok(format!(
        "{} {{ {} }}",
        spec.type_name,
        assignments.join("; ")
    ))
}

fn aggregate_pack_expr(
    fields: &[LoweredValue],
    node_index: usize,
    fn_name: &str,
) -> Result<(String, usize), PirLustreError> {
    let mut operand_exprs = Vec::with_capacity(fields.len());
    let mut widths = Vec::with_capacity(fields.len());
    for (index, field) in fields.iter().enumerate() {
        let width = field.bit_width().ok_or_else(|| {
            PirLustreError::Unsupported(format!(
                "aggregate field {} in node {node_index} of `{fn_name}` did not lower to a scalar width",
                index
            ))
        })?;
        operand_exprs.push(field.clone().into_bits(node_index + index, fn_name)?);
        widths.push(width);
    }
    let total_width = widths.iter().sum();
    let expr = if fields.is_empty() {
        "0".to_string()
    } else if fields.len() == 1 {
        operand_exprs[0].clone()
    } else {
        concat_expr(&operand_exprs, &widths)?
    };
    Ok((expr, total_width))
}

fn flattened_concat_leaf_refs(
    func: &ir::Fn,
    node_ref: &NodeRef,
) -> Result<Option<Vec<NodeRef>>, PirLustreError> {
    let node = func.get_node(*node_ref);
    match &node.payload {
        NodePayload::Nary(NaryOp::Concat, operands) => {
            let mut refs = Vec::new();
            for operand in operands {
                if let Some(nested) = flattened_concat_leaf_refs(func, operand)? {
                    refs.extend(nested);
                } else {
                    refs.push(*operand);
                }
            }
            Ok(Some(refs))
        }
        _ => Ok(None),
    }
}

fn make_aggregate_bits_spec(type_name: String, widths: &[usize]) -> Option<AggregateBitsSpec> {
    if widths.len() <= 1 {
        return None;
    }
    Some(AggregateBitsSpec {
        type_name,
        fields: widths
            .iter()
            .enumerate()
            .map(|(index, width)| AggregateFieldSpec {
                name: format!("f{index}"),
                width: *width,
            })
            .collect(),
    })
}

fn flatten_type_bit_widths(ty: &ir::Type, widths: &mut Vec<usize>) -> Result<(), PirLustreError> {
    match ty {
        ir::Type::Bits(width) => {
            if bits_use_native_chunks(*width) {
                widths.extend(wide_bits_chunk_widths(*width));
            } else {
                widths.push(*width);
            }
            Ok(())
        }
        ir::Type::Tuple(members) => {
            for member in members {
                flatten_type_bit_widths(member, widths)?;
            }
            Ok(())
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            for _ in 0..*element_count {
                flatten_type_bit_widths(element_type, widths)?;
            }
            Ok(())
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected boundary type to be bits, tuple of bits, or array of bits, got {other}"
        ))),
    }
}

fn type_contains_array(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::Bits(_) => false,
        ir::Type::Tuple(members) => members.iter().any(|member| type_contains_array(member)),
        ir::Type::Array(_) => true,
        _ => false,
    }
}

fn make_aggregate_bits_spec_from_type(
    type_name: String,
    ty: &ir::Type,
) -> Result<Option<AggregateBitsSpec>, PirLustreError> {
    let mut widths = Vec::new();
    flatten_type_bit_widths(ty, &mut widths)?;
    Ok(make_aggregate_bits_spec(type_name, &widths))
}

fn lowered_value_from_native_expr(
    expr: String,
    ty: &ir::Type,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => Ok(if *width == 1 {
            LoweredValue::Bool(expr)
        } else if bits_use_native_chunks(*width) {
            native_types.register(ty)?;
            LoweredValue::Composite {
                expr,
                ty: ty.clone(),
            }
        } else {
            LoweredValue::Bits { expr, width: *width }
        }),
        ir::Type::Tuple(_) | ir::Type::Array(_) => {
            native_types.register(ty)?;
            Ok(LoweredValue::Composite {
                expr,
                ty: ty.clone(),
            })
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected native composite type to be bits/tuple/array, got {other}"
        ))),
    }
}

fn lowered_value_to_native_expr(
    value: &LoweredValue,
    ty: &ir::Type,
    node_index: usize,
    fn_name: &str,
    native_types: &mut NativeTypeRegistry,
) -> Result<String, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => {
            if *width == 1 {
                value.clone().into_bool(node_index, fn_name)
            } else if bits_use_native_chunks(*width) {
                native_types.register(ty)?;
                match value {
                    LoweredValue::Composite { expr, ty: value_ty } if value_ty == ty => {
                        Ok(expr.clone())
                    }
                    LoweredValue::Bits { width: value_width, .. }
                        if *value_width == *width =>
                    {
                        Err(PirLustreError::Unsupported(format!(
                            "node {node_index} in `{fn_name}` lowered to scalar bits[{value_width}] where native chunked bits[{width}] were required"
                        )))
                    }
                    LoweredValue::Aggregate { .. }
                    | LoweredValue::Bits { .. }
                    | LoweredValue::Bool(_)
                    | LoweredValue::Tuple(_) => Err(PirLustreError::Unsupported(format!(
                        "node {node_index} in `{fn_name}` lowered to a non-native value where native chunked bits[{width}] were required"
                    ))),
                    LoweredValue::Composite { ty: value_ty, .. } => Err(
                        PirLustreError::Unsupported(format!(
                            "node {node_index} in `{fn_name}` lowered to composite type {value_ty} where chunked bits[{width}] were required"
                        )),
                    ),
                }
            } else {
                value.clone().into_bits(node_index, fn_name)
            }
        }
        ir::Type::Tuple(_members) => {
            native_types.register(ty)?;
            match value {
                LoweredValue::Composite { expr, ty: value_ty } if value_ty == ty => Ok(expr.clone()),
                LoweredValue::Tuple(values) => {
                    native_record_constructor_expr(ty, values, node_index, fn_name, native_types)
                }
                LoweredValue::Aggregate { .. }
                | LoweredValue::Bits { .. }
                | LoweredValue::Bool(_) => Err(PirLustreError::Unsupported(format!(
                    "node {node_index} in `{fn_name}` lowered to a scalar/aggregate where native tuple {ty} was required"
                ))),
                LoweredValue::Composite { ty: value_ty, .. } => Err(PirLustreError::Unsupported(
                    format!(
                        "node {node_index} in `{fn_name}` lowered to composite type {value_ty} where tuple {ty} was required"
                    ),
                )),
            }
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            native_types.register(ty)?;
            match value {
                LoweredValue::Composite { expr, ty: value_ty } if value_ty == ty => Ok(expr.clone()),
                LoweredValue::Tuple(values) => native_array_literal_expr(
                    element_type,
                    &values[..*element_count],
                    node_index,
                    fn_name,
                    native_types,
                ),
                LoweredValue::Aggregate { .. }
                | LoweredValue::Bits { .. }
                | LoweredValue::Bool(_) => Err(PirLustreError::Unsupported(format!(
                    "node {node_index} in `{fn_name}` lowered to a scalar/aggregate where native array {ty} was required"
                ))),
                LoweredValue::Composite { ty: value_ty, .. } => Err(PirLustreError::Unsupported(
                    format!(
                        "node {node_index} in `{fn_name}` lowered to composite type {value_ty} where array {ty} was required"
                    ),
                )),
            }
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected native composite type to be bits/tuple/array, got {other}"
        ))),
    }
}

fn native_record_constructor_expr(
    ty: &ir::Type,
    values: &[LoweredValue],
    node_index: usize,
    fn_name: &str,
    native_types: &mut NativeTypeRegistry,
) -> Result<String, PirLustreError> {
    let ir::Type::Tuple(members) = ty else {
        return Err(PirLustreError::Unsupported(format!(
            "expected tuple type for record constructor, got {ty}"
        )));
    };
    native_types.register(ty)?;
    if values.len() != members.len() {
        return Err(PirLustreError::Unsupported(format!(
            "native tuple constructor in `{fn_name}` expected {} values, got {}",
            members.len(),
            values.len()
        )));
    }
    let assignments = members
        .iter()
        .zip(values.iter())
        .enumerate()
        .map(|(index, (member_ty, value))| {
            Ok(format!(
                "f{index} = {}",
                lowered_value_to_native_expr(value, member_ty, node_index + index, fn_name, native_types)?
            ))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    Ok(format!(
        "{} {{ {} }}",
        native_named_type_name(ty)?,
        assignments.join("; ")
    ))
}

fn native_array_literal_expr(
    element_ty: &ir::Type,
    values: &[LoweredValue],
    node_index: usize,
    fn_name: &str,
    native_types: &mut NativeTypeRegistry,
) -> Result<String, PirLustreError> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            lowered_value_to_native_expr(
                value,
                element_ty,
                node_index + index,
                fn_name,
                native_types,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| format!("[{}]", parts.join(", ")))
}

fn native_wide_bits_constructor_expr(
    bit_count: usize,
    chunk_exprs: &[String],
) -> Result<String, PirLustreError> {
    let ty = ir::Type::Bits(bit_count);
    let chunk_widths = wide_bits_chunk_widths(bit_count);
    if chunk_exprs.len() != chunk_widths.len() {
        return Err(PirLustreError::Unsupported(format!(
            "wide bits[{bit_count}] constructor expected {} chunks, got {}",
            chunk_widths.len(),
            chunk_exprs.len()
        )));
    }
    let assignments = chunk_exprs
        .iter()
        .enumerate()
        .map(|(index, expr)| format!("{} = {}", wide_bits_chunk_field_name(index), expr))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(format!("{} {{ {} }}", native_named_type_name(&ty)?, assignments))
}

fn wide_bits_zero_expr(bit_count: usize) -> Result<String, PirLustreError> {
    let chunk_exprs = wide_bits_chunk_widths(bit_count)
        .into_iter()
        .map(zero_bits_expr)
        .collect::<Vec<_>>();
    native_wide_bits_constructor_expr(bit_count, &chunk_exprs)
}

fn wide_bits_all_ones_expr(bit_count: usize) -> Result<String, PirLustreError> {
    let chunk_exprs = wide_bits_chunk_widths(bit_count)
        .into_iter()
        .map(|chunk_width| {
            Ok::<_, PirLustreError>(scalar_literal_expr(&bit_mask(chunk_width)?, chunk_width))
        })
        .collect::<Result<Vec<_>, _>>()?;
    native_wide_bits_constructor_expr(bit_count, &chunk_exprs)
}

fn wide_bits_chunk_exprs(expr: &str, bit_count: usize) -> Vec<(String, usize)> {
    wide_bits_chunk_widths(bit_count)
        .into_iter()
        .enumerate()
        .map(|(index, chunk_width)| (wide_bits_chunk_access_expr(expr, index), chunk_width))
        .collect()
}

fn native_zero_value(
    ty: &ir::Type,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => Ok(if *width == 1 {
            LoweredValue::Bool("false".to_string())
        } else if bits_use_native_chunks(*width) {
            native_types.register(ty)?;
            LoweredValue::Composite {
                expr: wide_bits_zero_expr(*width)?,
                ty: ty.clone(),
            }
        } else {
            LoweredValue::Bits {
                expr: zero_bits_expr(*width),
                width: *width,
            }
        }),
        ir::Type::Tuple(members) => {
            native_types.register(ty)?;
            let members = members
                .iter()
                .map(|member| native_zero_value(member, native_types))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(LoweredValue::Composite {
                expr: native_record_constructor_expr(ty, &members, 0, "zero", native_types)?,
                ty: ty.clone(),
            })
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            native_types.register(ty)?;
            let elements = (0..*element_count)
                .map(|_| native_zero_value(element_type, native_types))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(LoweredValue::Composite {
                expr: native_array_literal_expr(element_type, &elements, 0, "zero", native_types)?,
                ty: ty.clone(),
            })
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected native composite type to be bits/tuple/array, got {other}"
        ))),
    }
}

fn native_array_index_expr(index_expr: &str, index_width: usize, element_count: usize) -> String {
    let index_int = bits_to_int_expr(index_expr, index_width);
    if element_count <= 1 {
        "0".to_string()
    } else {
        format!(
            "(if ({index_int} < {element_count}) then {index_int} else {})",
            element_count - 1
        )
    }
}

fn lower_value_from_aggregate_type(
    param_name: &str,
    ty: &ir::Type,
    spec: &AggregateBitsSpec,
    next_index: &mut usize,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => {
            let field_index = *next_index;
            let field = spec.fields.get(field_index).ok_or_else(|| {
                PirLustreError::Unsupported(format!(
                    "aggregate `{}` did not contain field index {} for `{param_name}`",
                    spec.type_name, field_index
                ))
            })?;
            if field.width != *width {
                return Err(PirLustreError::Unsupported(format!(
                    "aggregate `{}` field {} width {} did not match expected bits[{width}]",
                    spec.type_name, field.name, field.width
                )));
            }
            *next_index += 1;
            Ok(aggregate_field_access_value(param_name, spec, field_index))
        }
        ir::Type::Tuple(members) => {
            native_types.register(ty)?;
            let mut member_values = Vec::with_capacity(members.len());
            for member in members {
                member_values.push(lower_value_from_aggregate_type(
                    param_name,
                    member,
                    spec,
                    next_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_record_constructor_expr(
                    ty,
                    &member_values,
                    0,
                    "aggregate_type",
                    native_types,
                )?,
                ty: ty.clone(),
            })
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            native_types.register(ty)?;
            let mut element_values = Vec::with_capacity(*element_count);
            for _ in 0..*element_count {
                element_values.push(lower_value_from_aggregate_type(
                    param_name,
                    element_type,
                    spec,
                    next_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_array_literal_expr(
                    element_type,
                    &element_values,
                    0,
                    "aggregate_type",
                    native_types,
                )?,
                ty: ty.clone(),
            })
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected boundary type to be bits, tuple of bits, or array of bits, got {other}"
        ))),
    }
}

fn lower_value_from_aggregate_value(
    ty: &ir::Type,
    spec: &AggregateBitsSpec,
    lowered_fields: &[LoweredValue],
    next_index: &mut usize,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => {
            let start = *next_index;
            let mut total_width = 0usize;
            let mut end = start;
            while total_width < *width {
                let field = spec.fields.get(end).ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "aggregate `{}` ran out of fields while reconstructing bits[{width}]",
                        spec.type_name
                    ))
                })?;
                total_width += field.width;
                end += 1;
            }
            if total_width != *width {
                return Err(PirLustreError::Unsupported(format!(
                    "aggregate `{}` fields starting at {} covered {} bits, expected {}",
                    spec.type_name, start, total_width, width
                )));
            }
            *next_index = end;
            let fields = lowered_fields[start..end].to_vec();
            if fields.len() == 1 {
                Ok(fields[0].clone())
            } else {
                Ok(LoweredValue::Aggregate {
                    spec: AggregateBitsSpec {
                        type_name: spec.type_name.clone(),
                        fields: spec.fields[start..end].to_vec(),
                    },
                    fields,
                })
            }
        }
        ir::Type::Tuple(members) => {
            native_types.register(ty)?;
            let mut member_values = Vec::with_capacity(members.len());
            for member in members {
                member_values.push(lower_value_from_aggregate_value(
                    member,
                    spec,
                    lowered_fields,
                    next_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_record_constructor_expr(
                    ty,
                    &member_values,
                    0,
                    "aggregate_value",
                    native_types,
                )?,
                ty: ty.clone(),
            })
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            native_types.register(ty)?;
            let mut element_values = Vec::with_capacity(*element_count);
            for _ in 0..*element_count {
                element_values.push(lower_value_from_aggregate_value(
                    element_type,
                    spec,
                    lowered_fields,
                    next_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_array_literal_expr(
                    element_type,
                    &element_values,
                    0,
                    "aggregate_value",
                    native_types,
                )?,
                ty: ty.clone(),
            })
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected boundary type to be bits, tuple of bits, or array of bits, got {other}"
        ))),
    }
}

fn make_structured_param_value(
    param_name: &str,
    ty: &ir::Type,
    aggregate: &AggregateBitsSpec,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    if matches!(ty, ir::Type::Bits(_)) {
        Ok(LoweredValue::Aggregate {
            spec: aggregate.clone(),
            fields: aggregate
                .fields
                .iter()
                .enumerate()
                .map(|(index, _)| aggregate_field_access_value(param_name, aggregate, index))
                .collect(),
        })
    } else {
        let mut next_index = 0usize;
        let value = lower_value_from_aggregate_type(
            param_name,
            ty,
            aggregate,
            &mut next_index,
            native_types,
        )?;
        if next_index != aggregate.fields.len() {
            return Err(PirLustreError::Unsupported(format!(
                "aggregate `{}` consumed {} of {} fields for `{param_name}`",
                aggregate.type_name,
                next_index,
                aggregate.fields.len()
            )));
        }
        Ok(value)
    }
}

fn make_structured_param_spec(
    param_name: &str,
    ty: &ir::Type,
    aggregate: &AggregateBitsSpec,
    native_types: &mut NativeTypeRegistry,
) -> Result<StructuredParamSpec, PirLustreError> {
    let value = make_structured_param_value(param_name, ty, aggregate, native_types)?;
    Ok(StructuredParamSpec {
        param_name: param_name.to_string(),
        value,
    })
}

fn make_boundary_param_spec(
    param_name: &str,
    ty: &ir::Type,
    aggregate: Option<&AggregateBitsSpec>,
    native_types: &mut NativeTypeRegistry,
) -> Result<Option<StructuredParamSpec>, PirLustreError> {
    if let Some(aggregate) = aggregate {
        return Ok(Some(make_structured_param_spec(
            param_name,
            ty,
            aggregate,
            native_types,
        )?));
    }
    if matches!(ty, ir::Type::Tuple(_) | ir::Type::Array(_))
        || matches!(ty, ir::Type::Bits(width) if bits_use_native_chunks(*width))
    {
        return Ok(Some(StructuredParamSpec {
            param_name: param_name.to_string(),
            value: lowered_value_from_native_expr(param_name.to_string(), ty, native_types)?,
        }));
    }
    Ok(None)
}

fn make_boundary_param_spec_from_expr_name(
    param_name: &str,
    expr_name: &str,
    ty: &ir::Type,
    aggregate: Option<&AggregateBitsSpec>,
    native_types: &mut NativeTypeRegistry,
) -> Result<Option<StructuredParamSpec>, PirLustreError> {
    if let Some(aggregate) = aggregate {
        return Ok(Some(StructuredParamSpec {
            param_name: param_name.to_string(),
            value: make_structured_param_value(expr_name, ty, aggregate, native_types)?,
        }));
    }
    if matches!(ty, ir::Type::Tuple(_) | ir::Type::Array(_))
        || matches!(ty, ir::Type::Bits(width) if bits_use_native_chunks(*width))
    {
        return Ok(Some(StructuredParamSpec {
            param_name: param_name.to_string(),
            value: lowered_value_from_native_expr(expr_name.to_string(), ty, native_types)?,
        }));
    }
    Ok(None)
}

fn make_structured_value_from_aggregate_fields(
    ty: &ir::Type,
    aggregate: &AggregateBitsSpec,
    lowered_fields: Vec<LoweredValue>,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    if matches!(ty, ir::Type::Bits(_)) {
        return Ok(LoweredValue::Aggregate {
            spec: aggregate.clone(),
            fields: lowered_fields,
        });
    }
    let mut next_index = 0usize;
    let value = lower_value_from_aggregate_value(
        ty,
        aggregate,
        &lowered_fields,
        &mut next_index,
        native_types,
    )?;
    if next_index != aggregate.fields.len() {
        return Err(PirLustreError::Unsupported(format!(
            "aggregate `{}` consumed {} of {} fields while reconstructing structured value",
            aggregate.type_name,
            next_index,
            aggregate.fields.len()
        )));
    }
    Ok(value)
}

fn flatten_lowered_value_leaves(
    value: &LoweredValue,
    out: &mut Vec<LoweredValue>,
) -> Result<(), PirLustreError> {
    match value {
        LoweredValue::Bits { .. } | LoweredValue::Bool(_) => {
            out.push(value.clone());
            Ok(())
        }
        LoweredValue::Composite { expr, ty } => match ty {
            ir::Type::Bits(width) => {
                if bits_use_native_chunks(*width) {
                    for (chunk_expr, chunk_width) in wide_bits_chunk_exprs(expr, *width) {
                        let chunk_value = lowered_value_from_native_expr(
                            chunk_expr,
                            &ir::Type::Bits(chunk_width),
                            &mut NativeTypeRegistry::default(),
                        )?;
                        flatten_lowered_value_leaves(&chunk_value, out)?;
                    }
                    Ok(())
                } else {
                    out.push(value.clone());
                    Ok(())
                }
            }
            ir::Type::Tuple(members) => {
                for (index, member_ty) in members.iter().enumerate() {
                    let field_value = lowered_value_from_native_expr(
                        format!("({expr}).f{index}"),
                        member_ty,
                        &mut NativeTypeRegistry::default(),
                    )?;
                    flatten_lowered_value_leaves(&field_value, out)?;
                }
                Ok(())
            }
            ir::Type::Array(ir::ArrayTypeData {
                element_type,
                element_count,
            }) => {
                for index in 0..*element_count {
                    let element_value = lowered_value_from_native_expr(
                        format!("{expr}[{index}]"),
                        element_type,
                        &mut NativeTypeRegistry::default(),
                    )?;
                    flatten_lowered_value_leaves(&element_value, out)?;
                }
                Ok(())
            }
            other => Err(PirLustreError::Unsupported(format!(
                "expected native composite type to be bits/tuple/array, got {other}"
            ))),
        },
        LoweredValue::Aggregate { fields, .. } | LoweredValue::Tuple(fields) => {
            for field in fields {
                flatten_lowered_value_leaves(field, out)?;
            }
            Ok(())
        }
    }
}

fn aggregate_constructor_expr_from_value(
    spec: &AggregateBitsSpec,
    value: &LoweredValue,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    let mut leaves = Vec::new();
    flatten_lowered_value_leaves(value, &mut leaves)?;
    if leaves.len() != spec.fields.len() {
        return Err(PirLustreError::Unsupported(format!(
            "aggregate constructor for `{}` expected {} leaves, got {}",
            spec.type_name,
            spec.fields.len(),
            leaves.len()
        )));
    }
    aggregate_constructor_expr(spec, &leaves, node_index, fn_name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LoweredValue {
    Bits {
        expr: String,
        width: usize,
    },
    Bool(String),
    Composite {
        expr: String,
        ty: ir::Type,
    },
    Aggregate {
        spec: AggregateBitsSpec,
        fields: Vec<LoweredValue>,
    },
    Tuple(Vec<LoweredValue>),
}

fn lower_bits_node(
    func: &ir::Fn,
    node_ref: &NodeRef,
    ctx: &mut LoweringContext,
) -> Result<String, PirLustreError> {
    lower_value(func, node_ref, ctx)?.into_bits(node_ref.index, &func.name)
}

fn lower_bool_node(
    func: &ir::Fn,
    node_ref: &NodeRef,
    ctx: &mut LoweringContext,
) -> Result<String, PirLustreError> {
    lower_value(func, node_ref, ctx)?.into_bool(node_ref.index, &func.name)
}

fn lower_literal_value(
    ty: &ir::Type,
    value: &xlsynth::IrValue,
    fn_name: &str,
    node_index: usize,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => {
            let bits = value.to_bits()?;
            if *width == 1 {
                let decimal = bits.to_string_fmt(IrFormatPreference::UnsignedDecimal, false);
                match decimal.as_str() {
                    "0" => Ok(LoweredValue::Bool("false".to_string())),
                    "1" => Ok(LoweredValue::Bool("true".to_string())),
                    other => Err(PirLustreError::Unsupported(format!(
                        "bits[1] literal in `{fn_name}` node {node_index} had non-boolean value `{other}`"
                    ))),
                }
            } else if bits_use_native_chunks(*width) {
                native_types.register(ty)?;
                let chunk_exprs = wide_bits_chunk_widths(*width)
                    .into_iter()
                    .scan(0usize, |start, chunk_width| {
                        let chunk_bits = bits.width_slice(*start as i64, chunk_width as i64);
                        *start += chunk_width;
                        Some((
                            chunk_width,
                            chunk_bits.to_string_fmt(IrFormatPreference::UnsignedDecimal, false),
                        ))
                    })
                    .map(|(chunk_width, decimal)| scalar_literal_expr(&decimal, chunk_width))
                    .collect::<Vec<_>>();
                Ok(LoweredValue::Composite {
                    expr: native_wide_bits_constructor_expr(*width, &chunk_exprs)?,
                    ty: ty.clone(),
                })
            } else {
                let decimal = bits.to_string_fmt(IrFormatPreference::UnsignedDecimal, false);
                Ok(LoweredValue::Bits {
                    expr: scalar_literal_expr(&decimal, *width),
                    width: *width,
                })
            }
        }
        ir::Type::Tuple(members) => {
            let element_count = value.get_element_count()?;
            if element_count != members.len() {
                return Err(PirLustreError::Unsupported(format!(
                    "tuple literal in `{fn_name}` node {node_index} had {element_count} elements, expected {}",
                    members.len()
                )));
            }
            let mut lowered = Vec::with_capacity(members.len());
            for (index, member) in members.iter().enumerate() {
                let element = value.get_element(index)?;
                lowered.push(lower_literal_value(
                    member,
                    &element,
                    fn_name,
                    node_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_record_constructor_expr(ty, &lowered, node_index, fn_name, native_types)?,
                ty: ty.clone(),
            })
        }
        ir::Type::Array(ir::ArrayTypeData {
            element_type,
            element_count,
        }) => {
            let actual_count = value.get_element_count()?;
            if actual_count != *element_count {
                return Err(PirLustreError::Unsupported(format!(
                    "array literal in `{fn_name}` node {node_index} had {actual_count} elements, expected {element_count}"
                )));
            }
            let mut lowered = Vec::with_capacity(*element_count);
            for index in 0..*element_count {
                let element = value.get_element(index)?;
                lowered.push(lower_literal_value(
                    element_type,
                    &element,
                    fn_name,
                    node_index,
                    native_types,
                )?);
            }
            Ok(LoweredValue::Composite {
                expr: native_array_literal_expr(
                    element_type,
                    &lowered,
                    node_index,
                    fn_name,
                    native_types,
                )?,
                ty: ty.clone(),
            })
        }
        other => Err(PirLustreError::Unsupported(format!(
            "expected literal node in `{fn_name}` to be bits/tuple/array, got {other}"
        ))),
    }
}

fn lower_value(
    func: &ir::Fn,
    node_ref: &NodeRef,
    ctx: &mut LoweringContext,
) -> Result<LoweredValue, PirLustreError> {
    if let Some(existing) = ctx.cache.get(&node_ref.index) {
        return Ok(existing.clone());
    }

    let node = func.get_node(*node_ref);
    let lowered = match &node.payload {
        NodePayload::GetParam(_) => {
            let param_name = sanitize_name(
                node.name
                    .as_ref()
                    .ok_or_else(|| {
                        PirLustreError::Unsupported(format!(
                            "parameter node {} in `{}` is missing a name",
                            node_ref.index, func.name
                        ))
                    })?
                    .as_str(),
            );
            if let Some(value) = ctx.structured_params.get(&param_name) {
                value.clone()
            } else {
                lowered_value_from_native_expr(
                    param_name,
                    func.get_node_ty(*node_ref),
                    &mut ctx.native_types,
                )?
            }
        }
        NodePayload::Literal(value) => {
            lower_literal_value(
                func.get_node_ty(*node_ref),
                value,
                &func.name,
                node_ref.index,
                &mut ctx.native_types,
            )?
        }
        NodePayload::Tuple(members) => {
            let lowered_members = members
                .iter()
                .map(|member| lower_value(func, member, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            LoweredValue::Composite {
                expr: native_record_constructor_expr(
                    func.get_node_ty(*node_ref),
                    &lowered_members,
                    node_ref.index,
                    &func.name,
                    &mut ctx.native_types,
                )?,
                ty: func.get_node_ty(*node_ref).clone(),
            }
        }
        NodePayload::Array(elements) => {
            let ir::Type::Array(ir::ArrayTypeData { element_type, .. }) = func.get_node_ty(*node_ref)
            else {
                return Err(PirLustreError::Unsupported(format!(
                    "array node {} in `{}` did not have an array type",
                    node_ref.index, func.name
                )));
            };
            let lowered_elements = elements
                .iter()
                .map(|element| lower_value(func, element, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            LoweredValue::Composite {
                expr: native_array_literal_expr(
                    element_type,
                    &lowered_elements,
                    node_ref.index,
                    &func.name,
                    &mut ctx.native_types,
                )?,
                ty: func.get_node_ty(*node_ref).clone(),
            }
        }
        NodePayload::TupleIndex { tuple, index } => {
            let lowered_tuple = lower_value(func, tuple, ctx)?;
            match lowered_tuple {
                LoweredValue::Tuple(values) => values.get(*index).cloned().ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "tuple_index {} was out of bounds for node {} in `{}`",
                        index, tuple.index, func.name
                    ))
                })?,
                LoweredValue::Bits { .. }
                | LoweredValue::Bool(_)
                | LoweredValue::Aggregate { .. } => {
                    return Err(PirLustreError::Unsupported(format!(
                        "tuple_index operand {} in `{}` did not lower to a tuple value",
                        tuple.index, func.name
                    )));
                }
                LoweredValue::Composite { expr, ty } => {
                    let ir::Type::Tuple(members) = &ty else {
                        return Err(PirLustreError::Unsupported(format!(
                            "tuple_index operand {} in `{}` lowered to composite non-tuple type {ty}",
                            tuple.index, func.name
                        )));
                    };
                    let member_ty = members.get(*index).ok_or_else(|| {
                        PirLustreError::Unsupported(format!(
                            "tuple_index {} was out of bounds for node {} in `{}`",
                            index, tuple.index, func.name
                        ))
                    })?;
                    lowered_value_from_native_expr(
                        format!("({expr}).f{index}"),
                        member_ty,
                        &mut ctx.native_types,
                    )?
                }
            }
        }
        NodePayload::ArrayIndex {
            array,
            indices,
            assumed_in_bounds: _,
        } => {
            let mut lowered_array = lower_value(func, array, ctx)?;
            for index_ref in indices {
                let index_expr = lower_bits_node(func, index_ref, ctx)?;
                let index_width =
                    expect_bits_width(func.get_node_ty(*index_ref), "array_index selector")?;
                let values = match lowered_array {
                    LoweredValue::Tuple(values) => values,
                    LoweredValue::Composite { expr, ty } => {
                        let ir::Type::Array(ir::ArrayTypeData {
                            element_type,
                            element_count,
                        }) = &ty
                        else {
                            return Err(PirLustreError::Unsupported(format!(
                                "array_index operand {} in `{}` lowered to composite non-array type {ty}",
                                array.index, func.name
                            )));
                        };
                        lowered_array = lowered_value_from_native_expr(
                            format!(
                                "{expr}[{}]",
                                native_array_index_expr(&index_expr, index_width, *element_count)
                            ),
                            element_type,
                            &mut ctx.native_types,
                        )?;
                        continue;
                    }
                    LoweredValue::Bits { .. }
                    | LoweredValue::Bool(_)
                    | LoweredValue::Aggregate { .. } => {
                        return Err(PirLustreError::Unsupported(format!(
                            "array_index operand {} in `{}` did not lower to an array-like value",
                            array.index, func.name
                        )));
                    }
                };
                if values.is_empty() {
                    return Err(PirLustreError::Unsupported(format!(
                        "array_index operand {} in `{}` lowered to an empty array",
                        array.index, func.name
                    )));
                }
                // XLS array indexing clamps out-of-bounds indices to the final element.
                lowered_array = lower_select_value(&index_expr, &values, None, &func.name)?;
            }
            lowered_array
        }
        NodePayload::Unop(op, arg) => {
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "unary operand")?;
            let result_width = expect_bits_width(func.get_node_ty(*node_ref), "unary result")?;
            let expr = match op {
                Unop::Identity => lower_value(func, arg, ctx)?,
                Unop::Neg => {
                    let arg_expr = lower_bits_node(func, arg, ctx)?;
                    LoweredValue::Bits {
                        expr: wrap_unsigned_expr(
                            &format!("({} - {arg_expr})", zero_bits_expr(result_width)),
                            result_width,
                        )?,
                        width: result_width,
                    }
                }
                Unop::Not if result_width == 1 => {
                    let arg_expr = lower_bool_node(func, arg, ctx)?;
                    LoweredValue::Bool(format!("(not {arg_expr})"))
                }
                Unop::Not => {
                    let arg_value = lower_value(func, arg, ctx)?;
                    if bits_use_native_chunks(result_width) {
                        wide_bitwise_not_value(&arg_value, result_width, node_ref.index, &func.name)?
                    } else {
                        let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bits {
                            expr: bitwise_not_expr(&arg_expr, result_width)?,
                            width: result_width,
                        }
                    }
                }
                Unop::OrReduce => {
                    let arg_value = lower_value(func, arg, ctx)?;
                    if bits_use_native_chunks(arg_width) {
                        let chunk_conditions = lowered_wide_bits_chunks(&arg_value, node_ref.index, &func.name)?
                            .into_iter()
                            .map(|(chunk_expr, chunk_width)| {
                                unsigned_compare_condition(
                                    &chunk_expr,
                                    chunk_width,
                                    &zero_bits_expr(chunk_width),
                                    chunk_width,
                                    "<>",
                                )
                            })
                            .collect::<Vec<_>>();
                        LoweredValue::Bool(combine_bool_terms("or", &chunk_conditions, Some("false")))
                    } else {
                        let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &arg_expr,
                            arg_width,
                            &zero_bits_expr(arg_width),
                            arg_width,
                            "<>",
                        ))
                    }
                }
                Unop::AndReduce => {
                    let arg_value = lower_value(func, arg, ctx)?;
                    if bits_use_native_chunks(arg_width) {
                        let chunk_conditions = lowered_wide_bits_chunks(&arg_value, node_ref.index, &func.name)?
                            .into_iter()
                            .map(|(chunk_expr, chunk_width)| {
                                Ok::<_, PirLustreError>(unsigned_compare_condition(
                                    &chunk_expr,
                                    chunk_width,
                                    &scalar_literal_expr(&bit_mask(chunk_width)?, chunk_width),
                                    chunk_width,
                                    "=",
                                ))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        LoweredValue::Bool(combine_bool_terms("and", &chunk_conditions, Some("true")))
                    } else {
                        let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &arg_expr,
                            arg_width,
                            &scalar_literal_expr(&bit_mask(arg_width)?, arg_width),
                            arg_width,
                            "=",
                        ))
                    }
                }
                Unop::XorReduce => {
                    let conditions = if bits_use_native_chunks(arg_width) {
                        let mut out = Vec::new();
                        for (chunk_expr, chunk_width) in
                            lowered_wide_bits_chunks(&lower_value(func, arg, ctx)?, node_ref.index, &func.name)?
                        {
                            for bit_index in 0..chunk_width {
                                out.push(bit_test_condition(&chunk_expr, chunk_width, bit_index)?);
                            }
                        }
                        out
                    } else {
                        let arg_expr = lower_bits_node(func, arg, ctx)?;
                        (0..arg_width)
                            .map(|bit_index| bit_test_condition(&arg_expr, arg_width, bit_index))
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    LoweredValue::Bool(xor_conditions_expr(&conditions)?)
                }
                other => {
                    return Err(PirLustreError::Unsupported(format!(
                        "unsupported unary operator `{}` in `{}`",
                        ir::unop_to_operator(*other),
                        func.name
                    )));
                }
            };
            if !matches!(op, Unop::OrReduce | Unop::AndReduce | Unop::XorReduce)
                && result_width < arg_width
            {
                return Err(PirLustreError::Unsupported(format!(
                    "unsupported unary width change from {arg_width} to {result_width} in `{}`",
                    func.name
                )));
            }
            expr
        }
        NodePayload::Binop(op, lhs, rhs) => {
            let lhs_width = expect_bits_width(func.get_node_ty(*lhs), "binary lhs")?;
            let rhs_width = expect_bits_width(func.get_node_ty(*rhs), "binary rhs")?;
            let result_width = expect_bits_width(func.get_node_ty(*node_ref), "binary result")?;
            let lhs_value = lower_value(func, lhs, ctx)?;
            let rhs_value = lower_value(func, rhs, ctx)?;
            let lowered = match op {
                Binop::Eq => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "=",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "=",
                        ))
                    }
                }
                Binop::Ne => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "<>",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "<>",
                        ))
                    }
                }
                Binop::Ugt => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            ">",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, ">",
                        ))
                    }
                }
                Binop::Uge => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            ">=",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, ">=",
                        ))
                    }
                }
                Binop::Ult => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "<",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "<",
                        ))
                    }
                }
                Binop::Ule => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_unsigned_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "<=",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(unsigned_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "<=",
                        ))
                    }
                }
                Binop::Sgt => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_signed_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            ">",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(signed_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, ">",
                        )?)
                    }
                }
                Binop::Sge => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_signed_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            ">=",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(signed_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, ">=",
                        )?)
                    }
                }
                Binop::Slt => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_signed_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "<",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(signed_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "<",
                        )?)
                    }
                }
                Binop::Sle => {
                    if std::cmp::max(lhs_width, rhs_width) > 64 {
                        LoweredValue::Bool(wide_signed_compare_condition(
                            &lhs_value,
                            lhs_width,
                            &rhs_value,
                            rhs_width,
                            "<=",
                            node_ref.index,
                            &func.name,
                        )?)
                    } else {
                        let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                        LoweredValue::Bool(signed_compare_condition(
                            &lhs_expr, lhs_width, &rhs_expr, rhs_width, "<=",
                        )?)
                    }
                }
                _ => {
                    let lhs_expr = lhs_value.clone().into_bits(node_ref.index, &func.name)?;
                    let rhs_expr = rhs_value.clone().into_bits(node_ref.index, &func.name)?;
                    LoweredValue::Bits {
                        expr: lower_binop_expr(
                            *op,
                            &lhs_expr,
                            lhs_width,
                            &rhs_expr,
                            rhs_width,
                            result_width,
                        )?,
                        width: result_width,
                    }
                }
            };
            lowered
        }
        NodePayload::ZeroExt { arg, new_bit_count } => {
            let arg_value = lower_value(func, arg, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "zero extension operand")?;
            if *new_bit_count < arg_width {
                return Err(PirLustreError::Unsupported(format!(
                    "zero_ext in `{}` shrank width from {arg_width} to {new_bit_count}",
                    func.name
                )));
            }
            if *new_bit_count == 1 {
                arg_value
            } else {
                LoweredValue::Bits {
                    expr: wrap_unsigned_expr(
                        &cast_bits_expr(
                            &arg_value.into_bits(node_ref.index, &func.name)?,
                            arg_width,
                            *new_bit_count,
                        ),
                        *new_bit_count,
                    )?,
                    width: *new_bit_count,
                }
            }
        }
        NodePayload::SignExt { arg, new_bit_count } => {
            let arg_value = lower_value(func, arg, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "sign extension operand")?;
            if *new_bit_count < arg_width {
                return Err(PirLustreError::Unsupported(format!(
                    "sign_ext in `{}` shrank width from {arg_width} to {new_bit_count}",
                    func.name
                )));
            }
            if *new_bit_count == 1 {
                arg_value
            } else if let LoweredValue::Bool(condition) = arg_value {
                if bits_use_native_chunks(*new_bit_count) {
                    wide_sign_ext_bool_value(&condition, *new_bit_count)?
                } else {
                    LoweredValue::Bits {
                        expr: format!(
                            "(if {condition} then {} else {})",
                            scalar_literal_expr(&bit_mask(*new_bit_count)?, *new_bit_count),
                            zero_bits_expr(*new_bit_count)
                        ),
                        width: *new_bit_count,
                    }
                }
            } else {
                let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                LoweredValue::Bits {
                    expr: wrap_unsigned_expr(
                        &sign_extend_expr(
                            &bits_to_int_expr(&arg_expr, arg_width),
                            arg_width,
                            *new_bit_count,
                        )?,
                        *new_bit_count,
                    )?,
                    width: *new_bit_count,
                }
            }
        }
        NodePayload::BitSlice { arg, start, width } => {
            let arg_value = lower_value(func, arg, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "bit slice operand")?;
            bit_slice_lowered_value(
                &arg_value,
                arg_width,
                *start,
                *width,
                node_ref.index,
                &func.name,
            )?
        }
        NodePayload::DynamicBitSlice { arg, start, width } => {
            let arg_value = lower_value(func, arg, ctx)?;
            let start_expr = lower_bits_node(func, start, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "dynamic bit slice operand")?;
            let start_width =
                expect_bits_width(func.get_node_ty(*start), "dynamic bit slice start")?;
            if *width == 1 && bits_use_native_chunks(arg_width) {
                let chunks =
                    lowered_wide_bits_chunks(&arg_value, node_ref.index, &func.name)?;
                LoweredValue::Bool(wide_dynamic_bit_test_condition(
                    &chunks,
                    &start_expr,
                    start_width,
                    arg_width,
                )?)
            } else if matches!(
                &arg_value,
                LoweredValue::Composite {
                    ty: ir::Type::Bits(width),
                    ..
                } if bits_use_native_chunks(*width)
            ) {
                let arg_expr = match &arg_value {
                    LoweredValue::Composite { expr, .. } => expr.clone(),
                    _ => unreachable!("matched above"),
                };
                if *width == 1 {
                    unreachable!("handled wide width-1 dynamic slices above");
                } else {
                    let start_match_expr = bits_to_int_expr(&start_expr, start_width);
                    let mut out =
                        native_zero_value(&ir::Type::Bits(*width), &mut ctx.native_types)?;
                    for start in (0..arg_width).rev() {
                        let branch = bit_slice_lowered_value(
                            &LoweredValue::Composite {
                                expr: arg_expr.clone(),
                                ty: ir::Type::Bits(arg_width),
                            },
                            arg_width,
                            start,
                            *width,
                            node_ref.index,
                            &func.name,
                        )?;
                        out = lower_boolean_select_value(
                            &format!("({start_match_expr} = {start})"),
                            &[out, branch],
                            None,
                            &func.name,
                        )?;
                    }
                    out
                }
            } else {
                let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                let expr = dynamic_bit_slice_expr(
                    &arg_expr,
                    arg_width,
                    &start_expr,
                    start_width,
                    arg_width,
                    *width,
                )?;
                if *width == 1 {
                    LoweredValue::Bool(unsigned_compare_condition(&expr, *width, "1", 1, "="))
                } else {
                    LoweredValue::Bits {
                        expr,
                        width: *width,
                    }
                }
            }
        }
        NodePayload::BitSliceUpdate {
            arg,
            start,
            update_value,
        } => {
            let arg_value = lower_value(func, arg, ctx)?;
            let start_expr = lower_bits_node(func, start, ctx)?;
            let update_value_ref = *update_value;
            let update_value = lower_value(func, update_value, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "bit slice update operand")?;
            let start_width =
                expect_bits_width(func.get_node_ty(*start), "bit slice update start")?;
            let update_width =
                expect_bits_width(func.get_node_ty(update_value_ref), "bit slice update value")?;
            if bits_use_native_chunks(arg_width) {
                wide_bit_slice_update_value(
                    &arg_value,
                    arg_width,
                    &start_expr,
                    start_width,
                    &update_value,
                    update_width,
                    node_ref.index,
                    &func.name,
                    &mut ctx.native_types,
                )?
            } else {
                let arg_expr = arg_value.into_bits(node_ref.index, &func.name)?;
                let update_expr = update_value.into_bits(node_ref.index, &func.name)?;
                LoweredValue::Bits {
                    expr: bit_slice_update_expr(
                        &arg_expr,
                        arg_width,
                        &start_expr,
                        start_width,
                        arg_width,
                        &update_expr,
                        update_width,
                        update_width,
                    )?,
                    width: arg_width,
                }
            }
        }
        NodePayload::Sel {
            selector,
            cases,
            default,
        } => {
            let selector_width = expect_bits_width(func.get_node_ty(*selector), "sel selector")?;
            let selector_expr = if selector_width == 1 {
                Some(lower_bool_node(func, selector, ctx)?)
            } else {
                None
            };
            let selector_expr = selector_expr.unwrap_or(lower_bits_node(func, selector, ctx)?);
            let cases = cases
                .iter()
                .map(|case| lower_value(func, case, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            let default = default
                .as_ref()
                .map(|node_ref| lower_value(func, node_ref, ctx))
                .transpose()?;
            if selector_width == 1 {
                lower_boolean_select_value(&selector_expr, &cases, default.as_ref(), &func.name)?
            } else {
                lower_select_value(&selector_expr, &cases, default.as_ref(), &func.name)?
            }
        }
        NodePayload::PrioritySel {
            selector,
            cases,
            default,
        } => {
            let selector_ref = *selector;
            let selector = lower_bits_node(func, selector, ctx)?;
            let selector_width =
                expect_bits_width(func.get_node_ty(selector_ref), "priority_sel selector")?;
            let cases = cases
                .iter()
                .map(|case| lower_value(func, case, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            let default = default
                .as_ref()
                .map(|node_ref| lower_value(func, node_ref, ctx))
                .transpose()?;
            lower_priority_select_value(
                &selector,
                selector_width,
                &cases,
                default.as_ref(),
                &func.name,
            )?
        }
        NodePayload::Nary(NaryOp::Concat, operands) => {
            let mut lowered_operands = Vec::with_capacity(operands.len());
            let mut operand_widths = Vec::with_capacity(operands.len());
            for operand in operands {
                lowered_operands.push(lower_value(func, operand, ctx)?);
                operand_widths.push(expect_bits_width(
                    func.get_node_ty(*operand),
                    "concat operand",
                )?);
            }
            concat_lowered_values(
                &lowered_operands,
                &operand_widths,
                operand_widths.iter().sum(),
                node_ref.index,
                &func.name,
            )?
        }
        NodePayload::Nary(op, operands) => {
            let result_width = expect_bits_width(func.get_node_ty(*node_ref), "nary result")?;
            if result_width == 1 {
                let mut lowered_operands = Vec::with_capacity(operands.len());
                for operand in operands {
                    lowered_operands.push(lower_bool_node(func, operand, ctx)?);
                }
                LoweredValue::Bool(bitwise_nary_bool_expr(*op, &lowered_operands)?)
            } else {
                let mut lowered_operands = Vec::with_capacity(operands.len());
                for operand in operands {
                    lowered_operands.push(lower_value(func, operand, ctx)?);
                }
                if bits_use_native_chunks(result_width) {
                    wide_bitwise_nary_value(*op, &lowered_operands, result_width, node_ref.index, &func.name)?
                } else {
                    LoweredValue::Bits {
                        expr: bitwise_nary_expr(
                            *op,
                            &lowered_operands
                                .into_iter()
                                .enumerate()
                                .map(|(index, value)| value.into_bits(node_ref.index + index, &func.name))
                                .collect::<Result<Vec<_>, _>>()?,
                            result_width,
                        )?,
                        width: result_width,
                    }
                }
            }
        }
        NodePayload::OneHotSel { selector, cases } => {
            let selector_expr = lower_bits_node(func, selector, ctx)?;
            let selector_width =
                expect_bits_width(func.get_node_ty(*selector), "one_hot_sel selector")?;
            let case_exprs = cases
                .iter()
                .map(|case| lower_bits_node(func, case, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            let result_width =
                expect_bits_width(func.get_node_ty(*node_ref), "one_hot_sel result")?;
            if result_width == 1 {
                LoweredValue::Bool(one_hot_sel_bool_expr(
                    &selector_expr,
                    selector_width,
                    &case_exprs,
                )?)
            } else {
                LoweredValue::Bits {
                    expr: one_hot_sel_expr(
                        &selector_expr,
                        selector_width,
                        &case_exprs,
                        result_width,
                    )?,
                    width: result_width,
                }
            }
        }
        NodePayload::Encode { arg } => {
            let arg_expr = lower_bits_node(func, arg, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "encode operand")?;
            let result_width = expect_bits_width(func.get_node_ty(*node_ref), "encode result")?;
            if result_width == 1 {
                LoweredValue::Bool(encode_bool_expr(&arg_expr, arg_width)?)
            } else {
                LoweredValue::Bits {
                    expr: encode_expr(&arg_expr, arg_width, result_width)?,
                    width: result_width,
                }
            }
        }
        NodePayload::Decode { arg, width } => {
            let arg_expr = lower_bits_node(func, arg, ctx)?;
            let arg_width = expect_bits_width(func.get_node_ty(*arg), "decode operand")?;
            if bits_use_native_chunks(*width) {
                wide_decode_value(&arg_expr, arg_width, *width)?
            } else {
                LoweredValue::Bits {
                    expr: decode_expr(&arg_expr, arg_width, *width)?,
                    width: *width,
                }
            }
        }
        payload => {
            return Err(PirLustreError::Unsupported(format!(
                "unsupported node payload `{}` in `{}`",
                payload.get_operator(),
                func.name
            )));
        }
    };

    register_lowered_value_native_types(&lowered, &mut ctx.native_types)?;
    let lowered = maybe_hoist_value(ctx, node_ref, node, lowered)?;
    ctx.cache.insert(node_ref.index, lowered.clone());
    Ok(lowered)
}

fn register_lowered_value_native_types(
    value: &LoweredValue,
    native_types: &mut NativeTypeRegistry,
) -> Result<(), PirLustreError> {
    match value {
        LoweredValue::Composite { ty, .. } => native_types.register(ty),
        LoweredValue::Aggregate { fields, .. } | LoweredValue::Tuple(fields) => {
            for field in fields {
                register_lowered_value_native_types(field, native_types)?;
            }
            Ok(())
        }
        LoweredValue::Bits { .. } | LoweredValue::Bool(_) => Ok(()),
    }
}

impl LoweredValue {
    fn into_bits(self, node_index: usize, fn_name: &str) -> Result<String, PirLustreError> {
        match self {
            Self::Bits { expr, .. } => Ok(expr),
            Self::Bool(expr) => Ok(bool_to_bits_expr(&expr)),
            Self::Composite { ty, .. } => Err(PirLustreError::Unsupported(format!(
                "node {node_index} in `{fn_name}` lowered to composite type {ty} where a bits value was required"
            ))),
            Self::Aggregate { fields, .. } => {
                Ok(aggregate_pack_expr(&fields, node_index, fn_name)?.0)
            }
            Self::Tuple(_) => Err(PirLustreError::Unsupported(format!(
                "node {node_index} in `{fn_name}` lowered to a tuple where a bits value was required"
            ))),
        }
    }

    fn into_bool(self, node_index: usize, fn_name: &str) -> Result<String, PirLustreError> {
        match self {
            Self::Bool(expr) => Ok(expr),
            Self::Bits { expr, width } if width == 1 => Ok(format!("({expr} = 1)")),
            Self::Composite { ty, .. } => Err(PirLustreError::Unsupported(format!(
                "node {node_index} in `{fn_name}` lowered to composite type {ty} where a boolean value was required"
            ))),
            Self::Aggregate { fields, spec } if spec.total_width() == 1 && fields.len() == 1 => {
                fields
                    .into_iter()
                    .next()
                    .expect("aggregate width 1 should contain one field")
                    .into_bool(node_index, fn_name)
            }
            Self::Aggregate { fields, .. } => {
                let (expr, width) = aggregate_pack_expr(&fields, node_index, fn_name)?;
                if width == 1 {
                    Ok(format!("({expr} = 1)"))
                } else {
                    Err(PirLustreError::Unsupported(format!(
                        "node {node_index} in `{fn_name}` lowered to aggregate bits[{width}] where a boolean value was required"
                    )))
                }
            }
            Self::Bits { width, .. } => Err(PirLustreError::Unsupported(format!(
                "node {node_index} in `{fn_name}` lowered to bits[{width}] where a boolean value was required"
            ))),
            Self::Tuple(_) => Err(PirLustreError::Unsupported(format!(
                "node {node_index} in `{fn_name}` lowered to a tuple where a boolean value was required"
            ))),
        }
    }

    fn bit_width(&self) -> Option<usize> {
        match self {
            Self::Bits { width, .. } => Some(*width),
            Self::Bool(_) => Some(1),
            Self::Composite { ty, .. } => match ty {
                ir::Type::Bits(width) => Some(*width),
                _ => None,
            },
            Self::Aggregate { spec, .. } => Some(spec.total_width()),
            Self::Tuple(_) => None,
        }
    }
}

fn lowered_wide_bits_chunks(
    value: &LoweredValue,
    node_index: usize,
    fn_name: &str,
) -> Result<Vec<(String, usize)>, PirLustreError> {
    match value {
        LoweredValue::Composite {
            expr,
            ty: ir::Type::Bits(width),
        } if bits_use_native_chunks(*width) => Ok(wide_bits_chunk_exprs(expr, *width)),
        LoweredValue::Aggregate { spec, fields } if bits_use_native_chunks(spec.total_width()) => {
            let expected_widths = wide_bits_chunk_widths(spec.total_width());
            if fields.len() != expected_widths.len() {
                return Err(PirLustreError::Unsupported(format!(
                    "node {node_index} in `{fn_name}` lowered to aggregate `{}` with {} fields; expected {} native chunk fields for bits[{}]",
                    spec.type_name,
                    fields.len(),
                    expected_widths.len(),
                    spec.total_width()
                )));
            }
            fields
                .iter()
                .zip(expected_widths.into_iter())
                .enumerate()
                .map(|(index, (field, expected_width))| {
                    let actual_width = field.bit_width().ok_or_else(|| {
                        PirLustreError::Unsupported(format!(
                            "aggregate field {index} in node {node_index} of `{fn_name}` did not lower to a scalar width"
                        ))
                    })?;
                    if actual_width != expected_width {
                        return Err(PirLustreError::Unsupported(format!(
                            "aggregate field {index} in node {node_index} of `{fn_name}` had width {actual_width}; expected native chunk width {expected_width}"
                        )));
                    }
                    Ok((field.clone().into_bits(node_index + index, fn_name)?, actual_width))
                })
                .collect()
        }
        LoweredValue::Composite { ty, .. } => Err(PirLustreError::Unsupported(format!(
            "node {node_index} in `{fn_name}` lowered to composite type {ty} where chunked wide bits were required"
        ))),
        other => Err(PirLustreError::Unsupported(format!(
            "node {node_index} in `{fn_name}` lowered to {other:?} where chunked wide bits were required"
        ))),
    }
}

fn wide_bits_from_chunk_exprs(
    bit_count: usize,
    chunk_exprs: &[String],
) -> Result<LoweredValue, PirLustreError> {
    Ok(LoweredValue::Composite {
        expr: native_wide_bits_constructor_expr(bit_count, chunk_exprs)?,
        ty: ir::Type::Bits(bit_count),
    })
}

fn bit_slice_lowered_value(
    value: &LoweredValue,
    expr_width: usize,
    start: usize,
    width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    if width == 0 {
        return Ok(LoweredValue::Bits {
            expr: zero_bits_expr(0),
            width: 0,
        });
    }
    if expr_width == 1 && start == 0 && width == 1 {
        return value
            .clone()
            .into_bool(node_index, fn_name)
            .map(LoweredValue::Bool);
    }
    if let LoweredValue::Aggregate { fields, .. } = value {
        return aggregate_bit_slice(fields, start, width, node_index, fn_name);
    }
    if matches!(
        value,
        LoweredValue::Composite {
            ty: ir::Type::Bits(bit_count),
            ..
        } if bits_use_native_chunks(*bit_count)
    ) {
        let expr = match value {
            LoweredValue::Composite { expr, .. } => expr,
            _ => unreachable!("matched composite wide bits above"),
        };
        return wide_bit_slice_value(expr, expr_width, start, width, node_index, fn_name);
    }
    let expr = value.clone().into_bits(node_index, fn_name)?;
    if width == 1 {
        Ok(LoweredValue::Bool(bit_test_condition(
            &expr, expr_width, start,
        )?))
    } else {
        Ok(LoweredValue::Bits {
            expr: bit_slice_expr(&expr, expr_width, start, width)?,
            width,
        })
    }
}

fn concat_lowered_values(
    values: &[LoweredValue],
    widths: &[usize],
    target_width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    assert_eq!(values.len(), widths.len());
    if target_width == 0 {
        return Ok(LoweredValue::Bits {
            expr: zero_bits_expr(0),
            width: 0,
        });
    }
    if bits_use_native_chunks(target_width) {
        let mut ranges = Vec::with_capacity(values.len());
        let mut lsb = 0usize;
        for (value, width) in values.iter().zip(widths.iter()).rev() {
            ranges.push((lsb, lsb + *width, value, *width));
            lsb += *width;
        }
        let mut chunk_exprs = Vec::new();
        let mut chunk_lsb = 0usize;
        for chunk_width in wide_bits_chunk_widths(target_width) {
            let chunk_end = chunk_lsb + chunk_width;
            if chunk_width == 1 {
                let mut chunk_expr = None;
                for (value_lsb, value_end, value, value_width) in ranges.iter().rev() {
                    let part_start = chunk_lsb.max(*value_lsb);
                    let part_end = chunk_end.min(*value_end);
                    if part_start >= part_end {
                        continue;
                    }
                    let slice = bit_slice_lowered_value(
                        value,
                        *value_width,
                        part_start - *value_lsb,
                        1,
                        node_index,
                        fn_name,
                    )?;
                    chunk_expr = Some(slice.into_bool(node_index, fn_name)?);
                    break;
                }
                chunk_exprs.push(chunk_expr.unwrap_or_else(|| "false".to_string()));
            } else {
                let mut part_exprs = Vec::new();
                let mut part_widths = Vec::new();
                for (value_lsb, value_end, value, value_width) in ranges.iter().rev() {
                    let part_start = chunk_lsb.max(*value_lsb);
                    let part_end = chunk_end.min(*value_end);
                    if part_start >= part_end {
                        continue;
                    }
                    let slice_width = part_end - part_start;
                    let slice = bit_slice_lowered_value(
                        value,
                        *value_width,
                        part_start - *value_lsb,
                        slice_width,
                        node_index,
                        fn_name,
                    )?;
                    part_exprs.push(slice.into_bits(node_index, fn_name)?);
                    part_widths.push(slice_width);
                }
                let chunk_expr = if part_exprs.len() == 1 && part_widths[0] == chunk_width {
                    part_exprs.pop().expect("single part must exist")
                } else {
                    concat_expr(&part_exprs, &part_widths)?
                };
                chunk_exprs.push(chunk_expr);
            }
            chunk_lsb = chunk_end;
        }
        wide_bits_from_chunk_exprs(target_width, &chunk_exprs)
    } else if target_width == 1 {
        values
            .first()
            .cloned()
            .ok_or_else(|| {
                PirLustreError::Unsupported(format!(
                    "concat in `{fn_name}` expected at least one operand"
                ))
            })?
            .into_bool(node_index, fn_name)
            .map(LoweredValue::Bool)
    } else {
        let exprs = values
            .iter()
            .enumerate()
            .map(|(index, value)| value.clone().into_bits(node_index + index, fn_name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LoweredValue::Bits {
            expr: concat_expr(&exprs, widths)?,
            width: target_width,
        })
    }
}

fn chunk_fill_native_expr(
    chunk_width: usize,
    sign_condition: Option<&str>,
) -> Result<String, PirLustreError> {
    if chunk_width == 1 {
        Ok(sign_condition.unwrap_or("false").to_string())
    } else if let Some(condition) = sign_condition {
        Ok(format!(
            "(if {condition} then {} else {})",
            scalar_literal_expr(&bit_mask(chunk_width)?, chunk_width),
            zero_bits_expr(chunk_width)
        ))
    } else {
        Ok(zero_bits_expr(chunk_width))
    }
}

fn chunk_fill_bits_expr(
    chunk_width: usize,
    sign_condition: Option<&str>,
) -> Result<String, PirLustreError> {
    if chunk_width == 1 {
        Ok(match sign_condition {
            Some(condition) => bool_to_bits_expr(condition),
            None => "0".to_string(),
        })
    } else {
        chunk_fill_native_expr(chunk_width, sign_condition)
    }
}

fn slice_to_native_expr(
    value: &LoweredValue,
    expr_width: usize,
    start: usize,
    width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    let slice = bit_slice_lowered_value(value, expr_width, start, width, node_index, fn_name)?;
    if width == 1 {
        slice.into_bool(node_index, fn_name)
    } else {
        slice.into_bits(node_index, fn_name)
    }
}

fn slice_to_bits_expr(
    value: &LoweredValue,
    expr_width: usize,
    start: usize,
    width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    bit_slice_lowered_value(value, expr_width, start, width, node_index, fn_name)?
        .into_bits(node_index, fn_name)
}

fn wide_chunk_native_expr(
    value: &LoweredValue,
    source_width: usize,
    chunk_start: usize,
    chunk_width: usize,
    sign_condition: Option<&str>,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    if chunk_start >= source_width {
        return chunk_fill_native_expr(chunk_width, sign_condition);
    }
    let available_width = std::cmp::min(chunk_width, source_width - chunk_start);
    if available_width == chunk_width {
        return slice_to_native_expr(
            value,
            source_width,
            chunk_start,
            chunk_width,
            node_index,
            fn_name,
        );
    }
    let high_fill_width = chunk_width - available_width;
    let high_fill = chunk_fill_bits_expr(high_fill_width, sign_condition)?;
    let low_bits = slice_to_bits_expr(
        value,
        source_width,
        chunk_start,
        available_width,
        node_index,
        fn_name,
    )?;
    concat_expr(&[high_fill, low_bits], &[high_fill_width, available_width])
}

fn wide_compare_chunk_conditions(
    lhs_chunks: &[(String, usize)],
    rhs_chunks: &[(String, usize)],
) -> Result<(String, String, String), PirLustreError> {
    let eq_terms = lhs_chunks
        .iter()
        .zip(rhs_chunks.iter())
        .map(|((lhs_expr, width), (rhs_expr, _))| {
            let lhs_bits = if *width == 1 {
                bool_to_bits_expr(lhs_expr)
            } else {
                lhs_expr.clone()
            };
            let rhs_bits = if *width == 1 {
                bool_to_bits_expr(rhs_expr)
            } else {
                rhs_expr.clone()
            };
            unsigned_compare_condition(&lhs_bits, *width, &rhs_bits, *width, "=")
        })
        .collect::<Vec<_>>();
    let eq = combine_bool_terms("and", &eq_terms, Some("true"));
    let mut prefix_eq = "true".to_string();
    let mut lt_terms = Vec::new();
    let mut gt_terms = Vec::new();
    for ((lhs_expr, width), (rhs_expr, _)) in lhs_chunks.iter().zip(rhs_chunks.iter()).rev() {
        let lhs_bits = if *width == 1 {
            bool_to_bits_expr(lhs_expr)
        } else {
            lhs_expr.clone()
        };
        let rhs_bits = if *width == 1 {
            bool_to_bits_expr(rhs_expr)
        } else {
            rhs_expr.clone()
        };
        lt_terms.push(format!(
            "({prefix_eq} and {})",
            unsigned_compare_condition(&lhs_bits, *width, &rhs_bits, *width, "<")
        ));
        gt_terms.push(format!(
            "({prefix_eq} and {})",
            unsigned_compare_condition(&lhs_bits, *width, &rhs_bits, *width, ">")
        ));
        prefix_eq = format!(
            "({prefix_eq} and {})",
            unsigned_compare_condition(&lhs_bits, *width, &rhs_bits, *width, "=")
        );
    }
    Ok((
        eq,
        combine_bool_terms("or", &lt_terms, Some("false")),
        combine_bool_terms("or", &gt_terms, Some("false")),
    ))
}

fn wide_unsigned_compare_condition(
    lhs_value: &LoweredValue,
    lhs_width: usize,
    rhs_value: &LoweredValue,
    rhs_width: usize,
    operator: &str,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    let compare_width = std::cmp::max(lhs_width, rhs_width);
    let mut lhs_chunks = Vec::new();
    let mut rhs_chunks = Vec::new();
    let mut chunk_start = 0usize;
    for chunk_width in wide_bits_chunk_widths(compare_width) {
        lhs_chunks.push((
            wide_chunk_native_expr(
                lhs_value,
                lhs_width,
                chunk_start,
                chunk_width,
                None,
                node_index,
                fn_name,
            )?,
            chunk_width,
        ));
        rhs_chunks.push((
            wide_chunk_native_expr(
                rhs_value,
                rhs_width,
                chunk_start,
                chunk_width,
                None,
                node_index,
                fn_name,
            )?,
            chunk_width,
        ));
        chunk_start += chunk_width;
    }
    let (eq, lt, gt) = wide_compare_chunk_conditions(&lhs_chunks, &rhs_chunks)?;
    Ok(match operator {
        "=" => eq,
        "<>" => format!("(not {eq})"),
        "<" => lt,
        "<=" => format!("({lt} or {eq})"),
        ">" => gt,
        ">=" => format!("({gt} or {eq})"),
        other => {
            return Err(PirLustreError::Unsupported(format!(
                "unsupported wide unsigned comparison operator `{other}`"
            )))
        }
    })
}

fn sign_flip_chunk_native_expr(expr: &str, chunk_width: usize) -> Result<String, PirLustreError> {
    if chunk_width == 1 {
        Ok(format!("(not {expr})"))
    } else {
        let sign_bit = scalar_literal_expr(&bit_modulus(chunk_width - 1)?, chunk_width);
        wrap_unsigned_expr(&format!("({expr} + {sign_bit})"), chunk_width)
    }
}

fn wide_signed_compare_condition(
    lhs_value: &LoweredValue,
    lhs_width: usize,
    rhs_value: &LoweredValue,
    rhs_width: usize,
    operator: &str,
    node_index: usize,
    fn_name: &str,
) -> Result<String, PirLustreError> {
    let compare_width = std::cmp::max(lhs_width, rhs_width);
    let lhs_sign = if lhs_width == 0 {
        "false".to_string()
    } else {
        bit_slice_lowered_value(lhs_value, lhs_width, lhs_width - 1, 1, node_index, fn_name)?
            .into_bool(node_index, fn_name)?
    };
    let rhs_sign = if rhs_width == 0 {
        "false".to_string()
    } else {
        bit_slice_lowered_value(rhs_value, rhs_width, rhs_width - 1, 1, node_index, fn_name)?
            .into_bool(node_index, fn_name)?
    };
    let mut lhs_chunks = Vec::new();
    let mut rhs_chunks = Vec::new();
    let chunk_widths = wide_bits_chunk_widths(compare_width);
    let mut chunk_start = 0usize;
    for (chunk_index, chunk_width) in chunk_widths.iter().copied().enumerate() {
        let mut lhs_expr = wide_chunk_native_expr(
            lhs_value,
            lhs_width,
            chunk_start,
            chunk_width,
            Some(&lhs_sign),
            node_index,
            fn_name,
        )?;
        let mut rhs_expr = wide_chunk_native_expr(
            rhs_value,
            rhs_width,
            chunk_start,
            chunk_width,
            Some(&rhs_sign),
            node_index,
            fn_name,
        )?;
        if chunk_index + 1 == chunk_widths.len() {
            lhs_expr = sign_flip_chunk_native_expr(&lhs_expr, chunk_width)?;
            rhs_expr = sign_flip_chunk_native_expr(&rhs_expr, chunk_width)?;
        }
        lhs_chunks.push((lhs_expr, chunk_width));
        rhs_chunks.push((rhs_expr, chunk_width));
        chunk_start += chunk_width;
    }
    let (eq, lt, gt) = wide_compare_chunk_conditions(&lhs_chunks, &rhs_chunks)?;
    Ok(match operator {
        "=" => eq,
        "<>" => format!("(not {eq})"),
        "<" => lt,
        "<=" => format!("({lt} or {eq})"),
        ">" => gt,
        ">=" => format!("({gt} or {eq})"),
        other => {
            return Err(PirLustreError::Unsupported(format!(
                "unsupported wide signed comparison operator `{other}`"
            )))
        }
    })
}

fn wide_bit_slice_update_value(
    arg_value: &LoweredValue,
    arg_width: usize,
    start_expr: &str,
    start_width: usize,
    update_value: &LoweredValue,
    update_width: usize,
    node_index: usize,
    fn_name: &str,
    native_types: &mut NativeTypeRegistry,
) -> Result<LoweredValue, PirLustreError> {
    for bit_count in 65..=std::cmp::max(arg_width, update_width) {
        if bits_use_native_chunks(bit_count) {
            native_types.register(&ir::Type::Bits(bit_count))?;
        }
    }
    let start_int = bits_to_int_expr(start_expr, start_width);
    let mut chunk_exprs = Vec::new();
    let mut chunk_start = 0usize;
    for chunk_width in wide_bits_chunk_widths(arg_width) {
        chunk_exprs.push(wide_chunk_native_expr(
            arg_value,
            arg_width,
            chunk_start,
            chunk_width,
            None,
            node_index,
            fn_name,
        )?);
        chunk_start += chunk_width;
    }
    let mut out = wide_bits_from_chunk_exprs(arg_width, &chunk_exprs)?;
    for start in (0..arg_width).rev() {
        let replace_width = std::cmp::min(update_width, arg_width - start);
        let high_width = arg_width - (start + replace_width);
        let mut parts = Vec::new();
        let mut part_widths = Vec::new();
        if high_width > 0 {
            parts.push(bit_slice_lowered_value(
                arg_value,
                arg_width,
                start + replace_width,
                high_width,
                node_index,
                fn_name,
            )?);
            part_widths.push(high_width);
        }
        if replace_width > 0 {
            parts.push(bit_slice_lowered_value(
                update_value,
                update_width,
                0,
                replace_width,
                node_index,
                fn_name,
            )?);
            part_widths.push(replace_width);
        }
        if start > 0 {
            parts.push(bit_slice_lowered_value(
                arg_value,
                arg_width,
                0,
                start,
                node_index,
                fn_name,
            )?);
            part_widths.push(start);
        }
        let branch = concat_lowered_values(&parts, &part_widths, arg_width, node_index, fn_name)?;
        out = lower_boolean_select_value(
            &format!("({start_int} = {start})"),
            &[out, branch],
            None,
            fn_name,
        )?;
    }
    Ok(out)
}

fn wide_bitwise_not_value(
    arg_value: &LoweredValue,
    bit_count: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    let chunk_exprs = lowered_wide_bits_chunks(arg_value, node_index, fn_name)?
        .into_iter()
        .map(|(chunk_expr, chunk_width)| bitwise_not_expr(&chunk_expr, chunk_width))
        .collect::<Result<Vec<_>, _>>()?;
    wide_bits_from_chunk_exprs(bit_count, &chunk_exprs)
}

fn wide_bitwise_nary_value(
    op: NaryOp,
    operands: &[LoweredValue],
    bit_count: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    let chunk_widths = wide_bits_chunk_widths(bit_count);
    let operand_chunks = operands
        .iter()
        .map(|operand| lowered_wide_bits_chunks(operand, node_index, fn_name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut chunk_exprs = Vec::with_capacity(chunk_widths.len());
    for (chunk_index, chunk_width) in chunk_widths.into_iter().enumerate() {
        let chunk_operands = operand_chunks
            .iter()
            .map(|chunks| chunks[chunk_index].0.clone())
            .collect::<Vec<_>>();
        chunk_exprs.push(bitwise_nary_expr(op, &chunk_operands, chunk_width)?);
    }
    wide_bits_from_chunk_exprs(bit_count, &chunk_exprs)
}

fn wide_sign_ext_bool_value(
    condition: &str,
    bit_count: usize,
) -> Result<LoweredValue, PirLustreError> {
    Ok(LoweredValue::Composite {
        expr: format!(
            "(if {condition} then {} else {})",
            wide_bits_all_ones_expr(bit_count)?,
            wide_bits_zero_expr(bit_count)?
        ),
        ty: ir::Type::Bits(bit_count),
    })
}

fn wide_decode_value(
    arg_expr: &str,
    arg_width: usize,
    result_width: usize,
) -> Result<LoweredValue, PirLustreError> {
    let zero = wide_bits_zero_expr(result_width)?;
    let limit = scalar_literal_expr(&result_width.to_string(), arg_width);
    let in_range = unsigned_compare_condition(arg_expr, arg_width, &limit, arg_width, "<");
    let mut chunk_exprs = Vec::new();
    let mut chunk_start = 0usize;
    for chunk_width in wide_bits_chunk_widths(result_width) {
        let chunk_limit = scalar_literal_expr(&(chunk_start + chunk_width).to_string(), arg_width);
        let chunk_start_lit = scalar_literal_expr(&chunk_start.to_string(), arg_width);
        let in_chunk = if chunk_start == 0 {
            unsigned_compare_condition(arg_expr, arg_width, &chunk_limit, arg_width, "<")
        } else {
            format!(
                "({} and {})",
                unsigned_compare_condition(arg_expr, arg_width, &chunk_start_lit, arg_width, ">="),
                unsigned_compare_condition(arg_expr, arg_width, &chunk_limit, arg_width, "<")
            )
        };
        let local_shift = if chunk_start == 0 {
            cast_bits_expr(arg_expr, arg_width, chunk_width)
        } else {
            cast_bits_expr(
                &format!("({arg_expr} - {chunk_start_lit})"),
                arg_width,
                chunk_width,
            )
        };
        let one = scalar_literal_expr("1", chunk_width);
        let shifted = wrap_unsigned_expr(&format!("({one} lsh {local_shift})"), chunk_width)?;
        chunk_exprs.push(format!("(if {in_chunk} then {shifted} else {})", zero_bits_expr(chunk_width)));
        chunk_start += chunk_width;
    }
    Ok(LoweredValue::Composite {
        expr: format!(
            "(if {in_range} then {} else {zero})",
            native_wide_bits_constructor_expr(result_width, &chunk_exprs)?
        ),
        ty: ir::Type::Bits(result_width),
    })
}

fn lower_binop_expr(
    op: Binop,
    lhs: &str,
    lhs_width: usize,
    rhs: &str,
    rhs_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    let lhs_for_result = cast_bits_expr(lhs, lhs_width, result_width);
    let rhs_for_result = cast_bits_expr(rhs, rhs_width, result_width);
    match op {
        Binop::Add => wrap_unsigned_expr(
            &format!("({lhs_for_result} + {rhs_for_result})"),
            result_width,
        ),
        Binop::Sub => wrap_unsigned_expr(
            &format!("({lhs_for_result} - {rhs_for_result})"),
            result_width,
        ),
        Binop::Shll => shift_left_expr(lhs, rhs, lhs_width, rhs_width, result_width),
        Binop::Shrl => shift_right_logical_expr(lhs, rhs, lhs_width, rhs_width, result_width),
        Binop::Shra => shift_right_arithmetic_expr(lhs, rhs, lhs_width, rhs_width, result_width),
        Binop::Umul | Binop::Smul => wrap_unsigned_expr(
            &format!("({lhs_for_result} * {rhs_for_result})"),
            result_width,
        ),
        Binop::Eq => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "=",
        ))),
        Binop::Ne => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "<>",
        ))),
        Binop::Ugt => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, ">",
        ))),
        Binop::Uge => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, ">=",
        ))),
        Binop::Ult => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "<",
        ))),
        Binop::Ule => Ok(bool_to_bits_expr(&unsigned_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "<=",
        ))),
        Binop::Sgt => Ok(bool_to_bits_expr(&signed_compare_condition(
            lhs, lhs_width, rhs, rhs_width, ">",
        )?)),
        Binop::Sge => Ok(bool_to_bits_expr(&signed_compare_condition(
            lhs, lhs_width, rhs, rhs_width, ">=",
        )?)),
        Binop::Slt => Ok(bool_to_bits_expr(&signed_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "<",
        )?)),
        Binop::Sle => Ok(bool_to_bits_expr(&signed_compare_condition(
            lhs, lhs_width, rhs, rhs_width, "<=",
        )?)),
        other => Err(PirLustreError::Unsupported(format!(
            "unsupported binary operator `{}`",
            ir::binop_to_operator(other)
        ))),
    }
}

fn expect_bits_width(ty: &ir::Type, context: &str) -> Result<usize, PirLustreError> {
    match ty {
        ir::Type::Bits(width) => Ok(*width),
        _ => Err(PirLustreError::Unsupported(format!(
            "expected {context} to be a bits type, got {ty}"
        ))),
    }
}

fn flattened_type_width(ty: &ir::Type, context: &str) -> Result<usize, PirLustreError> {
    let mut widths = Vec::new();
    flatten_type_bit_widths(ty, &mut widths).map_err(|_| {
        PirLustreError::Unsupported(format!(
            "expected {context} to be bits or tuple of bits, got {ty}"
        ))
    })?;
    Ok(widths.into_iter().sum())
}

fn scalar_literal_expr(decimal: &str, bit_count: usize) -> String {
    let scalar_ty = ScalarBitsType::for_width(bit_count);
    if scalar_ty.is_machine() {
        format!("({} {decimal})", scalar_ty.lustre_type_name())
    } else {
        decimal.to_string()
    }
}

fn zero_bits_expr(bit_count: usize) -> String {
    scalar_literal_expr("0", bit_count)
}

fn unwrap_top_level_ctor<'a>(expr: &'a str, ctor_name: &str) -> Option<&'a str> {
    let prefix = format!("({ctor_name} ");
    expr.strip_prefix(&prefix)?.strip_suffix(')')
}

fn is_plain_decimal_literal(expr: &str) -> bool {
    if expr.is_empty() {
        return false;
    }
    let rest = expr.strip_prefix('-').unwrap_or(expr);
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

fn bits_to_int_expr(expr: &str, bit_count: usize) -> String {
    if ScalarBitsType::for_width(bit_count).is_machine() {
        if let Some(inner) = unwrap_top_level_ctor(expr, "int") {
            inner.to_string()
        } else if let Some(inner) =
            unwrap_top_level_ctor(expr, ScalarBitsType::for_width(bit_count).lustre_type_name())
        {
            if is_plain_decimal_literal(inner) {
                inner.to_string()
            } else {
                format!("(int {expr})")
            }
        } else {
            format!("(int {expr})")
        }
    } else {
        expr.to_string()
    }
}

fn cast_bits_expr(expr: &str, from_width: usize, to_width: usize) -> String {
    let from_ty = ScalarBitsType::for_width(from_width);
    let to_ty = ScalarBitsType::for_width(to_width);
    if from_ty == to_ty {
        expr.to_string()
    } else if to_ty.is_machine() {
        if unwrap_top_level_ctor(expr, to_ty.lustre_type_name()).is_some() {
            expr.to_string()
        } else {
            format!("({} {expr})", to_ty.lustre_type_name())
        }
    } else {
        bits_to_int_expr(expr, from_width)
    }
}

fn raw_mask_decimal(bit_count: usize) -> Result<String, PirLustreError> {
    if bit_count == 0 {
        Ok("0".to_string())
    } else {
        bit_mask(bit_count)
    }
}

fn mask_bits_expr(expr: &str, bit_count: usize) -> Result<String, PirLustreError> {
    if bit_count == 0 {
        return Ok(zero_bits_expr(bit_count));
    }
    let scalar_ty = ScalarBitsType::for_width(bit_count);
    match scalar_ty {
        ScalarBitsType::BigInt => {
            let modulus = bit_modulus(bit_count)?;
            Ok(format!("(({expr}) mod {modulus})"))
        }
        _ => {
            let carrier_width = scalar_ty.carrier_width().expect("machine carrier width");
            let casted = if unwrap_top_level_ctor(expr, scalar_ty.lustre_type_name()).is_some() {
                expr.to_string()
            } else {
                format!("({} {expr})", scalar_ty.lustre_type_name())
            };
            if carrier_width == bit_count {
                Ok(casted)
            } else {
                let mask = scalar_literal_expr(&raw_mask_decimal(bit_count)?, bit_count);
                Ok(format!("({casted} && {mask})"))
            }
        }
    }
}

fn wrap_unsigned_expr(expr: &str, bit_count: usize) -> Result<String, PirLustreError> {
    mask_bits_expr(expr, bit_count)
}

fn signed_int_expr(expr: &str, bit_count: usize) -> Result<String, PirLustreError> {
    if bit_count == 0 {
        return Ok("0".to_string());
    }
    let sign_bit = bit_modulus(bit_count - 1)?;
    let modulus = bit_modulus(bit_count)?;
    Ok(format!(
        "(if ({expr} < {sign_bit}) then {expr} else ({expr} - {modulus}))"
    ))
}

fn signed_compare_condition(
    lhs: &str,
    lhs_width: usize,
    rhs: &str,
    rhs_width: usize,
    operator: &str,
) -> Result<String, PirLustreError> {
    if lhs_width != rhs_width {
        let lhs_int = signed_int_expr(&bits_to_int_expr(lhs, lhs_width), lhs_width)?;
        let rhs_int = signed_int_expr(&bits_to_int_expr(rhs, rhs_width), rhs_width)?;
        return Ok(format!("({lhs_int} {operator} {rhs_int})"));
    }
    let scalar_ty = ScalarBitsType::for_width(lhs_width);
    if scalar_ty.is_machine() {
        let sign_bit = scalar_literal_expr(&bit_modulus(lhs_width - 1)?, lhs_width);
        let lhs_key = wrap_unsigned_expr(&format!("({lhs} + {sign_bit})"), lhs_width)?;
        let rhs_key = wrap_unsigned_expr(&format!("({rhs} + {sign_bit})"), rhs_width)?;
        Ok(format!("({lhs_key} {operator} {rhs_key})"))
    } else {
        let lhs_int = signed_int_expr(lhs, lhs_width)?;
        let rhs_int = signed_int_expr(rhs, rhs_width)?;
        Ok(format!("({lhs_int} {operator} {rhs_int})"))
    }
}

fn unsigned_compare_condition(
    lhs: &str,
    lhs_width: usize,
    rhs: &str,
    rhs_width: usize,
    operator: &str,
) -> String {
    if lhs_width == rhs_width {
        format!("({lhs} {operator} {rhs})")
    } else {
        let lhs_int = bits_to_int_expr(lhs, lhs_width);
        let rhs_int = bits_to_int_expr(rhs, rhs_width);
        format!("({lhs_int} {operator} {rhs_int})")
    }
}

fn shift_left_expr(
    lhs: &str,
    rhs: &str,
    lhs_width: usize,
    rhs_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if lhs_width != result_width {
        return Err(PirLustreError::Unsupported(format!(
            "shift-left result width {result_width} did not match lhs width {lhs_width}"
        )));
    }
    if ScalarBitsType::for_width(result_width).is_machine() {
        let lhs_cast = cast_bits_expr(lhs, lhs_width, result_width);
        let rhs_cast = cast_bits_expr(rhs, rhs_width, result_width);
        wrap_unsigned_expr(&format!("({lhs_cast} lsh {rhs_cast})"), result_width)
    } else {
        variable_shift_expr(
            &bits_to_int_expr(rhs, rhs_width),
            lhs_width,
            || Ok("0".to_string()),
            |amount| {
                let factor = bit_modulus(amount)?;
                wrap_unsigned_expr(
                    &format!("({} * {factor})", bits_to_int_expr(lhs, lhs_width)),
                    result_width,
                )
            },
        )
    }
}

fn shift_right_logical_expr(
    lhs: &str,
    rhs: &str,
    lhs_width: usize,
    rhs_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if lhs_width != result_width {
        return Err(PirLustreError::Unsupported(format!(
            "logical shift-right result width {result_width} did not match lhs width {lhs_width}"
        )));
    }
    if ScalarBitsType::for_width(result_width).is_machine() {
        let lhs_cast = cast_bits_expr(lhs, lhs_width, result_width);
        let rhs_cast = cast_bits_expr(rhs, rhs_width, result_width);
        wrap_unsigned_expr(&format!("({lhs_cast} rsh {rhs_cast})"), result_width)
    } else {
        variable_shift_expr(
            &bits_to_int_expr(rhs, rhs_width),
            lhs_width,
            || Ok("0".to_string()),
            |amount| {
                let divisor = bit_modulus(amount)?;
                Ok(format!(
                    "({} / {divisor})",
                    bits_to_int_expr(lhs, lhs_width)
                ))
            },
        )
    }
}

fn shift_right_arithmetic_expr(
    lhs: &str,
    rhs: &str,
    lhs_width: usize,
    rhs_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if lhs_width != result_width {
        return Err(PirLustreError::Unsupported(format!(
            "arithmetic shift-right result width {result_width} did not match lhs width {lhs_width}"
        )));
    }
    if lhs_width == 0 {
        return Ok(zero_bits_expr(result_width));
    }
    let lhs_int = bits_to_int_expr(lhs, lhs_width);
    let rhs_int = bits_to_int_expr(rhs, rhs_width);
    let sign_bit = bit_modulus(lhs_width - 1)?;
    let ones = bit_mask(lhs_width)?;
    let nonnegative = variable_shift_expr(
        &rhs_int,
        lhs_width,
        || Ok("0".to_string()),
        |amount| {
            let divisor = bit_modulus(amount)?;
            Ok(format!("({lhs_int} / {divisor})"))
        },
    )?;
    let negative_int = variable_shift_expr(
        &rhs_int,
        lhs_width,
        || Ok(ones.to_string()),
        |amount| {
            let divisor = bit_modulus(amount)?;
            let shifted = format!("({lhs_int} / {divisor})");
            let fill = if amount == 0 {
                "0".to_string()
            } else {
                power_of_two_difference_decimal(lhs_width, lhs_width - amount)?
            };
            Ok(format!(
                "(({shifted} + {fill}) mod {})",
                bit_modulus(result_width)?
            ))
        },
    )?;
    wrap_unsigned_expr(
        &format!("(if ({lhs_int} < {sign_bit}) then {nonnegative} else {negative_int})"),
        result_width,
    )
}

fn variable_shift_expr<F, D>(
    rhs: &str,
    width: usize,
    default_expr: D,
    case_expr: F,
) -> Result<String, PirLustreError>
where
    F: Fn(usize) -> Result<String, PirLustreError>,
    D: Fn() -> Result<String, PirLustreError>,
{
    let mut out = default_expr()?;
    for amount in (0..width).rev() {
        let branch = case_expr(amount)?;
        out = format!("(if ({rhs} = {amount}) then {branch} else {out})");
    }
    Ok(out)
}

fn sign_extend_expr(
    expr: &str,
    from_width: usize,
    to_width: usize,
) -> Result<String, PirLustreError> {
    if from_width == 0 {
        return Ok("0".to_string());
    }
    let sign_bit = bit_modulus(from_width - 1)?;
    let from_modulus = bit_modulus(from_width)?;
    let to_modulus = bit_modulus(to_width)?;
    Ok(format!(
        "(if ({expr} < {sign_bit}) then {expr} else ({expr} + {}))",
        decimal_subtract(&to_modulus, &from_modulus)?
    ))
}

fn bitwise_not_expr(expr: &str, bit_count: usize) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(bit_count).is_machine() {
        wrap_unsigned_expr(&format!("(!{expr})"), bit_count)
    } else {
        let mask = bit_mask(bit_count)?;
        Ok(format!("({mask} - {expr})"))
    }
}

fn bit_slice_expr(
    expr: &str,
    expr_width: usize,
    start: usize,
    width: usize,
) -> Result<String, PirLustreError> {
    if width == 0 {
        return Ok(zero_bits_expr(width));
    }
    if ScalarBitsType::for_width(expr_width).is_machine() {
        let shifted = if start == 0 {
            expr.to_string()
        } else {
            format!(
                "({expr} rsh {})",
                scalar_literal_expr(&start.to_string(), expr_width)
            )
        };
        let casted = cast_bits_expr(&shifted, expr_width, width);
        return wrap_unsigned_expr(&casted, width);
    }
    let start_divisor = bit_modulus(start)?;
    let modulus = bit_modulus(width)?;
    Ok(format!("((({expr} / {start_divisor}) mod {modulus}))"))
}

fn dynamic_bit_slice_expr(
    expr: &str,
    expr_width: usize,
    start_expr: &str,
    start_width: usize,
    arg_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if result_width == 1 && ScalarBitsType::for_width(expr_width).is_machine() {
        let limit = scalar_literal_expr(&arg_width.to_string(), start_width);
        let in_range =
            unsigned_compare_condition(start_expr, start_width, &limit, start_width, "<");
        let shift_amount = cast_bits_expr(start_expr, start_width, expr_width);
        let one = scalar_literal_expr("1", expr_width);
        let bit_set = format!("((({expr} rsh {shift_amount}) && {one}) = {one})");
        return Ok(bool_to_bits_expr(&format!("(if {in_range} then {bit_set} else false)")));
    }
    let start_match_expr = bits_to_int_expr(start_expr, start_width);
    let mut out = zero_bits_expr(result_width);
    for start in (0..arg_width).rev() {
        let branch = bit_slice_expr(expr, expr_width, start, result_width)?;
        out = format!("(if ({start_match_expr} = {start}) then {branch} else {out})");
    }
    Ok(out)
}

fn wide_dynamic_bit_test_condition(
    chunk_exprs: &[(String, usize)],
    start_expr: &str,
    start_width: usize,
    arg_width: usize,
) -> Result<String, PirLustreError> {
    let selector_covers_full_range = match 1usize.checked_shl(start_width as u32) {
        Some(selector_range) => arg_width >= selector_range,
        None => true,
    };
    let in_range = if selector_covers_full_range {
        "true".to_string()
    } else {
        let limit = scalar_literal_expr(&arg_width.to_string(), start_width);
        unsigned_compare_condition(start_expr, start_width, &limit, start_width, "<")
    };

    let mut chunk_starts = Vec::with_capacity(chunk_exprs.len());
    let mut chunk_lsb = 0usize;
    for (_, chunk_width) in chunk_exprs {
        chunk_starts.push(chunk_lsb);
        chunk_lsb += *chunk_width;
    }

    let mut out = "false".to_string();
    for ((chunk_expr, chunk_width), chunk_lsb) in
        chunk_exprs.iter().zip(chunk_starts.into_iter()).rev()
    {
        let chunk_end = chunk_lsb + *chunk_width;
        let chunk_reaches_selector_limit = match 1usize.checked_shl(start_width as u32) {
            Some(selector_range) => chunk_end >= selector_range,
            None => true,
        };
        let chunk_in = if chunk_lsb == 0 {
            if chunk_reaches_selector_limit {
                "true".to_string()
            } else {
                let chunk_end_lit = scalar_literal_expr(&chunk_end.to_string(), start_width);
                unsigned_compare_condition(
                    start_expr,
                    start_width,
                    &chunk_end_lit,
                    start_width,
                    "<",
                )
            }
        } else {
            let chunk_lsb_lit = scalar_literal_expr(&chunk_lsb.to_string(), start_width);
            let at_or_above_lsb = unsigned_compare_condition(
                start_expr,
                start_width,
                &chunk_lsb_lit,
                start_width,
                ">=",
            );
            if chunk_reaches_selector_limit {
                at_or_above_lsb
            } else {
                let chunk_end_lit = scalar_literal_expr(&chunk_end.to_string(), start_width);
                let below_end = unsigned_compare_condition(
                    start_expr,
                    start_width,
                    &chunk_end_lit,
                    start_width,
                    "<",
                );
                combine_bool_terms("and", &[at_or_above_lsb, below_end], Some("false"))
            }
        };

        let bit_set = if *chunk_width == 1 {
            chunk_expr.clone()
        } else {
            let local_shift_expr = if chunk_lsb == 0 {
                start_expr.to_string()
            } else {
                let chunk_lsb_lit = scalar_literal_expr(&chunk_lsb.to_string(), start_width);
                format!("({start_expr} - {chunk_lsb_lit})")
            };
            let local_shift = cast_bits_expr(&local_shift_expr, start_width, *chunk_width);
            let one = scalar_literal_expr("1", *chunk_width);
            format!("((({chunk_expr} rsh {local_shift}) && {one}) = {one})")
        };
        out = format!("(if {chunk_in} then {bit_set} else {out})");
    }

    if in_range == "true" {
        Ok(out)
    } else {
        Ok(format!("(if {in_range} then {out} else false)"))
    }
}

fn bit_slice_update_expr(
    arg_expr: &str,
    arg_expr_width: usize,
    start_expr: &str,
    start_width: usize,
    arg_width: usize,
    update_expr: &str,
    update_expr_width: usize,
    update_width: usize,
) -> Result<String, PirLustreError> {
    let arg_int = bits_to_int_expr(arg_expr, arg_expr_width);
    let start_int = bits_to_int_expr(start_expr, start_width);
    let update_int = bits_to_int_expr(update_expr, update_expr_width);
    let mut out = arg_expr.to_string();
    for start in (0..arg_width).rev() {
        let replace_width = std::cmp::min(update_width, arg_width - start);
        let low = if start == 0 {
            "0".to_string()
        } else {
            format!("({arg_int} mod {})", bit_modulus(start)?)
        };
        let update = if replace_width == 0 {
            "0".to_string()
        } else {
            format!(
                "(({update_int} mod {}) * {})",
                bit_modulus(replace_width)?,
                bit_modulus(start)?
            )
        };
        let preserved_shift = start + replace_width;
        let high = if preserved_shift >= arg_width {
            "0".to_string()
        } else {
            let shift_amount = bit_modulus(preserved_shift)?;
            format!(
                "((({arg_int} / {shift_amount}) mod {}) * {shift_amount})",
                bit_modulus(arg_width - preserved_shift)?,
            )
        };
        let branch = wrap_unsigned_expr(&format!("({low} + {update} + {high})"), arg_width)?;
        out = format!("(if ({start_int} = {start}) then {branch} else {out})");
    }
    Ok(out)
}

fn concat_expr(operands: &[String], widths: &[usize]) -> Result<String, PirLustreError> {
    assert_eq!(operands.len(), widths.len());
    let mut iter = operands.iter().zip(widths.iter());
    let Some((first_expr, first_width)) = iter.next() else {
        return Err(PirLustreError::Unsupported(
            "concat requires at least one operand".to_string(),
        ));
    };
    let total_width: usize = widths.iter().sum();
    let scalar_ty = ScalarBitsType::for_width(total_width);
    let mut out = if scalar_ty.is_machine() {
        cast_bits_expr(first_expr, *first_width, total_width)
    } else {
        bits_to_int_expr(first_expr, *first_width)
    };
    let mut accumulated_width = *first_width;
    for (expr, width) in iter {
        if scalar_ty.is_machine() {
            let shifted = format!(
                "({out} lsh {})",
                scalar_literal_expr(&width.to_string(), total_width)
            );
            let rhs = cast_bits_expr(expr, *width, total_width);
            out = format!("({shifted} || {rhs})");
        } else {
            let rhs = bits_to_int_expr(expr, *width);
            out = format!("(({out} * {}) + {rhs})", bit_modulus(*width)?);
        }
        accumulated_width += *width;
    }
    wrap_unsigned_expr(&out, accumulated_width)
}

fn lower_select_value(
    selector_expr: &str,
    cases: &[LoweredValue],
    default: Option<&LoweredValue>,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    let first = cases.first().ok_or_else(|| {
        PirLustreError::Unsupported(format!("sel in `{fn_name}` did not contain any cases"))
    })?;
    match first {
        LoweredValue::Bits { .. } | LoweredValue::Bool(_) => {
            if scalar_cases_are_bool_compatible(cases, default) {
                let case_exprs = cases
                    .iter()
                    .enumerate()
                    .map(|(index, value)| value.clone().into_bool(index, fn_name))
                    .collect::<Result<Vec<_>, _>>()?;
                let default_expr = match default {
                    Some(value) => Some(value.clone().into_bool(usize::MAX, fn_name)?),
                    None => None,
                };
                Ok(LoweredValue::Bool(select_bool_expr(
                    selector_expr,
                    &case_exprs,
                    default_expr.as_deref(),
                )?))
            } else {
                let first_width = first.bit_width().ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` did not lower to a scalar width"
                    ))
                })?;
                let case_exprs = cases
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let width = value.bit_width().ok_or_else(|| {
                            PirLustreError::Unsupported(format!(
                                "sel in `{fn_name}` mixed scalar and tuple case values"
                            ))
                        })?;
                        if width != first_width {
                            return Err(PirLustreError::Unsupported(format!(
                                "sel in `{fn_name}` mixed bits[{first_width}] and bits[{width}] case values"
                            )));
                        }
                        value.clone().into_bits(index, fn_name)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_expr = match default {
                    Some(value) => {
                        let width = value.bit_width().ok_or_else(|| {
                            PirLustreError::Unsupported(format!(
                                "sel in `{fn_name}` mixed scalar and tuple values"
                            ))
                        })?;
                        if width != first_width {
                            return Err(PirLustreError::Unsupported(format!(
                                "sel in `{fn_name}` mixed bits[{first_width}] and bits[{width}] values"
                            )));
                        }
                        Some(value.clone().into_bits(usize::MAX, fn_name)?)
                    }
                    None => None,
                };
                Ok(LoweredValue::Bits {
                    expr: select_scalar_expr(
                        selector_expr,
                        &case_exprs.iter().map(String::as_str).collect::<Vec<_>>(),
                        default_expr.as_deref(),
                    )?,
                    width: first_width,
                })
            }
        }
        LoweredValue::Composite { ty, .. } => {
            let case_exprs = cases
                .iter()
                .map(|value| match value {
                    LoweredValue::Composite { expr, ty: case_ty } if case_ty == ty => {
                        Ok(expr.clone())
                    }
                    LoweredValue::Composite { ty: case_ty, .. } => {
                        Err(PirLustreError::Unsupported(format!(
                            "sel in `{fn_name}` mixed composite types {ty} and {case_ty}"
                        )))
                    }
                    _ => Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed composite and non-composite values"
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let default_expr = match default {
                Some(LoweredValue::Composite { expr, ty: default_ty }) if default_ty == ty => {
                    Some(expr.clone())
                }
                Some(LoweredValue::Composite { ty: default_ty, .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed composite types {ty} and {default_ty}"
                    )));
                }
                Some(_) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed composite and non-composite values"
                    )));
                }
                None => None,
            };
            Ok(LoweredValue::Composite {
                expr: select_scalar_expr(
                    selector_expr,
                    &case_exprs.iter().map(String::as_str).collect::<Vec<_>>(),
                    default_expr.as_deref(),
                )?,
                ty: ty.clone(),
            })
        }
        LoweredValue::Aggregate { spec, fields } => {
            let default_fields = match default {
                Some(LoweredValue::Aggregate {
                    spec: default_spec,
                    fields,
                }) => {
                    if spec.fields.len() != default_spec.fields.len() {
                        return Err(PirLustreError::Unsupported(format!(
                            "aggregate-valued sel in `{fn_name}` had inconsistent field counts"
                        )));
                    }
                    Some(fields)
                }
                Some(LoweredValue::Bits { .. })
                | Some(LoweredValue::Bool(_))
                | Some(LoweredValue::Tuple(_))
                | Some(LoweredValue::Composite { .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed aggregate and non-aggregate values"
                    )));
                }
                None => None,
            };
            let mut result = Vec::with_capacity(fields.len());
            for index in 0..fields.len() {
                let field_cases = cases
                    .iter()
                    .map(|value| match value {
                        LoweredValue::Aggregate {
                            spec: case_spec,
                            fields,
                        } => {
                            if case_spec.fields.len() != spec.fields.len() {
                                return Err(PirLustreError::Unsupported(format!(
                                    "aggregate-valued sel in `{fn_name}` had inconsistent field counts"
                                )));
                            }
                            fields.get(index).cloned().ok_or_else(|| {
                                PirLustreError::Unsupported(format!(
                                    "aggregate-valued sel in `{fn_name}` had inconsistent field counts"
                                ))
                            })
                        }
                        LoweredValue::Bits { .. }
                        | LoweredValue::Bool(_)
                        | LoweredValue::Tuple(_)
                        | LoweredValue::Composite { .. } => Err(PirLustreError::Unsupported(format!(
                            "sel in `{fn_name}` mixed aggregate and non-aggregate values"
                        ))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let field_default = default_fields.and_then(|fields| fields.get(index));
                result.push(lower_select_value(
                    selector_expr,
                    &field_cases,
                    field_default,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Aggregate {
                spec: spec.clone(),
                fields: result,
            })
        }
        LoweredValue::Tuple(fields) => {
            let tuple_len = fields.len();
            let default_tuple = match default {
                Some(LoweredValue::Tuple(fields)) => Some(fields),
                Some(LoweredValue::Bits { .. })
                | Some(LoweredValue::Bool(_))
                | Some(LoweredValue::Aggregate { .. })
                | Some(LoweredValue::Composite { .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed tuple and scalar values"
                    )));
                }
                None => None,
            };
            let mut result = Vec::with_capacity(tuple_len);
            for index in 0..tuple_len {
                let field_cases = cases
                    .iter()
                    .map(|value| match value {
                        LoweredValue::Tuple(fields) => {
                            fields.get(index).cloned().ok_or_else(|| {
                                PirLustreError::Unsupported(format!(
                                    "tuple-valued sel in `{fn_name}` had inconsistent tuple arity"
                                ))
                            })
                        }
                        LoweredValue::Bits { .. }
                        | LoweredValue::Bool(_)
                        | LoweredValue::Aggregate { .. }
                        | LoweredValue::Composite { .. } => Err(PirLustreError::Unsupported(
                            format!("sel in `{fn_name}` mixed tuple and scalar values"),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let field_default = default_tuple.and_then(|fields| fields.get(index));
                result.push(lower_select_value(
                    selector_expr,
                    &field_cases,
                    field_default,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Tuple(result))
        }
    }
}

fn lower_priority_select_value(
    selector_expr: &str,
    selector_width: usize,
    cases: &[LoweredValue],
    default: Option<&LoweredValue>,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    let first = cases.first().ok_or_else(|| {
        PirLustreError::Unsupported(format!(
            "priority_sel in `{fn_name}` did not contain any cases"
        ))
    })?;
    match first {
        LoweredValue::Bits { .. } | LoweredValue::Bool(_) => {
            if scalar_cases_are_bool_compatible(cases, default) {
                let case_exprs = cases
                    .iter()
                    .enumerate()
                    .map(|(index, value)| value.clone().into_bool(index, fn_name))
                    .collect::<Result<Vec<_>, _>>()?;
                let default_expr = match default {
                    Some(value) => Some(value.clone().into_bool(usize::MAX, fn_name)?),
                    None => None,
                };
                Ok(LoweredValue::Bool(priority_select_bool_expr(
                    selector_expr,
                    selector_width,
                    &case_exprs,
                    default_expr.as_deref(),
                )?))
            } else {
                let first_width = first.bit_width().ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` did not lower to a scalar width"
                    ))
                })?;
                let case_exprs = cases
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let width = value.bit_width().ok_or_else(|| {
                            PirLustreError::Unsupported(format!(
                                "priority_sel in `{fn_name}` mixed scalar and tuple case values"
                            ))
                        })?;
                        if width != first_width {
                            return Err(PirLustreError::Unsupported(format!(
                                "priority_sel in `{fn_name}` mixed bits[{first_width}] and bits[{width}] case values"
                            )));
                        }
                        value.clone().into_bits(index, fn_name)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_expr = match default {
                    Some(value) => {
                        let width = value.bit_width().ok_or_else(|| {
                            PirLustreError::Unsupported(format!(
                                "priority_sel in `{fn_name}` mixed scalar and tuple values"
                            ))
                        })?;
                        if width != first_width {
                            return Err(PirLustreError::Unsupported(format!(
                                "priority_sel in `{fn_name}` mixed bits[{first_width}] and bits[{width}] values"
                            )));
                        }
                        Some(value.clone().into_bits(usize::MAX, fn_name)?)
                    }
                    None => None,
                };
                Ok(LoweredValue::Bits {
                    expr: priority_select_scalar_expr(
                        selector_expr,
                        selector_width,
                        &case_exprs.iter().map(String::as_str).collect::<Vec<_>>(),
                        first_width,
                        default_expr.as_deref(),
                    )?,
                    width: first_width,
                })
            }
        }
        LoweredValue::Composite { ty, .. } => {
            let case_exprs = cases
                .iter()
                .map(|value| match value {
                    LoweredValue::Composite { expr, ty: case_ty } if case_ty == ty => {
                        Ok(expr.clone())
                    }
                    LoweredValue::Composite { ty: case_ty, .. } => {
                        Err(PirLustreError::Unsupported(format!(
                            "priority_sel in `{fn_name}` mixed composite types {ty} and {case_ty}"
                        )))
                    }
                    _ => Err(PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` mixed composite and non-composite values"
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut native_types = NativeTypeRegistry::default();
            let default_expr = match default {
                Some(LoweredValue::Composite { expr, ty: default_ty }) if default_ty == ty => {
                    expr.clone()
                }
                Some(LoweredValue::Composite { ty: default_ty, .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` mixed composite types {ty} and {default_ty}"
                    )));
                }
                Some(_) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` mixed composite and non-composite values"
                    )));
                }
                None => lowered_value_to_native_expr(
                    &native_zero_value(ty, &mut native_types)?,
                    ty,
                    usize::MAX,
                    fn_name,
                    &mut native_types,
                )?,
            };
            Ok(LoweredValue::Composite {
                expr: priority_select_scalar_expr(
                    selector_expr,
                    selector_width,
                    &case_exprs.iter().map(String::as_str).collect::<Vec<_>>(),
                    1,
                    Some(default_expr.as_str()),
                )?,
                ty: ty.clone(),
            })
        }
        LoweredValue::Aggregate { spec, fields } => {
            let default_fields = match default {
                Some(LoweredValue::Aggregate {
                    spec: default_spec,
                    fields,
                }) => {
                    if spec.fields.len() != default_spec.fields.len() {
                        return Err(PirLustreError::Unsupported(format!(
                            "aggregate-valued priority_sel in `{fn_name}` had inconsistent field counts"
                        )));
                    }
                    Some(fields)
                }
                Some(LoweredValue::Bits { .. })
                | Some(LoweredValue::Bool(_))
                | Some(LoweredValue::Tuple(_))
                | Some(LoweredValue::Composite { .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` mixed aggregate and non-aggregate values"
                    )));
                }
                None => None,
            };
            let mut result = Vec::with_capacity(fields.len());
            for index in 0..fields.len() {
                let field_cases = cases
                    .iter()
                    .map(|value| match value {
                        LoweredValue::Aggregate {
                            spec: case_spec,
                            fields,
                        } => {
                            if case_spec.fields.len() != spec.fields.len() {
                                return Err(PirLustreError::Unsupported(format!(
                                    "aggregate-valued priority_sel in `{fn_name}` had inconsistent field counts"
                                )));
                            }
                            fields.get(index).cloned().ok_or_else(|| {
                                PirLustreError::Unsupported(format!(
                                    "aggregate-valued priority_sel in `{fn_name}` had inconsistent field counts"
                                ))
                            })
                        }
                        LoweredValue::Bits { .. }
                        | LoweredValue::Bool(_)
                        | LoweredValue::Tuple(_)
                        | LoweredValue::Composite { .. } => Err(PirLustreError::Unsupported(format!(
                            "priority_sel in `{fn_name}` mixed aggregate and non-aggregate values"
                        ))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let field_default = default_fields.and_then(|fields| fields.get(index));
                result.push(lower_priority_select_value(
                    selector_expr,
                    selector_width,
                    &field_cases,
                    field_default,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Aggregate {
                spec: spec.clone(),
                fields: result,
            })
        }
        LoweredValue::Tuple(fields) => {
            let tuple_len = fields.len();
            let default_tuple = match default {
                Some(LoweredValue::Tuple(fields)) => Some(fields),
                Some(LoweredValue::Bits { .. })
                | Some(LoweredValue::Bool(_))
                | Some(LoweredValue::Aggregate { .. })
                | Some(LoweredValue::Composite { .. }) => {
                    return Err(PirLustreError::Unsupported(format!(
                        "priority_sel in `{fn_name}` mixed tuple and scalar values"
                    )));
                }
                None => None,
            };
            let mut result = Vec::with_capacity(tuple_len);
            for index in 0..tuple_len {
                let field_cases = cases
                    .iter()
                    .map(|value| match value {
                        LoweredValue::Tuple(fields) => {
                            fields.get(index).cloned().ok_or_else(|| {
                                PirLustreError::Unsupported(format!(
                                    "tuple-valued priority_sel in `{fn_name}` had inconsistent tuple arity"
                                ))
                            })
                        }
                        LoweredValue::Bits { .. }
                        | LoweredValue::Bool(_)
                        | LoweredValue::Aggregate { .. }
                        | LoweredValue::Composite { .. } => {
                            Err(PirLustreError::Unsupported(format!(
                                "priority_sel in `{fn_name}` mixed tuple and scalar values"
                            )))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let field_default = default_tuple.and_then(|fields| fields.get(index));
                result.push(lower_priority_select_value(
                    selector_expr,
                    selector_width,
                    &field_cases,
                    field_default,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Tuple(result))
        }
    }
}

fn aggregate_bit_slice(
    fields: &[LoweredValue],
    start: usize,
    width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    if width == 0 {
        return Ok(LoweredValue::Bits {
            expr: "0".to_string(),
            width: 0,
        });
    }
    let mut field_ranges = Vec::with_capacity(fields.len());
    let mut offset = 0usize;
    for field in fields.iter().rev() {
        let field_width = field.bit_width().ok_or_else(|| {
            PirLustreError::Unsupported(format!(
                "aggregate field in node {node_index} of `{fn_name}` did not lower to a scalar width"
            ))
        })?;
        field_ranges.push((offset, offset + field_width, field.clone(), field_width));
        offset += field_width;
    }
    let end = start + width;
    let mut parts = Vec::new();
    for (field_lsb, field_end, field, field_width) in field_ranges.into_iter().rev() {
        let part_start = start.max(field_lsb);
        let part_end = end.min(field_end);
        if part_start >= part_end {
            continue;
        }
        let slice_start = part_start - field_lsb;
        let slice_width = part_end - part_start;
        let part = if slice_start == 0 && slice_width == field_width {
            field
        } else {
            let expr = field.clone().into_bits(node_index, fn_name)?;
            if slice_width == 1 {
                LoweredValue::Bool(bit_test_condition(&expr, field_width, slice_start)?)
            } else {
                LoweredValue::Bits {
                    expr: bit_slice_expr(&expr, field_width, slice_start, slice_width)?,
                    width: slice_width,
                }
            }
        };
        parts.push(part);
    }

    if parts.is_empty() {
        return Err(PirLustreError::Unsupported(format!(
            "bit_slice({}, {}) on aggregate in node {node_index} of `{fn_name}` was empty",
            start, width
        )));
    }
    let part_widths = parts
        .iter()
        .map(|part| {
            part.bit_width().ok_or_else(|| {
                PirLustreError::Unsupported(format!(
                    "bit_slice({}, {}) on aggregate in node {node_index} of `{fn_name}` produced a non-scalar part",
                    start, width
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    concat_lowered_values(&parts, &part_widths, width, node_index, fn_name)
}

fn wide_bit_slice_value(
    expr: &str,
    expr_width: usize,
    start: usize,
    width: usize,
    node_index: usize,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    if width == 0 {
        return Ok(LoweredValue::Bits {
            expr: zero_bits_expr(0),
            width: 0,
        });
    }
    let end = start + width;
    let mut parts = Vec::new();
    let mut covered_width = 0usize;
    let mut chunk_ranges = Vec::new();
    let mut chunk_lsb = 0usize;
    for (chunk_expr, chunk_width) in wide_bits_chunk_exprs(expr, expr_width) {
        let chunk_end = chunk_lsb + chunk_width;
        chunk_ranges.push((chunk_lsb, chunk_end, chunk_expr, chunk_width));
        chunk_lsb = chunk_end;
    }
    for (chunk_lsb, chunk_end, chunk_expr, chunk_width) in chunk_ranges.into_iter().rev() {
        let part_start = start.max(chunk_lsb);
        let part_end = end.min(chunk_end);
        if part_start < part_end {
            let slice_start = part_start - chunk_lsb;
            let slice_width = part_end - part_start;
            let part = if slice_start == 0 && slice_width == chunk_width {
                lowered_value_from_native_expr(
                    chunk_expr,
                    &ir::Type::Bits(chunk_width),
                    &mut NativeTypeRegistry::default(),
                )?
            } else if slice_width == 1 {
                LoweredValue::Bool(bit_test_condition(&chunk_expr, chunk_width, slice_start)?)
            } else {
                LoweredValue::Bits {
                    expr: bit_slice_expr(&chunk_expr, chunk_width, slice_start, slice_width)?,
                    width: slice_width,
                }
            };
            covered_width += slice_width;
            parts.push(part);
        }
    }
    if covered_width < width {
        let pad_width = width - covered_width;
        let pad = if pad_width == 1 {
            LoweredValue::Bool("false".to_string())
        } else {
            LoweredValue::Bits {
                expr: zero_bits_expr(pad_width),
                width: pad_width,
            }
        };
        parts.insert(0, pad);
    }
    if width == 1 {
        return parts
            .into_iter()
            .next()
            .expect("width-1 slice should contain one part")
            .into_bool(node_index, fn_name)
            .map(LoweredValue::Bool);
    }
    let part_widths = parts
        .iter()
        .map(|part| {
            part.bit_width().ok_or_else(|| {
                PirLustreError::Unsupported(format!(
                    "wide bit_slice({}, {}) in node {node_index} of `{fn_name}` produced a non-scalar part",
                    start, width
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    concat_lowered_values(&parts, &part_widths, width, node_index, fn_name)
}

fn lower_boolean_select_value(
    selector_condition: &str,
    cases: &[LoweredValue],
    default: Option<&LoweredValue>,
    fn_name: &str,
) -> Result<LoweredValue, PirLustreError> {
    let false_value = cases.first().ok_or_else(|| {
        PirLustreError::Unsupported(format!("sel in `{fn_name}` did not contain any cases"))
    })?;
    let true_value = cases.get(1).or(default).unwrap_or(false_value);
    match false_value {
        LoweredValue::Bits { .. } | LoweredValue::Bool(_) => {
            if scalar_pair_is_bool_compatible(false_value, true_value) {
                let false_expr = false_value.clone().into_bool(0, fn_name)?;
                let true_expr = true_value.clone().into_bool(1, fn_name)?;
                Ok(LoweredValue::Bool(format!(
                    "(if {selector_condition} then {true_expr} else {false_expr})"
                )))
            } else {
                let false_width = false_value.bit_width().ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` did not lower to a scalar width"
                    ))
                })?;
                let true_width = true_value.bit_width().ok_or_else(|| {
                    PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed scalar and tuple values"
                    ))
                })?;
                if false_width != true_width {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed bits[{false_width}] and bits[{true_width}] values"
                    )));
                }
                let false_expr = false_value.clone().into_bits(0, fn_name)?;
                let true_expr = true_value.clone().into_bits(1, fn_name)?;
                Ok(LoweredValue::Bits {
                    expr: format!("(if {selector_condition} then {true_expr} else {false_expr})"),
                    width: false_width,
                })
            }
        }
        LoweredValue::Composite { ty, .. } => {
            let false_expr = match false_value {
                LoweredValue::Composite { expr, ty: false_ty } if false_ty == ty => expr.clone(),
                _ => unreachable!(),
            };
            let true_expr = match true_value {
                LoweredValue::Composite { expr, ty: true_ty } if true_ty == ty => expr.clone(),
                LoweredValue::Composite { ty: true_ty, .. } => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed composite types {ty} and {true_ty}"
                    )));
                }
                _ => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed composite and non-composite values"
                    )));
                }
            };
            Ok(LoweredValue::Composite {
                expr: format!("(if {selector_condition} then {true_expr} else {false_expr})"),
                ty: ty.clone(),
            })
        }
        LoweredValue::Aggregate { spec, fields } => {
            let true_fields = match true_value {
                LoweredValue::Aggregate {
                    spec: true_spec,
                    fields,
                } => {
                    if spec.fields.len() != true_spec.fields.len() {
                        return Err(PirLustreError::Unsupported(format!(
                            "aggregate-valued sel in `{fn_name}` had inconsistent field counts"
                        )));
                    }
                    fields
                }
                LoweredValue::Bits { .. }
                | LoweredValue::Bool(_)
                | LoweredValue::Tuple(_)
                | LoweredValue::Composite { .. } => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed aggregate and non-aggregate values"
                    )));
                }
            };
            let mut result = Vec::with_capacity(fields.len());
            for index in 0..fields.len() {
                result.push(lower_boolean_select_value(
                    selector_condition,
                    &[fields[index].clone(), true_fields[index].clone()],
                    None,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Aggregate {
                spec: spec.clone(),
                fields: result,
            })
        }
        LoweredValue::Tuple(fields) => {
            let true_fields = match true_value {
                LoweredValue::Tuple(fields) => fields,
                LoweredValue::Bits { .. }
                | LoweredValue::Bool(_)
                | LoweredValue::Aggregate { .. }
                | LoweredValue::Composite { .. } => {
                    return Err(PirLustreError::Unsupported(format!(
                        "sel in `{fn_name}` mixed tuple and scalar values"
                    )));
                }
            };
            if fields.len() != true_fields.len() {
                return Err(PirLustreError::Unsupported(format!(
                    "tuple-valued sel in `{fn_name}` had inconsistent tuple arity"
                )));
            }
            let mut result = Vec::with_capacity(fields.len());
            for index in 0..fields.len() {
                result.push(lower_boolean_select_value(
                    selector_condition,
                    &[fields[index].clone(), true_fields[index].clone()],
                    None,
                    fn_name,
                )?);
            }
            Ok(LoweredValue::Tuple(result))
        }
    }
}

fn select_scalar_expr(
    selector_expr: &str,
    case_exprs: &[&str],
    default_expr: Option<&str>,
) -> Result<String, PirLustreError> {
    let Some((last, _rest)) = case_exprs.split_last() else {
        return Err(PirLustreError::Unsupported(
            "sel requires at least one case".to_string(),
        ));
    };
    let mut out = default_expr.unwrap_or(last).to_string();
    let limit = if default_expr.is_some() {
        case_exprs.len()
    } else {
        case_exprs.len().saturating_sub(1)
    };
    for index in (0..limit).rev() {
        out = format!(
            "(if ({selector_expr} = {index}) then {} else {out})",
            case_exprs[index]
        );
    }
    Ok(out)
}

fn select_bool_expr(
    selector_expr: &str,
    case_exprs: &[String],
    default_expr: Option<&str>,
) -> Result<String, PirLustreError> {
    let Some((last, _rest)) = case_exprs.split_last() else {
        return Err(PirLustreError::Unsupported(
            "sel requires at least one case".to_string(),
        ));
    };
    let mut out = default_expr.unwrap_or(last).to_string();
    let limit = if default_expr.is_some() {
        case_exprs.len()
    } else {
        case_exprs.len().saturating_sub(1)
    };
    for index in (0..limit).rev() {
        out = format!(
            "(if ({selector_expr} = {index}) then {} else {out})",
            case_exprs[index]
        );
    }
    Ok(out)
}

fn priority_select_scalar_expr(
    selector_expr: &str,
    selector_width: usize,
    case_exprs: &[&str],
    result_width: usize,
    default_expr: Option<&str>,
) -> Result<String, PirLustreError> {
    let zero_expr = zero_bits_expr(result_width);
    let mut out = default_expr.unwrap_or(zero_expr.as_str()).to_string();
    for index in (0..case_exprs.len()).rev() {
        if index >= selector_width {
            continue;
        }
        out = format!(
            "(if {} then {} else {out})",
            bit_test_condition(selector_expr, selector_width, index)?,
            case_exprs[index]
        );
    }
    Ok(out)
}

fn priority_select_bool_expr(
    selector_expr: &str,
    selector_width: usize,
    case_exprs: &[String],
    default_expr: Option<&str>,
) -> Result<String, PirLustreError> {
    let mut out = default_expr.unwrap_or("false").to_string();
    for index in (0..case_exprs.len()).rev() {
        if index >= selector_width {
            continue;
        }
        out = format!(
            "(if {} then {} else {out})",
            bit_test_condition(selector_expr, selector_width, index)?,
            case_exprs[index]
        );
    }
    Ok(out)
}

fn bool_to_bits_expr(condition: &str) -> String {
    match condition {
        "true" => "1".to_string(),
        "false" => "0".to_string(),
        _ => format!("(if {condition} then 1 else 0)"),
    }
}

fn scalar_cases_are_bool_compatible(
    cases: &[LoweredValue],
    default: Option<&LoweredValue>,
) -> bool {
    let cases_ok = cases
        .iter()
        .all(|value| matches!(value.bit_width(), Some(1)));
    let default_ok = default.is_none_or(|value| matches!(value.bit_width(), Some(1)));
    cases_ok && default_ok
}

fn scalar_pair_is_bool_compatible(lhs: &LoweredValue, rhs: &LoweredValue) -> bool {
    matches!(lhs.bit_width(), Some(1)) && matches!(rhs.bit_width(), Some(1))
}

fn bitwise_nary_expr(
    op: NaryOp,
    operands: &[String],
    bit_count: usize,
) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(bit_count).is_machine() {
        match op {
            NaryOp::And | NaryOp::Nand | NaryOp::Or | NaryOp::Nor => {
                let joined = machine_bitwise_join_expr(op, operands, bit_count)?;
                return wrap_unsigned_expr(&joined, bit_count);
            }
            NaryOp::Xor | NaryOp::Concat => {}
        }
    }
    let bit_terms = (0..bit_count)
        .map(|bit_index| {
            let conditions = operands
                .iter()
                .map(|operand| bit_test_condition(operand, bit_count, bit_index))
                .collect::<Result<Vec<_>, _>>()?;
            let condition = match op {
                NaryOp::And => combine_bool_terms("and", &conditions, Some("true")),
                NaryOp::Nand => format!(
                    "(not {})",
                    combine_bool_terms("and", &conditions, Some("true"))
                ),
                NaryOp::Or => combine_bool_terms("or", &conditions, Some("false")),
                NaryOp::Nor => format!(
                    "(not {})",
                    combine_bool_terms("or", &conditions, Some("false"))
                ),
                NaryOp::Xor => xor_conditions_expr(&conditions)?,
                NaryOp::Concat => {
                    return Err(PirLustreError::Unsupported(
                        "internal error: concat should be handled separately".to_string(),
                    ));
                }
            };
            Ok(bit_term(bit_index, &condition))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    wrap_unsigned_expr(&sum_terms(&bit_terms)?, bit_count)
}

fn machine_bitwise_join_expr(
    op: NaryOp,
    operands: &[String],
    bit_count: usize,
) -> Result<String, PirLustreError> {
    let operator = match op {
        NaryOp::And | NaryOp::Nand => "&&",
        NaryOp::Or | NaryOp::Nor => "||",
        NaryOp::Xor | NaryOp::Concat => {
            return Err(PirLustreError::Unsupported(format!(
                "machine bitwise join does not support {:?}",
                op
            )));
        }
    };
    if operands.is_empty() {
        return Ok(zero_bits_expr(bit_count));
    }
    let joined = operands
        .iter()
        .map(|operand| cast_bits_expr(operand, bit_count, bit_count))
        .collect::<Vec<_>>()
        .join(&format!(" {operator} "));
    let expr = format!("({joined})");
    Ok(match op {
        NaryOp::Nand | NaryOp::Nor => format!("(!{expr})"),
        NaryOp::And | NaryOp::Or => expr,
        NaryOp::Xor | NaryOp::Concat => unreachable!(),
    })
}

fn bitwise_nary_bool_expr(op: NaryOp, operands: &[String]) -> Result<String, PirLustreError> {
    Ok(match op {
        NaryOp::And => combine_bool_terms("and", operands, Some("true")),
        NaryOp::Nand => format!(
            "(not {})",
            combine_bool_terms("and", operands, Some("true"))
        ),
        NaryOp::Or => combine_bool_terms("or", operands, Some("false")),
        NaryOp::Nor => format!(
            "(not {})",
            combine_bool_terms("or", operands, Some("false"))
        ),
        NaryOp::Xor => xor_conditions_expr(operands)?,
        NaryOp::Concat => {
            return Err(PirLustreError::Unsupported(
                "internal error: concat should be handled separately".to_string(),
            ));
        }
    })
}

fn one_hot_sel_expr(
    selector_expr: &str,
    selector_width: usize,
    case_exprs: &[String],
    result_width: usize,
) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(result_width).is_machine() {
        let zero = zero_bits_expr(result_width);
        let mut terms = Vec::new();
        for case_index in 0..case_exprs.len() {
            if case_index >= selector_width {
                break;
            }
            let selector_bit = bit_test_condition(selector_expr, selector_width, case_index)?;
            let case_expr = cast_bits_expr(&case_exprs[case_index], result_width, result_width);
            terms.push(format!("(if {selector_bit} then {case_expr} else {zero})"));
        }
        return wrap_unsigned_expr(
            &machine_bitwise_join_expr(NaryOp::Or, &terms, result_width)?,
            result_width,
        );
    }
    let bit_terms = (0..result_width)
        .map(|result_bit| {
            let mut selectors = Vec::new();
            for case_index in 0..case_exprs.len() {
                if case_index >= selector_width {
                    break;
                }
                let selector_bit = bit_test_condition(selector_expr, selector_width, case_index)?;
                let case_bit =
                    bit_test_condition(&case_exprs[case_index], result_width, result_bit)?;
                selectors.push(format!("({selector_bit} and {case_bit})"));
            }
            let condition = combine_bool_terms("or", &selectors, Some("false"));
            Ok(bit_term(result_bit, &condition))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    wrap_unsigned_expr(&sum_terms(&bit_terms)?, result_width)
}

fn one_hot_sel_bool_expr(
    selector_expr: &str,
    selector_width: usize,
    case_exprs: &[String],
) -> Result<String, PirLustreError> {
    let mut selectors = Vec::new();
    for case_index in 0..case_exprs.len() {
        if case_index >= selector_width {
            break;
        }
        let selector_bit = bit_test_condition(selector_expr, selector_width, case_index)?;
        let case_bit = bit_test_condition(&case_exprs[case_index], 1, 0)?;
        selectors.push(format!("({selector_bit} and {case_bit})"));
    }
    Ok(combine_bool_terms("or", &selectors, Some("false")))
}

fn encode_expr(
    arg_expr: &str,
    arg_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(result_width).is_machine() {
        let zero = zero_bits_expr(result_width);
        let mut terms = Vec::new();
        for input_bit in 0..arg_width {
            let condition = bit_test_condition(arg_expr, arg_width, input_bit)?;
            let literal = scalar_literal_expr(&input_bit.to_string(), result_width);
            terms.push(format!("(if {condition} then {literal} else {zero})"));
        }
        return wrap_unsigned_expr(
            &machine_bitwise_join_expr(NaryOp::Or, &terms, result_width)?,
            result_width,
        );
    }
    let bit_terms = (0..result_width)
        .map(|result_bit| {
            let mut contributors = Vec::new();
            for input_bit in 0..arg_width {
                if ((input_bit >> result_bit) & 1) == 1 {
                    contributors.push(bit_test_condition(arg_expr, arg_width, input_bit)?);
                }
            }
            let condition = combine_bool_terms("or", &contributors, Some("false"));
            Ok(bit_term(result_bit, &condition))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    wrap_unsigned_expr(&sum_terms(&bit_terms)?, result_width)
}

fn encode_bool_expr(arg_expr: &str, arg_width: usize) -> Result<String, PirLustreError> {
    let contributors = (0..arg_width)
        .map(|input_bit| bit_test_condition(arg_expr, arg_width, input_bit))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(combine_bool_terms("or", &contributors, Some("false")))
}

fn decode_expr(
    arg_expr: &str,
    arg_width: usize,
    result_width: usize,
) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(result_width).is_machine() && result_width > 1 {
        let zero = zero_bits_expr(result_width);
        let one = scalar_literal_expr("1", result_width);
        let limit = scalar_literal_expr(&result_width.to_string(), arg_width);
        let in_range = unsigned_compare_condition(arg_expr, arg_width, &limit, arg_width, "<");
        let shift_amount = cast_bits_expr(arg_expr, arg_width, result_width);
        let shifted = wrap_unsigned_expr(&format!("({one} lsh {shift_amount})"), result_width)?;
        return Ok(format!("(if {in_range} then {shifted} else {zero})"));
    }
    let bit_terms = (0..result_width)
        .map(|result_bit| {
            let literal = scalar_literal_expr(&result_bit.to_string(), arg_width);
            let condition = unsigned_compare_condition(arg_expr, arg_width, &literal, arg_width, "=");
            Ok(bit_term(result_bit, &condition))
        })
        .collect::<Result<Vec<_>, PirLustreError>>()?;
    wrap_unsigned_expr(&sum_terms(&bit_terms)?, result_width)
}

fn xor_conditions_expr(conditions: &[String]) -> Result<String, PirLustreError> {
    if conditions.is_empty() {
        return Ok("false".to_string());
    }
    let mut out = "false".to_string();
    for condition in conditions {
        out = format!("({condition} <> {out})");
    }
    Ok(out)
}

fn bit_test_condition(
    expr: &str,
    expr_width: usize,
    bit_index: usize,
) -> Result<String, PirLustreError> {
    if ScalarBitsType::for_width(expr_width).is_machine() {
        let shifted = if bit_index == 0 {
            expr.to_string()
        } else {
            format!(
                "({expr} rsh {})",
                scalar_literal_expr(&bit_index.to_string(), expr_width)
            )
        };
        let one = scalar_literal_expr("1", expr_width);
        return Ok(format!("((({shifted}) && {one}) = {one})"));
    }
    let bit_expr = bit_slice_expr(expr, expr_width, bit_index, 1)?;
    Ok(unsigned_compare_condition(&bit_expr, 1, "1", 1, "="))
}

fn bit_term(bit_index: usize, condition: &str) -> String {
    let bit_weight = bit_modulus(bit_index).expect("bit modulus should be representable");
    format!("(if {condition} then {bit_weight} else 0)")
}

fn sum_terms(terms: &[String]) -> Result<String, PirLustreError> {
    if terms.is_empty() {
        return Ok("0".to_string());
    }
    Ok(format!("({})", terms.join(" + ")))
}

fn combine_bool_terms(operator: &str, terms: &[String], empty_value: Option<&str>) -> String {
    if terms.is_empty() {
        return empty_value.unwrap_or("false").to_string();
    }
    format!("({})", terms.join(&format!(" {operator} ")))
}

fn bit_modulus(bit_count: usize) -> Result<String, PirLustreError> {
    Ok(ir_bits_to_unsigned_decimal(&pow2_irbits(bit_count)))
}

fn bit_mask(bit_count: usize) -> Result<String, PirLustreError> {
    Ok(ir_bits_to_unsigned_decimal(&IrBits::all_ones(bit_count)))
}

fn power_of_two_difference_decimal(
    lhs_power: usize,
    rhs_power: usize,
) -> Result<String, PirLustreError> {
    if lhs_power < rhs_power {
        return Err(PirLustreError::Unsupported(format!(
            "expected lhs power {lhs_power} to be >= rhs power {rhs_power}"
        )));
    }
    let width = lhs_power + 1;
    let lhs = pow2_irbits_with_width(lhs_power, width);
    let rhs = pow2_irbits_with_width(rhs_power, width);
    Ok(ir_bits_to_unsigned_decimal(&lhs.sub(&rhs)))
}

fn pow2_irbits(bit_count: usize) -> IrBits {
    pow2_irbits_with_width(bit_count, bit_count + 1)
}

fn pow2_irbits_with_width(bit_count: usize, width: usize) -> IrBits {
    let mut bits = vec![false; width.max(bit_count + 1)];
    bits[bit_count] = true;
    IrBits::from_lsb_is_0(&bits)
}

fn decimal_subtract(lhs: &str, rhs: &str) -> Result<String, PirLustreError> {
    let width = lhs.len().max(rhs.len()) * 4 + 1;
    let lhs_bits = irbits_from_decimal_string(lhs, width)?;
    let rhs_bits = irbits_from_decimal_string(rhs, width)?;
    if lhs_bits.ult(&rhs_bits) {
        return Err(PirLustreError::Unsupported(format!(
            "decimal subtraction requires lhs >= rhs, got {lhs} < {rhs}"
        )));
    }
    Ok(ir_bits_to_unsigned_decimal(&lhs_bits.sub(&rhs_bits)))
}

fn irbits_from_decimal_string(text: &str, width: usize) -> Result<IrBits, PirLustreError> {
    let trimmed = text.trim_start_matches('0');
    let normalized = if trimmed.is_empty() { "0" } else { trimmed };
    let typed = format!("bits[{width}]:{normalized}");
    let value = xlsynth::IrValue::parse_typed(&typed)?;
    Ok(value.to_bits()?)
}

fn ir_bits_to_unsigned_decimal(bits: &IrBits) -> String {
    bits.to_string_fmt(IrFormatPreference::UnsignedDecimal, false)
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (index, ch) in name.chars().enumerate() {
        let valid = ch.is_ascii_alphanumeric() || ch == '_';
        if index == 0 {
            if ch.is_ascii_alphabetic() || ch == '_' {
                out.push(ch);
            } else if valid {
                out.push('_');
                out.push(ch);
            } else {
                out.push('_');
            }
        } else if valid {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".to_string() } else { out }
}

fn flattened_concat_segment_widths(
    func: &ir::Fn,
    node_ref: &NodeRef,
) -> Result<Option<Vec<usize>>, PirLustreError> {
    let node = func.get_node(*node_ref);
    match &node.payload {
        NodePayload::Nary(NaryOp::Concat, operands) => {
            let mut widths = Vec::new();
            for operand in operands {
                if let Some(nested) = flattened_concat_segment_widths(func, operand)? {
                    widths.extend(nested);
                } else {
                    widths.push(expect_bits_width(
                        func.get_node_ty(*operand),
                        "concat operand",
                    )?);
                }
            }
            Ok(Some(widths))
        }
        _ => Ok(None),
    }
}

fn normalize_concat_segment_widths(widths: Option<Vec<usize>>) -> Option<Vec<usize>> {
    match widths {
        Some(widths) if widths.len() > 1 => Some(widths),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use xlsynth_pir::ir::NaryOp;

    use crate::{FrontendOptions, compile_dslx_text, parse_ir_text};

    use super::{
        AggregateBitsSpec, AggregateFieldSpec, ObservationParamLayout, StateProjectionParamLayout,
        bitwise_nary_expr, encode_expr, export_pir_fn_to_lustre_assumption,
        export_pir_fn_to_lustre_observation, export_pir_fn_to_lustre_relation,
        export_pir_fn_to_lustre_state_projection, export_pir_fn_to_lustre_step,
        export_pir_fn_to_named_lustre_assumption_with_aggregates,
        export_pir_fn_to_named_lustre_assumption_with_state_aggregate,
        export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates,
        export_pir_fn_to_named_lustre_next_state_projection_with_aggregates,
        export_pir_fn_to_named_lustre_observation_with_aggregates,
        export_pir_fn_to_named_lustre_observation_with_state_aggregate,
        export_pir_fn_to_named_lustre_relation_with_state_aggregates,
        export_pir_fn_to_named_lustre_state_projection_with_aggregates,
        export_pir_fn_to_named_lustre_state_projection_with_state_aggregate,
        export_pir_fn_to_named_lustre_step, bits_to_int_expr, cast_bits_expr,
        dynamic_bit_slice_expr, lustre_bits_type_name,
        machine_bitwise_join_expr, one_hot_sel_expr, priority_select_bool_expr,
        priority_select_scalar_expr, select_bool_expr, wide_dynamic_bit_test_condition,
        wrap_unsigned_expr,
    };

    fn structured_state_step_ir() -> &'static str {
        r#"
package structured_state_step

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  hi: bits[4] = bit_slice(s, start=4, width=4, id=3)
  lo: bits[4] = bit_slice(i, start=0, width=4, id=4)
  next_s: bits[8] = concat(hi, lo, id=5)
  ret tuple.6: (bits[8], bits[8]) = tuple(i, next_s, id=6)
}
"#
    }

    fn structured_interface_step_ir() -> &'static str {
        r#"
package structured_interface_step

top fn step(i: (bits[4], bits[4]) id=1, s: bits[8] id=2) -> ((bits[4], bits[4]), bits[8]) {
  i_lo: bits[4] = tuple_index(i, index=0, id=3)
  i_hi: bits[4] = tuple_index(i, index=1, id=4)
  o: (bits[4], bits[4]) = tuple(i_hi, i_lo, id=5)
  ret tuple.6: ((bits[4], bits[4]), bits[8]) = tuple(o, s, id=6)
}
"#
    }

    fn structured_array_interface_step_ir() -> &'static str {
        r#"
package structured_array_interface_step

top fn step(i: (bits[8][2], bits[1]) id=1, s: (bits[8][2], bits[1]) id=2) -> ((bits[8][2], bits[1]), (bits[8][2], bits[1])) {
  ret tuple.3: ((bits[8][2], bits[1]), (bits[8][2], bits[1])) = tuple(i, s, id=3)
}
"#
    }

    fn array_index_step_ir() -> &'static str {
        r#"
package array_index_step

top fn step(i: bits[8][2] id=1, s: bits[1] id=2) -> (bits[8], bits[1]) {
  o: bits[8] = array_index(i, indices=[s], assumed_in_bounds=true, id=3)
  ret tuple.4: (bits[8], bits[1]) = tuple(o, s, id=4)
}
"#
    }

    fn array_constructor_step_ir() -> &'static str {
        r#"
package array_constructor_step

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8][2], bits[8]) {
  o: bits[8][2] = array(i, s, id=3)
  ret tuple.4: (bits[8][2], bits[8]) = tuple(o, s, id=4)
}
"#
    }

    fn decode_step_ir() -> &'static str {
        r#"
package decode_step

top fn step(i: bits[3] id=1, s: bits[1] id=2) -> (bits[5], bits[1]) {
  o: bits[5] = decode(i, width=5, id=3)
  ret tuple.4: (bits[5], bits[1]) = tuple(o, s, id=4)
}
"#
    }

    fn or_reduce_step_ir() -> &'static str {
        r#"
package or_reduce_step

top fn step(i: bits[4] id=1, s: bits[1] id=2) -> (bits[1], bits[1]) {
  o: bits[1] = or_reduce(i, id=3)
  ret tuple.4: (bits[1], bits[1]) = tuple(o, s, id=4)
}
"#
    }

    fn aggregate_literal_step_ir() -> &'static str {
        r#"
package aggregate_literal_step

top fn step(i: bits[1] id=1, s: bits[1] id=2) -> ((bits[1], bits[8])[2], bits[1]) {
  o: (bits[1], bits[8])[2] = literal(value=[(0, 1), (1, 2)], id=3)
  ret tuple.4: ((bits[1], bits[8])[2], bits[1]) = tuple(o, s, id=4)
}
"#
    }

    #[test]
    fn exports_accumulator_step_to_pure_lustre() {
        let dslx = r#"
pub fn step(i: u8, s: u8) -> (u8, u8) {
    let o = i + s;
    let next_s = s + u8:1;
    (o, next_s)
}
"#;
        let artifacts = compile_dslx_text(
            Path::new("accumulator.x"),
            dslx,
            Some("step"),
            &FrontendOptions::default(),
        )
        .unwrap();

        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();
        assert_eq!(exported.node_name, "step");
        assert_eq!(exported.state_only_node_name, "step_state_only");
        assert!(exported.lustre_text.contains(&format!(
            "node step(i: {}; s: {})",
            lustre_bits_type_name(8),
            lustre_bits_type_name(8)
        )));
        assert!(exported.lustre_text.contains(&format!(
            "node step_state_only(i: {}; s: {})",
            lustre_bits_type_name(8),
            lustre_bits_type_name(8)
        )));
        assert!(exported.lustre_text.contains("uint8"));
        assert!(exported.lustre_text.contains("+ (uint8 1)"));
        assert!(exported.lustre_text.contains("keepalive = true;"));
        assert_eq!(exported.output_segment_widths, None);
    }

    #[test]
    fn exports_shift_ops_to_pure_lustre() {
        let ir_text = r#"
package shift_test

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  o: bits[8] = shll(i, s, id=3)
  next_s: bits[8] = shrl(i, s, id=4)
  ret tuple.5: (bits[8], bits[8]) = tuple(o, next_s, id=5)
}
"#;
        let artifacts = parse_ir_text(Path::new("shift_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported.lustre_text.contains(" lsh "));
        assert!(exported.lustre_text.contains(" rsh "));
    }

    #[test]
    fn exports_step_with_custom_node_name() {
        let dslx = r#"
pub fn step(i: u8, s: u8) -> (u8, u8) {
    (i, s)
}
"#;
        let artifacts = compile_dslx_text(
            Path::new("identity.x"),
            dslx,
            Some("step"),
            &FrontendOptions::default(),
        )
        .unwrap();

        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl_a").unwrap();
        assert_eq!(exported.node_name, "step_impl_a");
        assert_eq!(exported.state_only_node_name, "step_impl_a_state_only");
        assert!(exported.lustre_text.contains(&format!(
            "node step_impl_a(i: {}; s: {})",
            lustre_bits_type_name(8),
            lustre_bits_type_name(8)
        )));
    }

    #[test]
    fn exports_structured_state_record_when_next_state_concat_is_available() {
        let artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        let aggregate = exported.state_aggregate.as_ref().unwrap();
        assert_eq!(aggregate.type_name, "step_impl_state_t");
        assert_eq!(
            aggregate
                .fields
                .iter()
                .map(|field| field.width)
                .collect::<Vec<_>>(),
            vec![4, 4]
        );
        assert_eq!(
            exported.type_defs,
            vec!["type step_impl_state_t = struct { f0: uint8; f1: uint8 };".to_string()]
        );
        assert!(exported
            .full_step_lustre_text
            .contains("node step_impl(i: uint8; s: step_impl_state_t) returns (o: uint8; s_next: step_impl_state_t; keepalive: bool);"));
        assert!(exported
            .state_only_lustre_text
            .contains("node step_impl_state_only(i: uint8; s: step_impl_state_t) returns (s_next: step_impl_state_t);"));
        assert!(exported.state_only_lustre_text.contains("s.f0"));
        assert!(
            exported
                .state_only_lustre_text
                .contains("s_next = step_impl_state_t {")
        );
    }

    #[test]
    fn exports_structured_input_and_output_records_from_tuple_interface() {
        let artifacts = parse_ir_text(
            Path::new("structured_interface_step.ir"),
            structured_interface_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert_eq!(
            exported
                .input_aggregate
                .as_ref()
                .unwrap()
                .fields
                .iter()
                .map(|field| field.width)
                .collect::<Vec<_>>(),
            vec![4, 4]
        );
        assert_eq!(
            exported
                .output_aggregate
                .as_ref()
                .unwrap()
                .fields
                .iter()
                .map(|field| field.width)
                .collect::<Vec<_>>(),
            vec![4, 4]
        );
        assert!(exported
            .full_step_lustre_text
            .contains("node step_impl(i: step_impl_input_t; s: uint8) returns (o: step_impl_output_t; s_next: uint8; keepalive: bool);"));
        assert!(
            exported
                .full_step_lustre_text
                .contains("o = step_impl_output_t {")
        );
        assert!(exported.full_step_lustre_text.contains("i.f0"));
        assert!(exported.full_step_lustre_text.contains("i.f1"));
    }

    #[test]
    fn exports_structured_records_from_nested_array_interface() {
        let artifacts = parse_ir_text(
            Path::new("structured_array_interface_step.ir"),
            structured_array_interface_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.input_aggregate.is_none());
        assert!(exported.state_aggregate.is_none());
        assert!(exported.output_aggregate.is_none());
        assert!(exported.full_step_lustre_text.contains("o = i;"));
        assert!(exported.full_step_lustre_text.contains("s_next = s;"));
    }

    #[test]
    fn exports_array_index_from_array_parameter() {
        let artifacts = parse_ir_text(
            Path::new("array_index_step.ir"),
            array_index_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.input_aggregate.is_none());
        assert!(exported.full_step_lustre_text.contains("i["));
        assert!(exported.full_step_lustre_text.contains("else 1)"));
        assert!(exported.full_step_lustre_text.contains("s_next = "));
    }

    #[test]
    fn exports_array_constructor_to_structured_output_record() {
        let artifacts = parse_ir_text(
            Path::new("array_constructor_step.ir"),
            array_constructor_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.output_aggregate.is_none());
        assert!(exported.full_step_lustre_text.contains("c_3 = [i, s];"));
        assert!(exported.full_step_lustre_text.contains("o = c_3;"));
    }

    #[test]
    fn exports_decode_to_pure_lustre() {
        let artifacts =
            parse_ir_text(Path::new("decode_step.ir"), decode_step_ir(), Some("step")).unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.full_step_lustre_text.contains(" lsh "));
        assert!(exported
            .full_step_lustre_text
            .contains("if (i < (uint8 5)) then"));
    }

    #[test]
    fn exports_or_reduce_to_boolean_condition() {
        let artifacts =
            parse_ir_text(Path::new("or_reduce_step.ir"), or_reduce_step_ir(), Some("step"))
                .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.full_step_lustre_text.contains("o = "));
        assert!(!exported.full_step_lustre_text.contains("or_reduce"));
    }

    #[test]
    fn exports_nested_aggregate_literal_to_structured_output_record() {
        let artifacts = parse_ir_text(
            Path::new("aggregate_literal_step.ir"),
            aggregate_literal_step_ir(),
            Some("step"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_named_lustre_step(artifacts.top_fn().unwrap(), "step_impl").unwrap();

        assert!(exported.output_aggregate.is_none());
        assert!(exported.full_step_lustre_text.contains("o = ["));
        assert!(exported.full_step_lustre_text.contains("false"));
        assert!(exported.full_step_lustre_text.contains("(uint8 1)"));
        assert!(exported.full_step_lustre_text.contains("true"));
        assert!(exported.full_step_lustre_text.contains("(uint8 2)"));
    }

    #[test]
    fn exports_output_only_observation_to_pure_lustre() {
        let ir_text = r#"
package observation_test

top fn obs(o: bits[8] id=1) -> bits[4] {
  hi: bits[2] = bit_slice(o, start=6, width=2, id=2)
  lo: bits[2] = bit_slice(o, start=0, width=2, id=3)
  ret out: bits[4] = concat(hi, lo, id=4)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("observation_test.ir"), ir_text, Some("obs")).unwrap();
        let exported = export_pir_fn_to_lustre_observation(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(exported.node_name, "observation");
        assert_eq!(exported.param_layout, ObservationParamLayout::OutputOnly);
        assert_eq!(exported.output_param_width, Some(8));
        assert_eq!(exported.observation_width, 4);
        assert_eq!(exported.observation_segment_widths, Some(vec![2, 2]));
        assert!(
            exported
                .lustre_text
                .contains("node observation(o: uint8) returns (obs: uint8);")
        );
    }

    #[test]
    fn exports_input_state_output_observation_to_pure_lustre() {
        let ir_text = r#"
package observation_test

top fn obs(i: bits[8] id=1, s: bits[4] id=2, o: bits[8] id=3) -> bits[1] {
  o_hi: bits[1] = bit_slice(o, start=7, width=1, id=4)
  i_hi: bits[1] = bit_slice(i, start=7, width=1, id=5)
  s_hi: bits[1] = bit_slice(s, start=3, width=1, id=6)
  both: bits[1] = and(o_hi, i_hi, id=7)
  ret out: bits[1] = xor(both, s_hi, id=8)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("observation_test_3.ir"), ir_text, Some("obs")).unwrap();
        let exported = export_pir_fn_to_lustre_observation(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(
            exported.param_layout,
            ObservationParamLayout::InputStateOutput
        );
        assert_eq!(exported.input_width, Some(8));
        assert_eq!(exported.state_width, Some(4));
        assert_eq!(exported.output_param_width, Some(8));
        assert_eq!(exported.observation_width, 1);
        assert!(
            exported
                .lustre_text
                .contains("node observation(i: uint8; s: uint8; o: uint8) returns (obs: bool);")
        );
    }

    #[test]
    fn exports_structured_observation_return_to_lustre_record() {
        let ir_text = r#"
package observation_structured_test

top fn obs(o: bits[8] id=1) -> (bits[2], bits[1]) {
  hi: bits[2] = bit_slice(o, start=6, width=2, id=2)
  lo: bits[1] = bit_slice(o, start=0, width=1, id=3)
  ret out: (bits[2], bits[1]) = tuple(hi, lo, id=4)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("observation_structured_test.ir"),
            ir_text,
            Some("obs"),
        )
        .unwrap();
        let exported = export_pir_fn_to_lustre_observation(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(exported.observation_width, 3);
        assert_eq!(
            exported.observation_aggregate,
            Some(AggregateBitsSpec {
                type_name: "observation_obs_t".to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 2,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            })
        );
        assert!(
            exported
                .lustre_text
                .contains("type observation_obs_t = struct { f0: uint8; f1: bool };")
        );
        assert!(
            exported
                .lustre_text
                .contains("node observation(o: uint8) returns (obs: observation_obs_t);")
        );
    }

    #[test]
    fn exports_state_only_projection_to_pure_lustre() {
        let ir_text = r#"
package state_projection_test

top fn abs(s: bits[8] id=1) -> bits[3] {
  hi: bits[2] = bit_slice(s, start=6, width=2, id=2)
  lo: bits[1] = bit_slice(s, start=0, width=1, id=3)
  ret out: bits[3] = concat(hi, lo, id=4)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("state_projection_test.ir"), ir_text, Some("abs")).unwrap();
        let exported =
            export_pir_fn_to_lustre_state_projection(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(exported.node_name, "state_projection");
        assert_eq!(exported.param_layout, StateProjectionParamLayout::StateOnly);
        assert_eq!(exported.input_width, None);
        assert_eq!(exported.state_width, 8);
        assert_eq!(exported.abstraction_width, 3);
        assert_eq!(exported.abstraction_segment_widths, Some(vec![2, 1]));
        assert!(
            exported
                .lustre_text
                .contains("node state_projection(s: uint8) returns (abs: uint8);")
        );
    }

    #[test]
    fn exports_input_state_projection_to_pure_lustre() {
        let ir_text = r#"
package state_projection_test

top fn abs(i: bits[8] id=1, s: bits[4] id=2) -> bits[2] {
  i_hi: bits[1] = bit_slice(i, start=7, width=1, id=3)
  s_hi: bits[1] = bit_slice(s, start=3, width=1, id=4)
  ret out: bits[2] = concat(i_hi, s_hi, id=5)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("state_projection_test_2.ir"),
            ir_text,
            Some("abs"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_lustre_state_projection(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(
            exported.param_layout,
            StateProjectionParamLayout::InputState
        );
        assert_eq!(exported.input_width, Some(8));
        assert_eq!(exported.state_width, 4);
        assert_eq!(exported.abstraction_width, 2);
        assert!(
            exported
                .lustre_text
                .contains("node state_projection(i: uint8; s: uint8) returns (abs: uint8);")
        );
    }

    #[test]
    fn exports_input_state_prev_input_projection_to_pure_lustre() {
        let ir_text = r#"
package state_projection_test

top fn abs(i: bits[8] id=1, s: bits[4] id=2, i_prev: bits[8] id=3) -> bits[1] {
  stable: bits[1] = eq(i, i_prev, id=4)
  s_hi: bits[1] = bit_slice(s, start=3, width=1, id=5)
  ret out: bits[1] = xor(stable, s_hi, id=6)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("state_projection_test_3.ir"),
            ir_text,
            Some("abs"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_lustre_state_projection(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(
            exported.param_layout,
            StateProjectionParamLayout::InputStatePrevInput
        );
        assert_eq!(exported.input_width, Some(8));
        assert_eq!(exported.state_width, 4);
        assert_eq!(exported.abstraction_width, 1);
        assert!(exported.lustre_text.contains(
            "node state_projection(i: uint8; s: uint8; i_prev: uint8) returns (abs: bool);"
        ));
    }

    #[test]
    fn exports_structured_state_projection_return_to_lustre_record() {
        let ir_text = r#"
package state_projection_structured_test

top fn abs(s: bits[8] id=1) -> (bits[2], bits[1]) {
  hi: bits[2] = bit_slice(s, start=6, width=2, id=2)
  lo: bits[1] = bit_slice(s, start=0, width=1, id=3)
  ret out: (bits[2], bits[1]) = tuple(hi, lo, id=4)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("state_projection_structured_test.ir"),
            ir_text,
            Some("abs"),
        )
        .unwrap();
        let exported =
            export_pir_fn_to_lustre_state_projection(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(exported.abstraction_width, 3);
        assert_eq!(
            exported.abstraction_aggregate,
            Some(AggregateBitsSpec {
                type_name: "state_projection_abs_t".to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 2,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            })
        );
        assert!(
            exported
                .lustre_text
                .contains("type state_projection_abs_t = struct { f0: uint8; f1: bool };")
        );
        assert!(
            exported
                .lustre_text
                .contains("node state_projection(s: uint8) returns (abs: state_projection_abs_t);")
        );
    }

    #[test]
    fn priority_select_scalar_expr_prefers_lower_index_bits() {
        let expr = priority_select_scalar_expr("sel", 2, &["17", "34"], 8, Some("51")).unwrap();
        assert_eq!(
            expr,
            "(if (((sel) && (uint8 1)) = (uint8 1)) then 17 else (if ((((sel rsh (uint8 1))) && (uint8 1)) = (uint8 1)) then 34 else 51))"
        );
    }

    #[test]
    fn machine_bitwise_join_expr_uses_native_machine_ops() {
        let expr = machine_bitwise_join_expr(
            NaryOp::And,
            &["lhs".to_string(), "rhs".to_string(), "acc".to_string()],
            12,
        )
        .unwrap();
        assert_eq!(expr, "(lhs && rhs && acc)");
    }

    #[test]
    fn bitwise_nary_expr_uses_native_machine_ops_for_and() {
        let expr =
            bitwise_nary_expr(NaryOp::And, &["lhs".to_string(), "rhs".to_string()], 12).unwrap();
        assert!(expr.contains("&&"));
        assert!(!expr.contains("mod 2"));
        assert!(!expr.contains("then 2048 else 0"));
    }

    #[test]
    fn one_hot_sel_expr_uses_machine_or_of_gated_cases() {
        let expr = one_hot_sel_expr("sel", 2, &["a".to_string(), "b".to_string()], 12).unwrap();
        assert!(expr.contains("||"));
        assert!(expr.contains("then a else"));
        assert!(expr.contains("then b else"));
        assert!(!expr.contains("then 2048 else 0"));
    }

    #[test]
    fn encode_expr_uses_machine_or_of_index_literals() {
        let expr = encode_expr("sel", 4, 8).unwrap();
        assert!(expr.contains("||"));
        assert!(expr.contains("then (uint8 0) else (uint8 0)"));
        assert!(expr.contains("then (uint8 3) else (uint8 0)"));
        assert!(!expr.contains("then 128 else 0"));
    }

    #[test]
    fn select_bool_expr_emits_boolean_condition_tree() {
        let expr = select_bool_expr(
            "sel",
            &["(i = 1)".to_string(), "(s = 1)".to_string()],
            Some("(i = s)"),
        )
        .unwrap();
        assert_eq!(
            expr,
            "(if (sel = 0) then (i = 1) else (if (sel = 1) then (s = 1) else (i = s)))"
        );
    }

    #[test]
    fn exports_bool_sel_with_width1_selector_without_integer_roundtrip() {
        let ir_text = r#"
package bool_sel_test

top fn rel(i: bits[8] id=1, s_a: bits[8] id=2, s_b: bits[16] id=3, i_prev: bits[8] id=4) -> bits[1] {
  zero: bits[8] = literal(value=0, id=5)
  busy: bits[1] = ne(i, zero, id=6)
  s_a_lo: bits[1] = bit_slice(s_a, start=0, width=1, id=7)
  ret out: bits[1] = sel(busy, cases=[busy, s_a_lo], id=8)
}
"#;
        let artifacts = parse_ir_text(Path::new("bool_sel_test.ir"), ir_text, Some("rel")).unwrap();
        let exported = export_pir_fn_to_lustre_relation(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported.lustre_text.contains(
            "ok = (if (i <> (uint8 0)) then (((s_a) && (uint8 1)) = (uint8 1)) else (i <> (uint8 0)));"
        ));
        assert!(!exported.lustre_text.contains("(if ((if"));
    }

    #[test]
    fn exports_sign_ext_of_bool_without_integer_compare_roundtrip() {
        let ir_text = r#"
package bool_sign_ext_test

top fn step(i: bits[1] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  negated: bits[1] = not(i, id=3)
  extended: bits[7] = sign_ext(negated, new_bit_count=7, id=4)
  o: bits[8] = concat(i, extended, id=5)
  ret tuple.6: (bits[8], bits[8]) = tuple(o, s, id=6)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("bool_sign_ext_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(
            exported
                .lustre_text
                .contains("n_4 = (if b_3 then (uint8 127) else (uint8 0));")
        );
        assert!(!exported.lustre_text.contains("< 1) then"));
    }

    #[test]
    fn priority_select_bool_expr_prefers_lower_index_bits() {
        let expr = priority_select_bool_expr(
            "sel",
            2,
            &["(i = 1)".to_string(), "(s = 1)".to_string()],
            Some("(i = s)"),
        )
        .unwrap();
        assert_eq!(
            expr,
            "(if (((sel) && (uint8 1)) = (uint8 1)) then (i = 1) else (if ((((sel rsh (uint8 1))) && (uint8 1)) = (uint8 1)) then (s = 1) else (i = s)))"
        );
    }

    #[test]
    fn exports_priority_sel_to_pure_lustre() {
        let ir_text = r#"
package priority_sel_test

top fn step(i: bits[2] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  case0: bits[8] = literal(value=17, id=3)
  case1: bits[8] = literal(value=34, id=4)
  default_value: bits[8] = literal(value=51, id=5)
  o: bits[8] = priority_sel(i, cases=[case0, case1], default=default_value, id=6)
  ret tuple.7: (bits[8], bits[8]) = tuple(o, s, id=7)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("priority_sel_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported
            .lustre_text
            .contains("n_6 = (if (((i) && (uint8 1)) = (uint8 1)) then"));
        assert!(exported.lustre_text.contains("o = n_6;"));
        assert!(exported.lustre_text.contains("(uint8 17)"));
        assert!(exported.lustre_text.contains("(uint8 34)"));
        assert!(exported.lustre_text.contains("(uint8 51)"));
    }

    #[test]
    fn exports_bool_priority_sel_without_extra_integer_wrapping() {
        let ir_text = r#"
package bool_priority_sel_test

        top fn rel(i: bits[2] id=1, s_a: bits[8] id=2, s_b: bits[16] id=3, i_prev: bits[2] id=4) -> bits[1] {
  case0: bits[1] = literal(value=1, id=5)
  case1: bits[1] = literal(value=0, id=6)
  s_a_lo: bits[2] = bit_slice(s_a, start=0, width=2, id=7)
  default_value: bits[1] = eq(i_prev, s_a_lo, id=8)
  ret out: bits[1] = priority_sel(i, cases=[case0, case1], default=default_value, id=9)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("bool_priority_sel_test.ir"), ir_text, Some("rel")).unwrap();
        let exported = export_pir_fn_to_lustre_relation(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported.lustre_text.contains("ok = (if "));
        assert!(exported.lustre_text.contains("then true else"));
        assert!(exported.lustre_text.contains("then false else"));
        assert!(!exported.lustre_text.contains("= 1);"));
    }

    #[test]
    fn dynamic_width1_slice_uses_guarded_shift_instead_of_case_ladder() {
        let expr = dynamic_bit_slice_expr("state_bits", 12, "idx", 4, 12, 1).unwrap();
        assert!(expr.contains("state_bits rsh"));
        assert!(expr.contains("idx < (uint8 12)"));
        assert!(!expr.contains("idx = 11"));
        assert!(!expr.contains("mod 2"));
    }

    #[test]
    fn bits_to_int_expr_collapses_machine_literals_to_plain_ints() {
        assert_eq!(bits_to_int_expr("(uint8 25)", 8), "25");
        assert_eq!(bits_to_int_expr("(int foo)", 8), "foo");
        assert_eq!(bits_to_int_expr("(uint8 bar)", 8), "(int (uint8 bar))");
    }

    #[test]
    fn cast_bits_expr_drops_duplicate_machine_ctors() {
        assert_eq!(cast_bits_expr("(uint8 x)", 32, 8), "(uint8 x)");
        assert_eq!(cast_bits_expr("(uint16 y)", 32, 16), "(uint16 y)");
    }

    #[test]
    fn wrap_unsigned_expr_drops_duplicate_machine_ctors() {
        assert_eq!(wrap_unsigned_expr("(uint8 x)", 8).unwrap(), "(uint8 x)");
        assert_eq!(wrap_unsigned_expr("(uint16 y)", 16).unwrap(), "(uint16 y)");
    }

    #[test]
    fn wide_dynamic_width1_slice_uses_chunk_guards_instead_of_bit_case_ladder() {
        let chunks = vec![
            ("wide_state.f0".to_string(), 64),
            ("wide_state.f1".to_string(), 64),
            ("wide_state.f2".to_string(), 64),
            ("wide_state.f3".to_string(), 64),
        ];
        let expr = wide_dynamic_bit_test_condition(&chunks, "idx", 8, 256).unwrap();
        assert!(expr.contains("idx >= (uint8 192)"));
        assert!(expr.contains("wide_state.f3 rsh"));
        assert!(!expr.contains("idx = 255"));
        assert!(!expr.contains("idx = 63"));
        assert!(!expr.contains("if true then"));
    }

    #[test]
    fn exports_dynamic_width1_slice_without_index_case_chain() {
        let ir_text = r#"
package dynamic_width1_slice_test

top fn step(i: bits[4] id=1, s: bits[12] id=2) -> (bits[1], bits[12]) {
  b: bits[1] = dynamic_bit_slice(s, i, width=1, id=3)
  ret out: (bits[1], bits[12]) = tuple(b, s, id=4)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("dynamic_width1_slice_test.ir"),
            ir_text,
            Some("step"),
        )
        .unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported.lustre_text.contains(" rsh "));
        assert!(exported.lustre_text.contains("< (uint8 12)"));
        assert!(!exported.lustre_text.contains("= 11) then"));
        assert!(!exported.lustre_text.contains("mod 2"));
    }

    #[test]
    fn exports_wide_dynamic_width1_slice_without_bit_case_chain() {
        let ir_text = r#"
package dynamic_width1_slice_wide_test

top fn step(i: bits[8] id=1, s: bits[256] id=2) -> (bits[1], bits[256]) {
  b: bits[1] = dynamic_bit_slice(s, i, width=1, id=3)
  ret out: (bits[1], bits[256]) = tuple(b, s, id=4)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("dynamic_width1_slice_wide_test.ir"),
            ir_text,
            Some("step"),
        )
        .unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(!exported.lustre_text.contains("= 255) then"));
        assert!(!exported.lustre_text.contains("= 63) then"));
        assert!(!exported.lustre_text.contains("if true then"));
    }

    #[test]
    fn records_top_level_concat_segment_widths() {
        let ir_text = r#"
package concat_shape_test

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  high: bits[3] = bit_slice(i, start=5, width=3, id=3)
  mid: bits[2] = bit_slice(i, start=3, width=2, id=4)
  low_hi: bits[2] = bit_slice(i, start=1, width=2, id=5)
  low_lo: bits[1] = bit_slice(i, start=0, width=1, id=6)
  low: bits[3] = concat(low_hi, low_lo, id=7)
  o: bits[8] = concat(high, mid, low, id=8)
  ret tuple.9: (bits[8], bits[8]) = tuple(o, s, id=9)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("concat_shape_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert_eq!(exported.output_segment_widths, Some(vec![3, 2, 2, 1]));
    }

    #[test]
    fn exports_assumption_with_previous_input_to_pure_lustre() {
        let ir_text = r#"
package assumption_test

top fn env(i: bits[8] id=1, s: bits[8] id=2, i_prev: bits[8] id=3) -> bits[1] {
  ret eq.4: bits[1] = eq(i, i_prev, id=4)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("assumption_test.ir"), ir_text, Some("env")).unwrap();
        let exported = export_pir_fn_to_lustre_assumption(artifacts.top_fn().unwrap()).unwrap();

        assert!(
            exported
                .lustre_text
                .contains("node assumption(i: uint8; s: uint8; i_prev: uint8) returns (ok: bool);")
        );
        assert!(exported.lustre_text.contains("ok = (i = i_prev);"));
        assert_eq!(exported.prev_input_name.as_deref(), Some("i_prev"));
    }

    #[test]
    fn exports_assumption_with_structured_state_param() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();
        let ir_text = r#"
package structured_assumption

top fn env(i: bits[8] id=1, s: bits[8] id=2, i_prev: bits[8] id=3) -> bits[1] {
  s_hi: bits[4] = bit_slice(s, start=4, width=4, id=4)
  prev_hi: bits[4] = bit_slice(i_prev, start=4, width=4, id=5)
  ret out: bits[1] = eq(s_hi, prev_hi, id=6)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("structured_assumption.ir"), ir_text, Some("env")).unwrap();
        let exported = export_pir_fn_to_named_lustre_assumption_with_state_aggregate(
            artifacts.top_fn().unwrap(),
            "assumption",
            step_export.state_aggregate.as_ref(),
        )
        .unwrap();

        assert!(exported.lustre_text.contains(
            "node assumption(i: uint8; s: step_impl_state_t; i_prev: uint8) returns (ok: bool);"
        ));
        assert!(exported.lustre_text.contains("s.f0"));
        assert!(!exported.lustre_text.contains("(s /"));
    }

    #[test]
    fn exports_observation_with_structured_state_param() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();
        let ir_text = r#"
package structured_observation

top fn obs(i: bits[8] id=1, s: bits[8] id=2) -> bits[4] {
  ret out: bits[4] = bit_slice(s, start=4, width=4, id=3)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("structured_observation.ir"), ir_text, Some("obs")).unwrap();
        let exported = export_pir_fn_to_named_lustre_observation_with_state_aggregate(
            artifacts.top_fn().unwrap(),
            "observation",
            step_export.state_aggregate.as_ref(),
        )
        .unwrap();

        assert_eq!(exported.param_layout, ObservationParamLayout::InputState);
        assert!(
            exported
                .lustre_text
                .contains("node observation(i: uint8; s: step_impl_state_t) returns (obs: uint8);")
        );
        assert!(exported.lustre_text.contains("s.f"));
        assert!(!exported.lustre_text.contains("(s /"));
    }

    #[test]
    fn exports_helpers_with_structured_input_and_output_params() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_interface_step.ir"),
            structured_interface_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();

        let assumption_ir = r#"
package structured_input_assumption

top fn env(i: (bits[4], bits[4]) id=1, s: bits[8] id=2, i_prev: (bits[4], bits[4]) id=3) -> bits[1] {
  i_hi: bits[4] = tuple_index(i, index=1, id=4)
  i_prev_hi: bits[4] = tuple_index(i_prev, index=1, id=5)
  ret out: bits[1] = eq(i_hi, i_prev_hi, id=6)
}
"#;
        let assumption_artifacts = parse_ir_text(
            Path::new("structured_input_assumption.ir"),
            assumption_ir,
            Some("env"),
        )
        .unwrap();
        let assumption = export_pir_fn_to_named_lustre_assumption_with_aggregates(
            assumption_artifacts.top_fn().unwrap(),
            "assumption",
            step_export.input_aggregate.as_ref(),
            None,
        )
        .unwrap();
        assert!(assumption
            .lustre_text
            .contains("node assumption(i: step_impl_input_t; s: uint8; i_prev: step_impl_input_t) returns (ok: bool);"));
        assert!(assumption.lustre_text.contains("i.f1"));
        assert!(assumption.lustre_text.contains("i_prev.f1"));

        let observation_ir = r#"
package structured_output_observation

top fn obs(o: (bits[4], bits[4]) id=1) -> bits[4] {
  ret out: bits[4] = tuple_index(o, index=1, id=2)
}
"#;
        let observation_artifacts = parse_ir_text(
            Path::new("structured_output_observation.ir"),
            observation_ir,
            Some("obs"),
        )
        .unwrap();
        let observation = export_pir_fn_to_named_lustre_observation_with_aggregates(
            observation_artifacts.top_fn().unwrap(),
            "observation",
            None,
            None,
            step_export.output_aggregate.as_ref(),
        )
        .unwrap();
        assert!(
            observation
                .lustre_text
                .contains("node observation(o: step_impl_output_t) returns (obs: uint8);")
        );
        assert!(observation.lustre_text.contains(".f1;"));

        let projection_ir = r#"
package structured_input_projection

top fn abs(i: (bits[4], bits[4]) id=1, s: bits[8] id=2) -> bits[4] {
  ret out: bits[4] = tuple_index(i, index=0, id=3)
}
"#;
        let projection_artifacts = parse_ir_text(
            Path::new("structured_input_projection.ir"),
            projection_ir,
            Some("abs"),
        )
        .unwrap();
        let projection = export_pir_fn_to_named_lustre_state_projection_with_aggregates(
            projection_artifacts.top_fn().unwrap(),
            "state_projection",
            step_export.input_aggregate.as_ref(),
            None,
        )
        .unwrap();
        assert!(projection.lustre_text.contains(
            "node state_projection(i: step_impl_input_t; s: uint8) returns (abs: uint8);"
        ));
        assert!(projection.lustre_text.contains(".f0;"));
    }

    #[test]
    fn exports_relation_with_input_and_previous_input_to_pure_lustre() {
        let ir_text = r#"
package relation_test

top fn rel(i: bits[8] id=1, s_a: bits[8] id=2, s_b: bits[16] id=3, i_prev: bits[8] id=4) -> bits[1] {
  zero: bits[8] = literal(value=0, id=5)
  busy: bits[1] = ne(i, zero, id=6)
  ret sel.7: bits[1] = sel(busy, cases=[busy, busy], id=7)
}
"#;
        let artifacts = parse_ir_text(Path::new("relation_test.ir"), ir_text, Some("rel")).unwrap();
        let exported = export_pir_fn_to_lustre_relation(artifacts.top_fn().unwrap()).unwrap();

        assert!(exported.lustre_text.contains(
            "node relation(i: uint8; s_a: uint8; s_b: uint16; i_prev: uint8) returns (ok: bool);"
        ));
        assert_eq!(exported.input_width, Some(8));
        assert_eq!(exported.lhs_state_width, 8);
        assert_eq!(exported.rhs_state_width, 16);
        assert_eq!(exported.prev_input_name.as_deref(), Some("i_prev"));
        assert!(!exported.lustre_text.contains("(if ((if"));
    }

    #[test]
    fn exports_state_projection_and_relation_with_structured_state_params() {
        let lhs_artifacts = parse_ir_text(
            Path::new("structured_state_step_lhs.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let rhs_artifacts = parse_ir_text(
            Path::new("structured_state_step_rhs.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let lhs_step =
            export_pir_fn_to_named_lustre_step(lhs_artifacts.top_fn().unwrap(), "step_impl_a")
                .unwrap();
        let rhs_step =
            export_pir_fn_to_named_lustre_step(rhs_artifacts.top_fn().unwrap(), "step_impl_b")
                .unwrap();

        let projection_ir = r#"
package structured_projection

top fn abs(s: bits[8] id=1) -> bits[4] {
  ret out: bits[4] = bit_slice(s, start=4, width=4, id=2)
}
"#;
        let projection_artifacts = parse_ir_text(
            Path::new("structured_projection.ir"),
            projection_ir,
            Some("abs"),
        )
        .unwrap();
        let projection = export_pir_fn_to_named_lustre_state_projection_with_state_aggregate(
            projection_artifacts.top_fn().unwrap(),
            "state_projection",
            lhs_step.state_aggregate.as_ref(),
        )
        .unwrap();
        assert!(
            projection
                .lustre_text
                .contains("node state_projection(s: step_impl_a_state_t) returns (abs: uint8);")
        );
        assert!(projection.lustre_text.contains("s.f"));
        assert!(!projection.lustre_text.contains("(s /"));

        let relation_ir = r#"
package structured_relation

top fn rel(s_a: bits[8] id=1, s_b: bits[8] id=2) -> bits[1] {
  a_hi: bits[4] = bit_slice(s_a, start=4, width=4, id=3)
  b_hi: bits[4] = bit_slice(s_b, start=4, width=4, id=4)
  ret out: bits[1] = eq(a_hi, b_hi, id=5)
}
"#;
        let relation_artifacts = parse_ir_text(
            Path::new("structured_relation.ir"),
            relation_ir,
            Some("rel"),
        )
        .unwrap();
        let relation = export_pir_fn_to_named_lustre_relation_with_state_aggregates(
            relation_artifacts.top_fn().unwrap(),
            "relation",
            lhs_step.state_aggregate.as_ref(),
            rhs_step.state_aggregate.as_ref(),
        )
        .unwrap();

        assert!(relation.lustre_text.contains(
            "node relation(s_a: step_impl_a_state_t; s_b: step_impl_b_state_t) returns (ok: bool);"
        ));
        assert!(relation.lustre_text.contains("s_a.f0"));
        assert!(relation.lustre_text.contains("s_b.f0"));
    }

    #[test]
    fn exports_composed_next_state_projection_without_full_state_constructor() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();

        let projection_ir = r#"
package next_projection

top fn abs(s: bits[8] id=1) -> bits[4] {
  ret out: bits[4] = bit_slice(s, start=4, width=4, id=2)
}
"#;
        let projection_artifacts =
            parse_ir_text(Path::new("next_projection.ir"), projection_ir, Some("abs")).unwrap();
        let exported = export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
            step_artifacts.top_fn().unwrap(),
            projection_artifacts.top_fn().unwrap(),
            "next_abs",
            step_export.input_aggregate.as_ref(),
            step_export.state_aggregate.as_ref(),
        )
        .unwrap();

        assert!(
            exported
                .lustre_text
                .contains("node next_abs(i: uint8; s: step_impl_state_t) returns (abs: uint8);")
        );
        assert!(exported.lustre_text.contains("step_n_3"));
        assert!(!exported.lustre_text.contains("s_next ="));
        assert!(!exported.lustre_text.contains("step_impl_state_t {"));
    }

    #[test]
    fn exports_structured_next_state_projection_return_to_lustre_record() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();

        let projection_ir = r#"
package next_projection_structured

top fn abs(s: bits[8] id=1) -> (bits[4], bits[1]) {
  hi: bits[4] = bit_slice(s, start=4, width=4, id=2)
  lo: bits[1] = bit_slice(s, start=0, width=1, id=3)
  ret out: (bits[4], bits[1]) = tuple(hi, lo, id=4)
}
"#;
        let projection_artifacts = parse_ir_text(
            Path::new("next_projection_structured.ir"),
            projection_ir,
            Some("abs"),
        )
        .unwrap();
        let exported = export_pir_fn_to_named_lustre_next_state_projection_with_aggregates(
            step_artifacts.top_fn().unwrap(),
            projection_artifacts.top_fn().unwrap(),
            "next_abs",
            step_export.input_aggregate.as_ref(),
            step_export.state_aggregate.as_ref(),
        )
        .unwrap();

        assert_eq!(
            exported.abstraction_aggregate,
            Some(AggregateBitsSpec {
                type_name: "next_abs_abs_t".to_string(),
                fields: vec![
                    AggregateFieldSpec {
                        name: "f0".to_string(),
                        width: 4,
                    },
                    AggregateFieldSpec {
                        name: "f1".to_string(),
                        width: 1,
                    },
                ],
            })
        );
        assert!(
            exported
                .lustre_text
                .contains("type next_abs_abs_t = struct { f0: uint8; f1: bool };")
        );
        assert!(exported.lustre_text.contains(
            "node next_abs(i: uint8; s: step_impl_state_t) returns (abs: next_abs_abs_t);"
        ));
    }

    #[test]
    fn exports_composed_next_state_assumption_without_full_state_constructor() {
        let step_artifacts = parse_ir_text(
            Path::new("structured_state_step.ir"),
            structured_state_step_ir(),
            Some("step"),
        )
        .unwrap();
        let step_export =
            export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl")
                .unwrap();

        let assumption_ir = r#"
package next_assumption

top fn env(i: bits[8] id=1, s: bits[8] id=2, i_prev: bits[8] id=3) -> bits[1] {
  s_hi: bits[4] = bit_slice(s, start=4, width=4, id=4)
  i_hi: bits[4] = bit_slice(i, start=4, width=4, id=5)
  ret out: bits[1] = eq(s_hi, i_hi, id=6)
}
"#;
        let assumption_artifacts =
            parse_ir_text(Path::new("next_assumption.ir"), assumption_ir, Some("env")).unwrap();
        let exported = export_pir_fn_to_named_lustre_next_state_assumption_with_aggregates(
            step_artifacts.top_fn().unwrap(),
            assumption_artifacts.top_fn().unwrap(),
            "next_env",
            step_export.input_aggregate.as_ref(),
            step_export.state_aggregate.as_ref(),
        )
        .unwrap();

        assert!(exported.lustre_text.contains(
            "node next_env(i: uint8; s: step_impl_state_t; i_next: uint8) returns (ok: bool);"
        ));
        assert!(exported.lustre_text.contains("step_n_3"));
        assert!(exported.lustre_text.contains("i_next"));
        assert!(!exported.lustre_text.contains("s_next ="));
        assert!(!exported.lustre_text.contains("step_impl_state_t {"));
    }

    #[test]
    fn exports_wide_concat_without_bigint_scalarization() {
        let ir_text = r#"
package wide_concat_test

top fn step(i: bits[64] id=1, s: bits[96] id=2) -> (bits[96], bits[96]) {
  hi: bits[64] = bit_slice(s, start=32, width=64, id=3)
  lo: bits[32] = bit_slice(i, start=0, width=32, id=4)
  o: bits[96] = concat(hi, lo, id=5)
  ret tuple.6: (bits[96], bits[96]) = tuple(o, s, id=6)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("wide_concat_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(!exported.lustre_text.contains(" mod "));
        assert!(!exported.lustre_text.contains(" / "));
        assert!(exported.lustre_text.contains(": uint64"));
    }

    #[test]
    fn exports_wide_bit_slice_without_bigint_scalarization() {
        let ir_text = r#"
package wide_slice_test

top fn step(i: bits[8] id=1, s: bits[96] id=2) -> (bits[96], bits[80]) {
  o: bits[80] = bit_slice(s, start=8, width=80, id=3)
  ret tuple.4: (bits[96], bits[80]) = tuple(s, o, id=4)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("wide_slice_test.ir"), ir_text, Some("step")).unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(!exported.lustre_text.contains(" mod "));
        assert!(!exported.lustre_text.contains(" / "));
        assert!(exported.lustre_text.contains(": uint64"));
    }

    #[test]
    fn exports_wide_bit_slice_update_without_bigint_scalarization() {
        let ir_text = r#"
package wide_slice_update_test

top fn step(i: bits[8] id=1, s: bits[96] id=2) -> (bits[96], bits[96]) {
  start: bits[7] = bit_slice(i, start=0, width=7, id=3)
  update: bits[16] = zero_ext(i, new_bit_count=16, id=4)
  next_s: bits[96] = bit_slice_update(s, start, update, id=5)
  ret tuple.6: (bits[96], bits[96]) = tuple(next_s, next_s, id=6)
}
"#;
        let artifacts = parse_ir_text(
            Path::new("wide_slice_update_test.ir"),
            ir_text,
            Some("step"),
        )
        .unwrap();
        let exported = export_pir_fn_to_lustre_step(artifacts.top_fn().unwrap()).unwrap();

        assert!(!exported.lustre_text.contains(" mod "));
        assert!(!exported.lustre_text.contains(" / "));
        assert!(exported.lustre_text.contains(": uint64"));
    }

    #[test]
    fn exports_wide_unsigned_compare_without_bigint_scalarization() {
        let ir_text = r#"
package wide_compare_test

top fn rel(s_a: bits[96] id=1, s_b: bits[96] id=2) -> bits[1] {
  ret out: bits[1] = ult(s_a, s_b, id=3)
}
"#;
        let artifacts =
            parse_ir_text(Path::new("wide_compare_test.ir"), ir_text, Some("rel")).unwrap();
        let exported = export_pir_fn_to_lustre_relation(artifacts.top_fn().unwrap()).unwrap();

        assert!(!exported.lustre_text.contains(" mod "));
        assert!(!exported.lustre_text.contains(" / "));
        assert!(exported.lustre_text.contains(": uint64"));
    }
}
