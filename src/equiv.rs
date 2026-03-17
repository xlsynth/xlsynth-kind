use std::error::Error;
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::kind2_smt::{Kind2Error, Kind2Options, emit_kind2_system_smt};
use crate::pipeline::{
    FrontendArtifacts, FrontendOptions, PipelineError, compile_dslx_path, compile_dslx_text,
    parse_ir_text,
};
use crate::pir_lustre::{ExportedStepLustre, PirLustreError, export_pir_fn_to_lustre_step};
use crate::xls_smt::emit_ir_function_as_smtlib;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquivOutcome {
    Equivalent,
    Disproved(String),
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquivOptions {
    pub kind2: Kind2Options,
    pub z3_bin: PathBuf,
}

impl EquivOptions {
    /// Checks the configured Kind 2 runner and the host Z3 used for the miter.
    pub fn tools_available(&self) -> bool {
        self.kind2.runner.is_available()
            && Command::new(&self.z3_bin)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
    }
}

impl Default for EquivOptions {
    fn default() -> Self {
        Self {
            kind2: Kind2Options::default(),
            z3_bin: PathBuf::from(
                std::env::var("XLSYNTH_Z3_BIN").unwrap_or_else(|_| "z3".to_string()),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquivBundle {
    pub lustre_text: String,
    pub xls_smtlib: String,
    pub kind2_system_smt: String,
    pub miter_smt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquivCheck {
    pub outcome: EquivOutcome,
    pub bundle: EquivBundle,
}

#[derive(Debug)]
pub enum EquivError {
    Pipeline(PipelineError),
    Export(PirLustreError),
    Kind2(Kind2Error),
    Xlsynth(xlsynth::XlsynthError),
    Io(std::io::Error),
    ToolFailure(String),
}

impl Display for EquivError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(err) => write!(f, "{err}"),
            Self::Export(err) => write!(f, "{err}"),
            Self::Kind2(err) => write!(f, "{err}"),
            Self::Xlsynth(err) => write!(f, "{err}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::ToolFailure(err) => write!(f, "{err}"),
        }
    }
}

impl Error for EquivError {}

impl From<PipelineError> for EquivError {
    fn from(value: PipelineError) -> Self {
        Self::Pipeline(value)
    }
}

impl From<PirLustreError> for EquivError {
    fn from(value: PirLustreError) -> Self {
        Self::Export(value)
    }
}

impl From<Kind2Error> for EquivError {
    fn from(value: Kind2Error) -> Self {
        Self::Kind2(value)
    }
}

impl From<xlsynth::XlsynthError> for EquivError {
    fn from(value: xlsynth::XlsynthError) -> Self {
        Self::Xlsynth(value)
    }
}

impl From<std::io::Error> for EquivError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn prove_dslx_path_step_equivalence(
    path: &Path,
    entry_name: Option<&str>,
    frontend_options: &FrontendOptions,
    equiv_options: &EquivOptions,
) -> Result<EquivCheck, EquivError> {
    let artifacts = compile_dslx_path(path, entry_name, frontend_options)?;
    prove_frontend_step_equivalence(&artifacts, equiv_options)
}

pub fn prove_dslx_text_step_equivalence(
    path: &Path,
    dslx: &str,
    entry_name: Option<&str>,
    frontend_options: &FrontendOptions,
    equiv_options: &EquivOptions,
) -> Result<EquivCheck, EquivError> {
    let artifacts = compile_dslx_text(path, dslx, entry_name, frontend_options)?;
    prove_frontend_step_equivalence(&artifacts, equiv_options)
}

pub fn prove_ir_text_step_equivalence(
    path: &Path,
    ir_text: &str,
    top_name: Option<&str>,
    equiv_options: &EquivOptions,
) -> Result<EquivCheck, EquivError> {
    let artifacts = parse_ir_text(path, ir_text, top_name)?;
    prove_frontend_step_equivalence(&artifacts, equiv_options)
}

pub fn prove_frontend_step_equivalence(
    artifacts: &FrontendArtifacts,
    equiv_options: &EquivOptions,
) -> Result<EquivCheck, EquivError> {
    let top_fn = artifacts.top_fn()?;
    let exported = export_pir_fn_to_lustre_step(top_fn)?;
    let xls_smtlib = emit_ir_function_as_smtlib(&artifacts.ir_text, top_fn.name.as_str())?;
    let kind2_system_smt = emit_kind2_system_smt(
        &exported.lustre_text,
        exported.node_name.as_str(),
        &equiv_options.kind2,
    )?;
    let miter_smt = build_step_equivalence_miter(
        &xls_smtlib,
        &kind2_system_smt,
        top_fn.name.as_str(),
        &exported,
    )?;
    let outcome = run_z3_miter(&miter_smt, &equiv_options.z3_bin)?;

    Ok(EquivCheck {
        outcome,
        bundle: EquivBundle {
            lustre_text: exported.lustre_text,
            xls_smtlib,
            kind2_system_smt,
            miter_smt,
        },
    })
}

pub fn build_step_equivalence_miter(
    xls_smtlib: &str,
    kind2_system_smt: &str,
    xls_function_name: &str,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    let init_relation = parse_kind2_init_relation(kind2_system_smt, exported.node_name.as_str())?;
    let kind2_step_bv = extract_kind2_step_bv_semantics(&init_relation, exported)?;
    let tuple_type = format!(
        "(bits[{}], bits[{}])",
        exported.output_width, exported.state_width
    );
    let tuple_sort = smt_quoted(&tuple_type);
    let output_projector = smt_quoted(&format!("{tuple_type}_0"));
    let next_state_projector = smt_quoted(&format!("{tuple_type}_1"));

    Ok(format!(
        "{xls_smtlib}\n\
         (declare-const i_bv (_ BitVec {input_width}))\n\
         (declare-const s_bv (_ BitVec {state_width}))\n\
         (define-fun xls_result () {tuple_sort} (select {xls_function_name} i_bv s_bv))\n\
         (define-fun xls_o () (_ BitVec {output_width}) ({output_projector} xls_result))\n\
         (define-fun xls_s_next () (_ BitVec {state_width}) ({next_state_projector} xls_result))\n\
         (define-fun lustre_o () (_ BitVec {output_width}) {lustre_output_expr})\n\
         (define-fun lustre_s_next () (_ BitVec {state_width}) {lustre_next_state_expr})\n\
         (assert (or (distinct lustre_o xls_o)\n\
                     (distinct lustre_s_next xls_s_next)))\n\
         (check-sat)\n",
        input_width = exported.input_width,
        state_width = exported.state_width,
        output_width = exported.output_width,
        lustre_output_expr = kind2_step_bv.output_expr,
        lustre_next_state_expr = kind2_step_bv.next_state_expr,
    ))
}

fn run_z3_miter(miter_smt: &str, z3_bin: &Path) -> Result<EquivOutcome, EquivError> {
    let mut child = Command::new(z3_bin)
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;

        let stdin = child.stdin.as_mut().ok_or_else(|| {
            EquivError::ToolFailure("failed to open stdin for z3 process".to_string())
        })?;
        stdin.write_all(miter_smt.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(EquivError::ToolFailure(format!(
            "z3 exited with status {}.\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or_default().trim();
    Ok(match first_line {
        "unsat" => EquivOutcome::Equivalent,
        "sat" => EquivOutcome::Disproved(stdout.into_owned()),
        "unknown" => EquivOutcome::Unknown(stdout.into_owned()),
        other => EquivOutcome::Unknown(format!(
            "unexpected z3 output `{other}`\nfull stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )),
    })
}

fn smt_quoted(name: &str) -> String {
    format!("|{name}|")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Kind2InitRelation {
    name: String,
    args: Vec<Kind2InitArg>,
    body: SExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Kind2InitArg {
    name: String,
    sort: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Kind2StepBvSemantics {
    output_expr: String,
    next_state_expr: String,
}

fn parse_kind2_init_relation(
    kind2_system_smt: &str,
    node_name: &str,
) -> Result<Kind2InitRelation, EquivError> {
    let fallback_needle = "(define-fun __node_init_";
    let preferred_prefix = format!("__node_init_{node_name}_");
    let start = kind2_system_smt
        .match_indices(fallback_needle)
        .find_map(|(start, _)| {
            let after_define_fun = start + "(define-fun ".len();
            let remaining = &kind2_system_smt[after_define_fun..];
            let name_end = remaining.find(char::is_whitespace)?;
            let relation_name = &remaining[..name_end];
            relation_name
                .strip_prefix(&preferred_prefix)
                .filter(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
                .map(|_| start)
        })
        .or_else(|| kind2_system_smt.find(fallback_needle))
        .ok_or_else(|| {
            EquivError::ToolFailure(
                "Kind 2 SMT artifact did not contain an __node_init_ relation".to_string(),
            )
        })?;
    let after_define_fun = start + "(define-fun ".len();
    let remaining = &kind2_system_smt[after_define_fun..];
    let name_end = remaining.find(char::is_whitespace).ok_or_else(|| {
        EquivError::ToolFailure("failed to parse Kind 2 init relation name".to_string())
    })?;
    let name = remaining[..name_end].to_string();
    let args_open = skip_smt_whitespace(kind2_system_smt, after_define_fun + name_end);
    let (args_block, after_args) = extract_smt_paren_block(kind2_system_smt, args_open)?;
    let args = parse_kind2_init_args(args_block)?;
    let return_sort_start = skip_smt_whitespace(kind2_system_smt, after_args);
    let after_return_sort = skip_smt_atom(kind2_system_smt, return_sort_start);
    let body_start = skip_smt_whitespace(kind2_system_smt, after_return_sort);
    let (body_block, _) = extract_smt_paren_block(kind2_system_smt, body_start)?;
    let body = parse_smt_sexpr(body_block)?;
    Ok(Kind2InitRelation { name, args, body })
}

fn extract_kind2_step_bv_semantics(
    relation: &Kind2InitRelation,
    exported: &ExportedStepLustre,
) -> Result<Kind2StepBvSemantics, EquivError> {
    let output_rhs = find_kind2_equation_rhs(&relation.body, "step.usr.o@0").ok_or_else(|| {
        EquivError::ToolFailure(
            "Kind 2 init relation did not contain an equation for `step.usr.o@0`".to_string(),
        )
    })?;
    let next_state_rhs =
        find_kind2_equation_rhs(&relation.body, "step.usr.s_next@0").ok_or_else(|| {
            EquivError::ToolFailure(
                "Kind 2 init relation did not contain an equation for `step.usr.s_next@0`"
                    .to_string(),
            )
        })?;

    Ok(Kind2StepBvSemantics {
        output_expr: lower_kind2_int_expr_to_bv(&output_rhs, exported.output_width, exported)?,
        next_state_expr: lower_kind2_int_expr_to_bv(
            &next_state_rhs,
            exported.state_width,
            exported,
        )?,
    })
}

fn parse_kind2_init_args(args_block: &str) -> Result<Vec<Kind2InitArg>, EquivError> {
    let inner = strip_smt_outer_parens(args_block)?;
    let mut args = Vec::new();
    let mut index = 0;

    while index < inner.len() {
        index = skip_smt_whitespace(inner, index);
        if index >= inner.len() {
            break;
        }
        let (arg_block, next_index) = extract_smt_paren_block(inner, index)?;
        let arg_text = strip_smt_outer_parens(arg_block)?.trim();
        let split_index = arg_text.find(char::is_whitespace).ok_or_else(|| {
            EquivError::ToolFailure(format!(
                "failed to parse Kind 2 init relation argument from `{arg_text}`"
            ))
        })?;
        let name = arg_text[..split_index].trim().to_string();
        let sort = arg_text[split_index..].trim().to_string();
        if name.is_empty() || sort.is_empty() {
            return Err(EquivError::ToolFailure(format!(
                "failed to parse Kind 2 init relation argument from `{arg_text}`"
            )));
        }
        args.push(Kind2InitArg { name, sort });
        index = next_index;
    }

    Ok(args)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SExpr {
    Atom(String),
    List(Vec<SExpr>),
}

fn parse_smt_sexpr(text: &str) -> Result<SExpr, EquivError> {
    let (sexpr, end) = parse_smt_sexpr_at(text, 0)?;
    let end = skip_smt_whitespace(text, end);
    if end != text.len() {
        return Err(EquivError::ToolFailure(format!(
            "unexpected trailing SMT tokens after `{}`",
            &text[end..]
        )));
    }
    Ok(sexpr)
}

fn parse_smt_sexpr_at(text: &str, start: usize) -> Result<(SExpr, usize), EquivError> {
    let start = skip_smt_whitespace(text, start);
    let bytes = text.as_bytes();
    match bytes.get(start) {
        Some(b'(') => {
            let mut items = Vec::new();
            let mut index = start + 1;
            loop {
                index = skip_smt_whitespace(text, index);
                match bytes.get(index) {
                    Some(b')') => return Ok((SExpr::List(items), index + 1)),
                    Some(_) => {
                        let (item, next_index) = parse_smt_sexpr_at(text, index)?;
                        items.push(item);
                        index = next_index;
                    }
                    None => {
                        return Err(EquivError::ToolFailure(
                            "unterminated SMT list while parsing Kind 2 expression".to_string(),
                        ));
                    }
                }
            }
        }
        Some(_) => {
            let end = skip_smt_atom(text, start);
            Ok((SExpr::Atom(text[start..end].to_string()), end))
        }
        None => Err(EquivError::ToolFailure(
            "unexpected end of SMT text while parsing expression".to_string(),
        )),
    }
}

fn find_kind2_equation_rhs(body: &SExpr, lhs_name: &str) -> Option<SExpr> {
    find_kind2_equation_rhs_with_env(body, lhs_name, &BTreeMap::new())
}

fn find_kind2_equation_rhs_with_env(
    term: &SExpr,
    lhs_name: &str,
    env: &BTreeMap<String, SExpr>,
) -> Option<SExpr> {
    let SExpr::List(items) = term else {
        return None;
    };
    match items.as_slice() {
        [SExpr::Atom(head), SExpr::Atom(lhs), rhs] if head == "=" && lhs == lhs_name => {
            Some(resolve_kind2_let_refs(rhs, env))
        }
        [SExpr::Atom(head), bindings, body] if head == "let" => {
            let mut extended_env = env.clone();
            let SExpr::List(binding_items) = bindings else {
                return None;
            };
            for binding in binding_items {
                let SExpr::List(parts) = binding else {
                    return None;
                };
                let [SExpr::Atom(name), value] = parts.as_slice() else {
                    return None;
                };
                extended_env.insert(name.clone(), resolve_kind2_let_refs(value, &extended_env));
            }
            find_kind2_equation_rhs_with_env(body, lhs_name, &extended_env)
        }
        [SExpr::Atom(head), rest @ ..] if head == "and" => rest
            .iter()
            .find_map(|item| find_kind2_equation_rhs_with_env(item, lhs_name, env)),
        _ => None,
    }
}

fn resolve_kind2_let_refs(expr: &SExpr, env: &BTreeMap<String, SExpr>) -> SExpr {
    match expr {
        SExpr::Atom(atom) => env
            .get(atom)
            .cloned()
            .unwrap_or_else(|| SExpr::Atom(atom.clone())),
        SExpr::List(items) => SExpr::List(
            items.iter()
                .map(|item| resolve_kind2_let_refs(item, env))
                .collect(),
        ),
    }
}

fn lower_kind2_int_expr_to_bv(
    expr: &SExpr,
    target_width: usize,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    match expr {
        SExpr::Atom(atom) => lower_kind2_atom_to_bv(atom, target_width, exported),
        SExpr::List(items) => match items.as_slice() {
            [SExpr::Atom(head), lhs, rhs] if head == "bvadd" => Ok(format!(
                "(bvadd {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvsub" => Ok(format!(
                "(bvsub {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvmul" => Ok(format!(
                "(bvmul {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvand" => Ok(format!(
                "(bvand {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvor" => Ok(format!(
                "(bvor {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvshl" => Ok(format!(
                "(bvshl {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvlshr" => Ok(format!(
                "(bvlshr {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvashr" => Ok(format!(
                "(bvashr {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), arg] if head == "bv2nat" || head == "ubv_to_int" => {
                lower_kind2_int_expr_to_bv(arg, target_width, exported)
            }
            [SExpr::Atom(head), arg] if head == "bvnot" => Ok(format!(
                "(bvnot {})",
                lower_kind2_int_expr_to_bv(arg, target_width, exported)?
            )),
            [SExpr::List(indexed), arg] => {
                lower_kind2_indexed_bv_expr(indexed, arg, target_width, exported)
            }
            [
                SExpr::Atom(head),
                SExpr::Atom(bits_atom),
                SExpr::Atom(width_atom),
            ] if head == "_" && bits_atom.starts_with("bv") => {
                let value: u128 = bits_atom.trim_start_matches("bv").parse().map_err(|_| {
                    EquivError::ToolFailure(format!(
                        "failed to parse bitvector constant `{bits_atom}`"
                    ))
                })?;
                let source_width: usize = width_atom.parse().map_err(|_| {
                    EquivError::ToolFailure(format!(
                        "failed to parse bitvector width `{width_atom}`"
                    ))
                })?;
                adapt_bv_width_expr(
                    &format!("(_ bv{value} {source_width})"),
                    source_width,
                    target_width,
                )
            }
            [SExpr::Atom(head), condition, then_value, else_value] if head == "ite" => Ok(format!(
                "(ite {} {} {})",
                lower_kind2_bool_expr(condition, target_width, exported)?,
                lower_kind2_int_expr_to_bv(then_value, target_width, exported)?,
                lower_kind2_int_expr_to_bv(else_value, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "+" => Ok(format!(
                "(bvadd {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "-" => Ok(format!(
                "(bvsub {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "*" => Ok(format!(
                "(bvmul {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, SExpr::Atom(divisor_text)] if head == "div" => {
                let divisor: u128 = divisor_text.parse().map_err(|_| {
                    EquivError::ToolFailure(format!(
                        "unsupported divisor `{divisor_text}` in Kind 2 SMT"
                    ))
                })?;
                let shift = pow2_exponent(divisor).ok_or_else(|| {
                    EquivError::ToolFailure(format!(
                        "only power-of-two divisors are supported in Kind 2 SMT lowering, got {divisor}"
                    ))
                })?;
                Ok(format!(
                    "(bvlshr {} (_ bv{} {}))",
                    lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                    shift,
                    target_width
                ))
            }
            [SExpr::Atom(head), inner, SExpr::Atom(modulus_text)] if head == "mod" => {
                let modulus: u128 = modulus_text.parse().map_err(|_| {
                    EquivError::ToolFailure(format!(
                        "unsupported modulus `{modulus_text}` in Kind 2 SMT"
                    ))
                })?;
                lower_kind2_mod_to_bv(inner, modulus, target_width, exported)
            }
            _ => Err(EquivError::ToolFailure(format!(
                "unsupported Kind 2 SMT expression while lowering to bitvectors: {:?}",
                expr
            ))),
        },
    }
}

fn lower_kind2_bool_expr(
    expr: &SExpr,
    target_width: usize,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    match expr {
        SExpr::Atom(atom) if atom == "true" || atom == "false" => Ok(atom.clone()),
        SExpr::List(items) => match items.as_slice() {
            [SExpr::Atom(head), lhs, rhs] if head == "=" => Ok(format!(
                "(= {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvult" => Ok(format!(
                "(bvult {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvule" => Ok(format!(
                "(bvule {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvugt" => Ok(format!(
                "(bvugt {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "bvuge" => Ok(format!(
                "(bvuge {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "<" => Ok(format!(
                "(bvult {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "<=" => Ok(format!(
                "(bvule {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == ">" => Ok(format!(
                "(bvugt {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == ">=" => Ok(format!(
                "(bvuge {} {})",
                lower_kind2_int_expr_to_bv(lhs, target_width, exported)?,
                lower_kind2_int_expr_to_bv(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), arg] if head == "not" => Ok(format!(
                "(not {})",
                lower_kind2_bool_expr(arg, target_width, exported)?
            )),
            [SExpr::Atom(head), arg] if head == "is_int" => {
                let _ = lower_kind2_int_expr_to_bv(arg, target_width, exported)?;
                Ok("true".to_string())
            }
            [SExpr::Atom(head), lhs, rhs] if head == "and" => Ok(format!(
                "(and {} {})",
                lower_kind2_bool_expr(lhs, target_width, exported)?,
                lower_kind2_bool_expr(rhs, target_width, exported)?
            )),
            [SExpr::Atom(head), lhs, rhs] if head == "or" => Ok(format!(
                "(or {} {})",
                lower_kind2_bool_expr(lhs, target_width, exported)?,
                lower_kind2_bool_expr(rhs, target_width, exported)?
            )),
            _ => Err(EquivError::ToolFailure(format!(
                "unsupported Kind 2 SMT boolean expression: {:?}",
                expr
            ))),
        },
        SExpr::Atom(atom) => Err(EquivError::ToolFailure(format!(
            "unsupported Kind 2 SMT boolean atom `{atom}`"
        ))),
    }
}

fn lower_kind2_atom_to_bv(
    atom: &str,
    target_width: usize,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    let input_user_name = format!("step.usr.{}@0", exported.input_name);
    let state_user_name = format!("step.usr.{}@0", exported.state_name);
    if atom == input_user_name {
        return adapt_named_bv_width("i_bv", exported.input_width, target_width);
    }
    if atom == state_user_name {
        return adapt_named_bv_width("s_bv", exported.state_width, target_width);
    }
    if let Some(bits) = atom.strip_prefix("#b") {
        let source_width = bits.len();
        let value = u128::from_str_radix(bits, 2).map_err(|_| {
            EquivError::ToolFailure(format!(
                "unsupported binary bitvector atom `{atom}` while lowering to bitvectors"
            ))
        })?;
        return adapt_bv_width_expr(
            &format!("(_ bv{value} {source_width})"),
            source_width,
            target_width,
        );
    }
    if let Some(hex) = atom.strip_prefix("#x") {
        let source_width = hex.len() * 4;
        let value = u128::from_str_radix(hex, 16).map_err(|_| {
            EquivError::ToolFailure(format!(
                "unsupported hexadecimal bitvector atom `{atom}` while lowering to bitvectors"
            ))
        })?;
        return adapt_bv_width_expr(
            &format!("(_ bv{value} {source_width})"),
            source_width,
            target_width,
        );
    }
    let value: u128 = atom.parse().map_err(|_| {
        EquivError::ToolFailure(format!(
            "unsupported Kind 2 SMT atom `{atom}` while lowering to bitvectors"
        ))
    })?;
    let modulus = bit_modulus(target_width)?;
    Ok(format!("(_ bv{} {target_width})", value % modulus))
}

fn lower_kind2_indexed_bv_expr(
    indexed: &[SExpr],
    arg: &SExpr,
    target_width: usize,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    match indexed {
        [SExpr::Atom(prefix), SExpr::Atom(op), SExpr::Atom(amount)]
            if prefix == "_" && matches!(op.as_str(), "int2bv" | "int_to_bv") =>
        {
            let amount: usize = amount.parse().map_err(|_| {
                EquivError::ToolFailure(format!("failed to parse {op} width `{amount}`"))
            })?;
            let lowered = lower_kind2_int_expr_to_bv(arg, amount, exported)?;
            adapt_bv_width_expr(&lowered, amount, target_width)
        }
        [SExpr::Atom(prefix), SExpr::Atom(op), SExpr::Atom(amount)]
            if prefix == "_" && op == "zero_extend" =>
        {
            let amount: usize = amount.parse().map_err(|_| {
                EquivError::ToolFailure(format!("failed to parse zero_extend amount `{amount}`"))
            })?;
            let child_width = target_width.checked_sub(amount).ok_or_else(|| {
                EquivError::ToolFailure(format!(
                    "zero_extend amount {amount} exceeded target width {target_width}"
                ))
            })?;
            Ok(format!(
                "((_ zero_extend {amount}) {})",
                lower_kind2_int_expr_to_bv(arg, child_width, exported)?
            ))
        }
        [
            SExpr::Atom(prefix),
            SExpr::Atom(op),
            SExpr::Atom(hi),
            SExpr::Atom(lo),
        ] if prefix == "_" && op == "extract" => {
            let hi: usize = hi.parse().map_err(|_| {
                EquivError::ToolFailure(format!("failed to parse extract high index `{hi}`"))
            })?;
            let lo: usize = lo.parse().map_err(|_| {
                EquivError::ToolFailure(format!("failed to parse extract low index `{lo}`"))
            })?;
            let result_width = hi.checked_sub(lo).map(|delta| delta + 1).ok_or_else(|| {
                EquivError::ToolFailure(format!("invalid extract range {hi}:{lo} in Kind 2 SMT"))
            })?;
            let extracted = format!(
                "((_ extract {hi} {lo}) {})",
                lower_kind2_int_expr_to_bv(arg, hi + 1, exported)?
            );
            adapt_bv_width_expr(&extracted, result_width, target_width)
        }
        _ => Err(EquivError::ToolFailure(format!(
            "unsupported indexed bitvector SMT expression while lowering to bitvectors: {:?}",
            SExpr::List(vec![SExpr::List(indexed.to_vec()), arg.clone()])
        ))),
    }
}

fn lower_kind2_mod_to_bv(
    inner: &SExpr,
    modulus: u128,
    target_width: usize,
    exported: &ExportedStepLustre,
) -> Result<String, EquivError> {
    let exponent = pow2_exponent(modulus).ok_or_else(|| {
        EquivError::ToolFailure(format!(
            "only power-of-two moduli are supported in Kind 2 SMT lowering, got {modulus}"
        ))
    })?;
    if exponent > target_width {
        return Err(EquivError::ToolFailure(format!(
            "cannot lower modulus {modulus} into {target_width}-bit result"
        )));
    }
    let lowered = lower_kind2_int_expr_to_bv(inner, target_width, exported)?;
    if exponent == target_width {
        return Ok(lowered);
    }
    if exponent == 0 {
        return Ok(format!("(_ bv0 {target_width})"));
    }
    Ok(format!(
        "((_ zero_extend {}) ((_ extract {} 0) {}))",
        target_width - exponent,
        exponent - 1,
        lowered
    ))
}

fn adapt_named_bv_width(
    name: &str,
    source_width: usize,
    target_width: usize,
) -> Result<String, EquivError> {
    adapt_bv_width_expr(name, source_width, target_width)
}

fn adapt_bv_width_expr(
    expr: &str,
    source_width: usize,
    target_width: usize,
) -> Result<String, EquivError> {
    if source_width == target_width {
        return Ok(expr.to_string());
    }
    if source_width < target_width {
        return Ok(format!(
            "((_ zero_extend {}) {expr})",
            target_width - source_width
        ));
    }
    if target_width == 0 {
        return Ok("(_ bv0 0)".to_string());
    }
    Ok(format!("((_ extract {} 0) {expr})", target_width - 1))
}

fn pow2_exponent(value: u128) -> Option<usize> {
    if value == 0 || !value.is_power_of_two() {
        None
    } else {
        Some(value.trailing_zeros() as usize)
    }
}

fn bit_modulus(width: usize) -> Result<u128, EquivError> {
    if width > 127 {
        return Err(EquivError::ToolFailure(format!(
            "bit widths above 127 are not yet supported in equivalence checking, got {width}"
        )));
    }
    Ok(1u128 << width)
}

fn extract_smt_paren_block(text: &str, start: usize) -> Result<(&str, usize), EquivError> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return Err(EquivError::ToolFailure(format!(
            "expected `(` at byte {start} while parsing SMT text"
        )));
    }

    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Ok((&text[start..end], end));
                }
            }
            _ => {}
        }
    }

    Err(EquivError::ToolFailure(
        "unterminated parenthesized SMT block".to_string(),
    ))
}

fn strip_smt_outer_parens(text: &str) -> Result<&str, EquivError> {
    if !text.starts_with('(') || !text.ends_with(')') {
        return Err(EquivError::ToolFailure(format!(
            "expected SMT block wrapped in parentheses, got `{text}`"
        )));
    }
    Ok(&text[1..text.len() - 1])
}

fn skip_smt_whitespace(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while let Some(byte) = bytes.get(index) {
        if byte.is_ascii_whitespace() {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn skip_smt_atom(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while let Some(byte) = bytes.get(index) {
        if byte.is_ascii_whitespace() || *byte == b'(' || *byte == b')' {
            break;
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::FrontendOptions;

    use super::{
        EquivOptions, EquivOutcome, build_step_equivalence_miter, parse_kind2_init_relation,
        prove_dslx_path_step_equivalence, prove_ir_text_step_equivalence,
    };
    use crate::ExportedStepLustre;

    #[test]
    fn accumulator_roundtrip_matches_xls_smt() {
        let options = EquivOptions::default();
        if !options.tools_available() {
            eprintln!("Skipping roundtrip proof because Kind 2 or Z3 is unavailable: {options:?}");
            return;
        }

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dslx_path = repo_root.join("dslx").join("accumulator.x");
        let check = prove_dslx_path_step_equivalence(
            &dslx_path,
            Some("step"),
            &FrontendOptions::default(),
            &options,
        )
        .unwrap();

        assert_eq!(check.outcome, EquivOutcome::Equivalent);
        assert!(check.bundle.kind2_system_smt.contains("__node_init_step_0"));
        assert!(check.bundle.miter_smt.contains("(check-sat)"));
    }

    #[test]
    fn shift_roundtrip_matches_xls_smt() {
        let options = EquivOptions::default();
        if !options.tools_available() {
            eprintln!("Skipping roundtrip proof because Kind 2 or Z3 is unavailable: {options:?}");
            return;
        }

        let ir_text = r#"
package shift_test

top fn step(i: bits[8] id=1, s: bits[8] id=2) -> (bits[8], bits[8]) {
  o: bits[8] = shll(i, s, id=3)
  next_s: bits[8] = shra(i, s, id=4)
  ret tuple.5: (bits[8], bits[8]) = tuple(o, next_s, id=5)
}
"#;
        let check = prove_ir_text_step_equivalence(
            Path::new("shift_test.ir"),
            ir_text,
            Some("step"),
            &options,
        )
        .unwrap();

        assert_eq!(check.outcome, EquivOutcome::Equivalent);
    }

    #[test]
    fn parses_kind2_init_relation_signature() {
        let kind2_smt = r#"
(define-fun __node_init_step_0
 ((step.usr.i@0 Int)
  (step.usr.s@0 Int)
  (step.usr.o@0 Int)
  (step.usr.s_next@0 Int)
  (step.usr.keepalive@0 Bool)
  (step.res.init_flag@0 Bool)
  (step.res.subrange_1@0 Bool)
  (step.res.subrange_2@0 Bool)) Bool
 (and true))
"#;
        let parsed = parse_kind2_init_relation(kind2_smt, "step").unwrap();
        assert_eq!(parsed.name, "__node_init_step_0");
        assert_eq!(parsed.args.len(), 8);
        assert_eq!(parsed.args[0].name, "step.usr.i@0");
        assert_eq!(parsed.args[7].sort, "Bool");
        assert!(matches!(parsed.body, super::SExpr::List(_)));
    }

    #[test]
    fn parses_requested_kind2_init_relation_when_multiple_exist() {
        let kind2_smt = r#"
(define-fun __node_init_step_state_only_0
 ((step_state_only.usr.i@0 Int)
  (step_state_only.usr.s@0 Int)
  (step_state_only.usr.s_next@0 Int)) Bool
 (and (= step_state_only.usr.s_next@0 7)))
(define-fun __node_init_step_0
 ((step.usr.i@0 Int)
  (step.usr.s@0 Int)
  (step.usr.o@0 Int)
  (step.usr.s_next@0 Int)) Bool
 (and (= step.usr.o@0 5) (= step.usr.s_next@0 6)))
"#;
        let parsed = parse_kind2_init_relation(kind2_smt, "step").unwrap();
        assert_eq!(parsed.name, "__node_init_step_0");
        assert_eq!(parsed.args[2].name, "step.usr.o@0");
    }

    #[test]
    fn miter_uses_bv_wrapped_kind2_step_equations() {
        let xls_smt = r#"
(declare-datatypes ((|(bits[8], bits[8])| 0))
 (((mk-tuple (|(bits[8], bits[8])_0| (_ BitVec 8))
             (|(bits[8], bits[8])_1| (_ BitVec 8))))))
(declare-fun step ((_ BitVec 8) (_ BitVec 8)) |(bits[8], bits[8])|)
"#;
        let kind2_smt = r#"
(define-fun __node_init_step_0
 ((step.usr.i@0 Int)
  (step.usr.s@0 Int)
  (step.usr.o@0 Int)
  (step.usr.s_next@0 Int)
  (step.usr.keepalive@0 Bool)
  (step.res.init_flag@0 Bool)
  (step.res.subrange_1@0 Bool)
  (step.res.subrange_2@0 Bool)) Bool
 (and
  (= step.usr.o@0 (mod (+ step.usr.i@0 step.usr.s@0) 256))
  (= step.usr.s_next@0 (mod (+ step.usr.s@0 1) 256))
  (= step.usr.keepalive@0 true)))
"#;
        let exported = ExportedStepLustre {
            lustre_text: String::new(),
            type_defs: Vec::new(),
            state_only_lustre_text: String::new(),
            full_step_lustre_text: String::new(),
            node_name: "step".to_string(),
            state_only_node_name: "step_state_only".to_string(),
            input_name: "i".to_string(),
            state_name: "s".to_string(),
            input_ty: xlsynth_pir::ir::Type::Bits(8),
            input_width: 8,
            input_aggregate: None,
            state_ty: xlsynth_pir::ir::Type::Bits(8),
            state_width: 8,
            state_aggregate: None,
            output_ty: xlsynth_pir::ir::Type::Bits(8),
            output_width: 8,
            output_aggregate: None,
            output_segment_widths: None,
        };

        let miter = build_step_equivalence_miter(xls_smt, kind2_smt, "step", &exported).unwrap();
        assert!(miter.contains("(define-fun lustre_o () (_ BitVec 8) (bvadd i_bv s_bv))"));
        assert!(
            miter.contains("(define-fun lustre_s_next () (_ BitVec 8) (bvadd s_bv (_ bv1 8)))")
        );
    }
}
